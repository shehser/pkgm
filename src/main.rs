// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

mod metadata;
mod pkgutil;

use metadata::InstalledPkg;
use pkgutil::PkgUtil;

#[derive(Parser)]
#[command(name = "pkgm", version, about = "jst a package manager")]
struct Cli {
    #[arg(short, long, global = true)]
    root: Option<PathBuf>,
    #[arg(short = 'n', long, global = true, help = "Show what would be done without making changes")]
    dry_run: bool,
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
    Clean {
        #[arg(short, long)]
        packages: bool,
        #[arg(short, long)]
        repos: bool,
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
            util.set_dry_run(cli.dry_run);
            let dry = util.dry_run;

            // Open DB in readonly mode for dry-run to avoid write locks.
            util.db_open(dry)?;

            let (pkg_path, checksum_opt, downloaded, version) = if package.exists() {
                let (_name, ver) = metadata::parse_package_name(
                    package.file_name().unwrap_or_default().to_str().unwrap_or("")
                );
                (package.clone(), None, false, ver)
            } else {
                let name = package.to_string_lossy().to_string();
                let (url, ver, checksum_opt) = util.resolve_package(&name)?;
                if dry {
                    println!("[DRY RUN] Found {} version {} in repository", name, ver);
                    println!("[DRY RUN] Would download from {}", url);
                    (PathBuf::from(&format!("/tmp/dry-{}", name)), checksum_opt, true, ver)
                } else {
                    println!("Found {} version {} in repository", name, ver);
                    let downloaded_path = util.download_package(&url, &name, &ver, checksum_opt.as_deref())?;
                    (downloaded_path, checksum_opt, true, ver)
                }
            };

            if let Some(expected) = &checksum_opt {
                if dry {
                    println!("[DRY RUN] Would verify checksum: {}", expected);
                } else if let Err(e) = util.verify_checksum(&pkg_path, expected) {
                    if downloaded { let _ = fs::remove_file(&pkg_path); }
                    return Err(e);
                }
            }

            let (meta, files) = if dry && downloaded {
                let name = package.to_string_lossy().to_string();
                let dummy_meta = metadata::PkgMetadata {
                    name: name.clone(),
                    version: version.clone(),
                    description: Some("(dry-run)".into()),
                };
                (dummy_meta, BTreeSet::new())
            } else {
                util.pkg_open(&pkg_path)?
            };

            let name = &meta.name;
            let already = util.db_find_package(name);

            if already && !upgrade {
                anyhow::bail!("{} already installed (use --upgrade)", name);
            }
            if !already && upgrade {
                anyhow::bail!("{} not installed", name);
            }

            let conflicts = util.db_find_conflicts(name, &files);
            if !conflicts.is_empty() && !force {
                eprintln!("Conflicting files:");
                for f in &conflicts { eprintln!("  {}", f); }
                anyhow::bail!("use --force to overwrite");
            }

            if dry {
                println!("[DRY RUN] Would {} {} {}",
                    if upgrade { "upgrade" } else { "install" },
                    name,
                    meta.version
                );
                if already && upgrade {
                    if let Some(old) = util.packages.get(name) {
                        println!("  Old version: {}", old.version);
                    }
                }
                println!("  Files to be unpacked:");
                for f in &files {
                    println!("    {}", f);
                }
                if !conflicts.is_empty() {
                    println!("  Conflicts detected (would be overwritten because --force)");
                } else {
                    println!("  No conflicts.");
                }
                if downloaded { let _ = fs::remove_file(&pkg_path); }
                return Ok(());
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
                checksum: checksum_opt.clone(),
            });
            util.db_commit()?;
            util.run_ldconfig()?;
            println!("Installed {}", meta.name);

            if downloaded { let _ = fs::remove_file(&pkg_path); }
            Ok(())
        }

        Command::Unpack { package, dir } => {
            let util = PkgUtil::new(root);
            util.pkg_unpack(&package, &dir)?;
            println!("Unpacked {} into {}", package.display(), dir.display());
            Ok(())
        }

        Command::Remove { package } => {
            let mut util = PkgUtil::new(root);
            util.set_dry_run(cli.dry_run);
            util.db_open(util.dry_run)?;

            if !util.db_find_package(&package) {
                anyhow::bail!("{} not installed", package);
            }

            if util.dry_run {
                println!("[DRY RUN] Would remove package: {}", package);
                if let Some(pkg) = util.packages.get(&package) {
                    println!("  Files to remove:");
                    for f in &pkg.files {
                        println!("    {}", f);
                    }
                }
                return Ok(());
            }

            util.db_remove_package(&package);
            util.db_commit()?;
            util.run_ldconfig()?;
            println!("Removed {}", package);
            Ok(())
        }

        Command::Info { installed, list, owner, footprint } => {
            if let Some(fp) = footprint {
                let util = PkgUtil::new(root);
                return util.pkg_footprint(&fp);
            }

            let mut util = PkgUtil::new(root);
            util.db_open(false)?;

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
            Ok(())
        }

        Command::Apply { config, profile } => {
            let mut util = PkgUtil::new(root);
            util.set_dry_run(cli.dry_run);
            util.pkg_apply(&config, profile.as_deref())?;
            Ok(())
        }

        Command::Update { config, profile } => {
            let mut util = PkgUtil::new(root);
            util.set_dry_run(cli.dry_run);
            util.pkg_update(&config, profile.as_deref())?;
            Ok(())
        }

        Command::Repo { action } => {
            let util = PkgUtil::new(root);
            match action {
                RepoAction::List => util.repo_list()?,
                RepoAction::Add { name, url } => util.repo_add(&name, &url)?,
                RepoAction::Remove { name } => util.repo_remove(&name)?,
                RepoAction::Update => util.repo_update()?,
            }
            Ok(())
        }

        Command::Search { query } => {
            let util = PkgUtil::new(root);
            util.search(&query)?;
            Ok(())
        }

        Command::Verify { package } => {
            let mut util = PkgUtil::new(root);
            util.db_open(false)?;
            util.verify(&package)?;
            Ok(())
        }

        Command::Clean { packages, repos } => {
            let util = PkgUtil::new(root);
            if packages {
                util.clean_package_cache()?;
            }
            if repos {
                util.clean_repo_cache()?;
            }
            if !packages && !repos {
                util.clean_package_cache()?;
                util.clean_repo_cache()?;
            }
            Ok(())
        }
    }
}
