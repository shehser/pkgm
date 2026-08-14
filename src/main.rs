// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-2.0

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::thread;

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
    #[arg(long, global = true, help = "Disable automatic repository update")]
    no_auto_update: bool,
    #[arg(long, global = true, help = "Output in JSON format")]
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
    Checkupdates,
    CheckConfig {
        #[arg(short, long, default_value = "pkgm.toml")]
        config: PathBuf,
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
    env_logger::init();
    let cli = Cli::parse();
    let root = cli.root.unwrap_or_default();

    match cli.command {
        Command::Install { packages, upgrade, force } => {
            let mut util = PkgUtil::new(root);
            util.set_dry_run(cli.dry_run);
            util.set_no_auto_update(cli.no_auto_update);
            let dry = util.dry_run;

            util.db_open(dry)?;

            // Resolve all packages
            let mut resolved = Vec::new();
            for package in &packages {
                if package.exists() {
                    let (_name, ver) = metadata::parse_package_name(
                        package.file_name().unwrap_or_default().to_str().unwrap_or("")
                    );
                    resolved.push((package.clone(), None, false, ver, None));
                } else {
                    let name = package.to_string_lossy().to_string();
                    let (url, ver, checksum_opt) = util.resolve_package(&name)?;
                    resolved.push((package.clone(), Some((url, ver.clone(), checksum_opt)), true, ver, Some(name)));
                }
            }

            if dry {
                for (package, remote_info, _, _version, name_opt) in &resolved {
                    let name = name_opt.as_deref().unwrap_or(package.file_name().unwrap_or_default().to_str().unwrap_or(""));
                    if let Some((url, ver, checksum_opt)) = remote_info {
                        println!("[DRY RUN] Found {} version {} in repository", name, ver);
                        println!("[DRY RUN] Would download from {}", url);
                        println!("[DRY RUN] Would verify checksum: {:?}", checksum_opt);
                    } else {
                        println!("[DRY RUN] Local package: {}", package.display());
                    }
                }
                println!("[DRY RUN] No changes applied.");
                return Ok(());
            }

            // Parallel download remote packages
            let mut handles = Vec::new();
            for (_idx, (_, remote_info, _, _, name_opt)) in resolved.iter().enumerate() {
                if let Some((url, ver, checksum_opt)) = remote_info {
                    let name = name_opt.as_ref().unwrap().clone();
                    let url = url.clone();
                    let ver = ver.clone();
                    let checksum_opt = checksum_opt.clone();
                    let util_clone = util.clone_for_download();
                    handles.push(thread::spawn(move || {
                        util_clone.download_package(&url, &name, &ver, checksum_opt.as_deref())
                    }));
                }
            }

            let mut downloaded_paths = Vec::new();
            for handle in handles {
                let result = handle.join().unwrap();
                downloaded_paths.push(result?);
            }

            let mut pkg_paths = Vec::new();
            let mut download_idx = 0;
            for (package, remote_info, _, _, _) in resolved {
                if remote_info.is_some() {
                    pkg_paths.push(downloaded_paths[download_idx].clone());
                    download_idx += 1;
                } else {
                    pkg_paths.push(package.clone());
                }
            }

            let mut any_installed = false;
            for pkg_path in pkg_paths {
                let (meta, files) = util.pkg_open(&pkg_path)?;
                let name = meta.name.clone();

                let already = util.db_find_package(&name);
                if already && !upgrade {
                    anyhow::bail!("{} already installed (use --upgrade)", name);
                }
                if !already && upgrade {
                    anyhow::bail!("{} not installed", name);
                }

                let conflicts = util.db_find_conflicts(&name, &files);
                if !conflicts.is_empty() && !force {
                    eprintln!("Conflicting files for {}:", name);
                    for f in &conflicts { eprintln!("  {}", f); }
                    anyhow::bail!("use --force to overwrite");
                }

                if upgrade {
                    util.db_remove_package(&name);
                }
                util.pkg_install(&pkg_path)?;
                util.db_add_package(InstalledPkg {
                    name: meta.name.clone(),
                    version: meta.version,
                    description: meta.description,
                    files,
                    checksum: None,
                });

                if pkg_path.starts_with(std::env::temp_dir()) {
                    let _ = fs::remove_file(&pkg_path);
                }

                println!("Installed {}", meta.name);
                any_installed = true;
            }

            if any_installed {
                util.db_commit()?;
                util.run_ldconfig()?;
            }
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
            util.set_no_auto_update(cli.no_auto_update);
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
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&util.packages)?);
                } else {
                    for (name, pkg) in &util.packages {
                        println!("{} {}", name, pkg.version);
                    }
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
            util.set_no_auto_update(cli.no_auto_update);
            util.pkg_apply(&config, profile.as_deref())?;
            Ok(())
        }

        Command::Update { config, profile } => {
            let mut util = PkgUtil::new(root);
            util.set_dry_run(cli.dry_run);
            util.set_no_auto_update(cli.no_auto_update);
            util.pkg_update(&config, profile.as_deref())?;
            Ok(())
        }

        Command::Repo { action } => {
            let util = PkgUtil::new(root);
            match action {
                RepoAction::List => {
                    let repos = util.read_repos()?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&repos)?);
                    } else {
                        if repos.is_empty() {
                            println!("No repositories configured.");
                        } else {
                            println!("Repositories:");
                            for (name, url) in &repos {
                                println!("  {}: {}", name, url);
                            }
                        }
                    }
                }
                RepoAction::Add { name, url } => util.repo_add(&name, &url)?,
                RepoAction::Remove { name } => util.repo_remove(&name)?,
                RepoAction::Update => util.repo_update()?,
            }
            Ok(())
        }

        Command::Search { query } => {
            let mut util = PkgUtil::new(root);
            util.set_no_auto_update(cli.no_auto_update);
            let results = util.search_json(&query)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                util.search(&query)?;
            }
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

        Command::Checkupdates => {
            let mut util = PkgUtil::new(root);
            util.set_no_auto_update(cli.no_auto_update);
            util.db_open(false)?;
            let updates = util.check_updates()?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&updates)?);
            } else {
                if updates.is_empty() {
                    println!("All packages are up to date.");
                } else {
                    println!("Updates available:");
                    for update in &updates {
                        println!("  {} {} -> {}", update.name, update.current, update.latest);
                    }
                }
            }
            Ok(())
        }

        Command::CheckConfig { config } => {
            let util = PkgUtil::new(root);
            util.check_config(&config)?;
            Ok(())
        }
    }
}
