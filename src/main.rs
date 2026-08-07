// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod metadata;
mod pkgutil;

use metadata::InstalledPkg;
use pkgutil::PkgUtil;

#[derive(Parser)]
#[command(name = "pkgm", version, about = "Fast and reliable package manager")]
struct Cli {
    #[arg(short, long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Install {
        package: PathBuf,
        #[arg(short, long)]
        upgrade: bool,
        #[arg(short, long)]
        force: bool,
    },
    Unpack {
        package: PathBuf,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Remove {
        package: String,
    },
    Info {
        #[arg(short, long)]
        installed: bool,
        #[arg(short, long)]
        list: Option<String>,
        #[arg(short, long)]
        owner: Option<String>,
        #[arg(short, long)]
        footprint: Option<PathBuf>,
    },
    Apply {
        #[arg(short, long, default_value = "pkgm.toml")]
        config: PathBuf,
        #[arg(short, long)]
        profile: Option<String>,
    },
    Update {
        #[arg(short, long, default_value = "pkgm.toml")]
        config: PathBuf,
        #[arg(short, long)]
        profile: Option<String>,
    },
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
    Search {
        query: String,
    },
    Verify {
        #[arg(default_value = "all")]
        package: String,
    },
}

#[derive(Subcommand)]
enum RepoAction {
    List,
    Add { name: String, url: String },
    Remove { name: String },
    Update,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = cli.root.unwrap_or_default();

    match cli.command {
        Command::Install { package, upgrade, force } => {
            let mut util = PkgUtil::new(root);

            let (pkg_path, checksum_opt) = if package.exists() {
                (package, None)
            } else {
                let name = package.to_string_lossy().to_string();
                let (url, version, checksum_opt) = util.resolve_package(&name)?;
                println!("Found {} version {} in repository", name, version);
                let downloaded = pkgutil::download_package(&url, &name)?;
                (downloaded, checksum_opt)
            };

            util.db_open()?;
            let (meta, files) = util.pkg_open(&pkg_path)?;
            let name = &meta.name;

            if util.db_find_package(name) && !upgrade {
                anyhow::bail!("{} already installed (use --upgrade)", name);
            }
            if !util.db_find_package(name) && upgrade {
                anyhow::bail!("{} not installed", name);
            }

            let conflicts = util.db_find_conflicts(name, &files);
            if !conflicts.is_empty() && !force {
                eprintln!("Conflicting files:");
                for f in &conflicts {
                    eprintln!("  {}", f);
                }
                anyhow::bail!("use --force to overwrite");
            }

            if upgrade {
                util.db_remove_package(name);
            }

            util.pkg_install(&pkg_path)?;
            util.db_add_package(InstalledPkg {
                name: meta.name.clone(),
                version: meta.version,
                description: meta.description,
                files,
                checksum: checksum_opt,
            });
            util.db_commit()?;
            util.run_ldconfig()?;
            println!("Installed {}", meta.name);
        }
        Command::Unpack { package, dir } => {
            let util = PkgUtil::new(root);
            util.pkg_unpack(&package, &dir)?;
            println!("Unpacked {} into {}", package.display(), dir.display());
        }
        Command::Remove { package } => {
            let mut util = PkgUtil::new(root);
            util.db_open()?;
            if !util.db_find_package(&package) {
                anyhow::bail!("{} not installed", package);
            }
            util.db_remove_package(&package);
            util.db_commit()?;
            util.run_ldconfig()?;
            println!("Removed {}", package);
        }
        Command::Info { installed, list, owner, footprint } => {
            if let Some(fp) = footprint {
                let util = PkgUtil::new(root);
                return util.pkg_footprint(&fp);
            }

            let mut util = PkgUtil::new(root);
            util.db_open()?;

            if installed {
                for (name, pkg) in &util.packages {
                    println!("{} {}", name, pkg.version);
                }
            } else if let Some(target) = list {
                if util.db_find_package(&target) {
                    if let Some(pkg) = util.packages.get(&target) {
                        for f in &pkg.files {
                            println!("{}", f);
                        }
                    }
                } else if PathBuf::from(&target).exists() {
                    let (_, files) = util.pkg_open(&target)?;
                    for f in &files {
                        println!("{}", f);
                    }
                } else {
                    anyhow::bail!("{} is neither installed nor a valid archive", target);
                }
            } else if let Some(pattern) = owner {
                let re = regex::Regex::new(&pattern)?;
                let mut results = Vec::new();
                let mut max_len = 0;
                for (name, pkg) in &util.packages {
                    for f in &pkg.files {
                        if re.is_match(&format!("/{}", f)) {
                            results.push((name.clone(), f.clone()));
                            max_len = max_len.max(name.len());
                        }
                    }
                }
                if results.is_empty() {
                    println!("No owners match pattern: {}", pattern);
                } else {
                    for (pkg, f) in results {
                        println!("{:width$}  {}", pkg, f, width = max_len);
                    }
                }
            } else {
                anyhow::bail!("No info flag provided. See --help.");
            }
        }
        Command::Apply { config, profile } => {
            let mut util = PkgUtil::new(root);
            util.pkg_apply(&config, profile.as_deref())?;
        }
        Command::Update { config, profile } => {
            let mut util = PkgUtil::new(root);
            util.pkg_update(&config, profile.as_deref())?;
        }
        Command::Repo { action } => {
            let util = PkgUtil::new(root);
            match action {
                RepoAction::List => util.repo_list()?,
                RepoAction::Add { name, url } => util.repo_add(&name, &url)?,
                RepoAction::Remove { name } => util.repo_remove(&name)?,
                RepoAction::Update => util.repo_update()?,
            }
        }
        Command::Search { query } => {
            let util = PkgUtil::new(root);
            util.search(&query)?;
        }
        Command::Verify { package } => {
            let util = PkgUtil::new(root);
            util.verify(&package)?;
        }
    }

    Ok(())
}