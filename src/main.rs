// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-2.0

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
mod metadata;
mod pkgutil;

use metadata::InstalledPkg;
use pkgutil::PkgUtil;

#[derive(Parser)]
#[command(name = "pkgm", version, about = "simple package manager")]
struct Cli {
    #[arg(short, long, global = true)]
    root: Option<PathBuf>,
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
    Remove,
    List,
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let root = cli.root.unwrap_or_default();

    match cli.command {
        Command::Install { packages, upgrade, force } => {
            let mut util = PkgUtil::new(root);
            util.set_dry_run(cli.dry_run);
            let dry = util.dry_run;

            // Open DB once for all packages
            util.db_open(dry)?;

            for package in packages {
                // Determine if local file or remote package
                let (name, version, url, checksum) = if package.exists() {
                    let name = package.file_name().unwrap().to_str().unwrap().to_string();
                    let version = "0.0.1".to_string();
                    (name, version, package.to_string_lossy().to_string(), None)
                } else {
                    let name = package.to_string_lossy().to_string();
                    let (version, url, checksum) = util.resolve_package(&name)?;
                    (name, version, url, checksum)
                };

                if dry {
                    println!("[DRY RUN] Would install {} {}", name, version);
                    continue;
                }

                // Download if remote URL
                let pkg_path = if url.starts_with("http") {
                    util.download_package(&url, &name, &version)?
                } else {
                    PathBuf::from(&url)
                };

                let (meta, files) = util.pkg_open(&pkg_path)?;
                let name = meta.name;
                let already = util.db_find_package(&name);

                if already && !upgrade {
                    anyhow::bail!("{} already installed (use --upgrade)", name);
                }

                // Check conflicts with installed packages
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

                // Install and add to database
                util.pkg_install(&pkg_path)?;
                util.db_add_package(InstalledPkg {
                    name: name.clone(),
                    version: version.clone(),
                    description: meta.description,
                    files,
                    checksum,
                });

                // Cleanup temp file if downloaded
                if pkg_path.starts_with(std::env::temp_dir()) {
                    let _ = fs::remove_file(&pkg_path);
                }

                println!("Installed {} {}", name, version);
            }

            // Commit changes and update dynamic linker
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
                RepoAction::Remove => {
                    util.repo_remove()?;
                    println!("Repository removed");
                }
                RepoAction::List => {
                    if let Some(url) = util.repo_list()? {
                        if cli.json {
                            println!("{{\"repo\": \"{}\"}}", url);
                        } else {
                            println!("{}", url);
                        }
                    } else {
                        if cli.json {
                            println!("null");
                        } else {
                            println!("No repository configured");
                        }
                    }
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
