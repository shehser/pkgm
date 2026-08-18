// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-2.0

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod metadata;
mod pkgutil;

use metadata::InstalledPkg;
use pkgutil::PkgUtil;

#[derive(Parser)]
#[command(name = "pkgm", version, about = "simple package manager")]
struct Cli {
    #[arg(short, long, global = true, default_value = "/")]
    root: PathBuf,
    #[arg(short = 'n', long, global = true)]
    dry_run: bool,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Install {
        #[arg(required = true)]
        packages: Vec<PathBuf>,
        #[arg(short, long)]
        upgrade: bool,
        #[arg(short, long)]
        force: bool,
    },
    Remove {
        package: String,
    },
    List,
    Search {
        query: String,
    },
    Checkupdates,
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
    Clean,
}

#[derive(Subcommand)]
enum RepoAction {
    Add { url: String },
    Remove { url: String },
    List,
    Update,
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let root = cli.root;

    match cli.command {
        Command::Install { packages, upgrade, force } => {
            let mut util = PkgUtil::new(root);
            util.set_dry_run(cli.dry_run);
            let dry = util.dry_run;

            util.db_open(dry)?;

            if !dry {
                let _ = util.repo_update();
            }

            for package in packages {
                if dry {
                    println!("[DRY RUN] Would install {}", package.display());
                    continue;
                }

                let name = package.to_string_lossy().to_string();

                if package.exists() {
                    let (meta, files) = util.pkg_open(&package)?;
                    let name = meta.name;
                    let version = "0.0.1".to_string();
                    let already = util.db_find_package(&name);

                    if already && !upgrade {
                        anyhow::bail!("{} already installed (use --upgrade)", name);
                    }

                    let conflicts = util.db_find_conflicts(&name, &files);
                    if !conflicts.is_empty() && !force {
                        eprintln!("Conflicts for {}:", name);
                        for f in &conflicts {
                            eprintln!("  {}", f);
                        }
                        anyhow::bail!("use --force");
                    }

                    if upgrade {
                        util.db_remove_package(&name);
                    }

                    util.pkg_install(&package)?;
                    util.db_add_package(InstalledPkg {
                        name: name.clone(),
                        version,
                        description: meta.description,
                        files,
                        checksum: None,
                    });

                    println!("Installed {}", name);
                } else {
                    util.install_with_deps(&name, upgrade, force)?;
                }
            }

            if !dry {
                util.db_commit()?;
                util.run_ldconfig()?;
            }
            Ok(())
        }

        Command::Remove { package } => {
            let mut util = PkgUtil::new(root);
            util.db_open(false)?;

            if !util.db_find_package(&package) {
                anyhow::bail!("{} not installed", package);
            }

            util.db_remove_package(&package);
            util.db_commit()?;
            util.run_ldconfig()?;
            println!("Removed {}", package);
            Ok(())
        }

        Command::List => {
            let mut util = PkgUtil::new(root);
            util.db_open(false)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&util.packages)?);
            } else {
                for (name, pkg) in &util.packages {
                    println!("{} {}", name, pkg.version);
                }
            }
            Ok(())
        }

        Command::Search { query } => {
            let util = PkgUtil::new(root);
            let _ = util.repo_update();
            let results = util.search(&query)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                for (name, version, desc) in results {
                    println!("{} {} - {}", name, version, desc);
                }
            }
            Ok(())
        }

        Command::Checkupdates => {
            let mut util = PkgUtil::new(root);
            util.db_open(false)?;
            let _ = util.repo_update();
            let updates = util.check_updates()?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&updates)?);
            } else {
                if updates.is_empty() {
                    println!("All packages up to date");
                } else {
                    for (name, current, latest) in updates {
                        println!("{} {} -> {}", name, current, latest);
                    }
                }
            }
            Ok(())
        }

        Command::Repo { action } => {
            let util = PkgUtil::new(root);
            match action {
                RepoAction::Add { url } => {
                    util.repo_add(&url)?;
                    println!("Repository added: {}", url);
                }
                RepoAction::Remove { url } => {
                    util.repo_remove(&url)?;
                    println!("Repository removed: {}", url);
                }
                RepoAction::List => {
                    let repos = util.repo_list()?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&repos)?);
                    } else if repos.is_empty() {
                        println!("No repositories configured");
                    } else {
                        for repo in repos {
                            println!("{}", repo);
                        }
                    }
                }
                RepoAction::Update => {
                    util.repo_update()?;
                }
            }
            Ok(())
        }

        Command::Clean => {
            let util = PkgUtil::new(root);
            util.clean_cache()?;
            println!("Cache cleaned");
            Ok(())
        }
    }
}