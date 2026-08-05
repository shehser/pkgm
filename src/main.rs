// Copyright (C) 2026 Yersultan Muapyqov
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub mod metadata;
pub mod pkgutil;
pub mod pkgadd;
pub mod pkginfo;
pub mod pkgrm;

use crate::pkgadd::PkgAdd;
use crate::pkginfo::PkgInfoUtil;
use crate::pkgrm::PkgRm;

#[derive(Parser)]
#[command(name = "pkgm", version, about = "Package manager in Rust", propagate_version = true)]
pub struct Cli {
    /// Specify alternative installation root
    #[arg(short, long, global = true)]
    pub root: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Install or upgrade a package
    Install {
        /// Package archive file (.pkg.tar.gz)
        package: PathBuf,

        /// Upgrade package with the same name
        #[arg(short, long)]
        upgrade: bool,

        /// Force install, overwrite conflicting files
        #[arg(short, long)]
        force: bool,
    },
    /// Unpack archive content to a directory without modifying database
    Unpack {
        /// Package archive file (.pkg.tar.gz)
        package: PathBuf,

        /// Target directory (default: current directory)
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    /// Remove an installed package
    Remove {
        /// Name of the package to remove
        package: String,
    },
    /// Inspect database, installed packages, or footprint
    Info {
        /// List installed packages
        #[arg(short, long)]
        installed: bool,

        /// List files in installed package or package archive
        #[arg(short, long)]
        list: Option<String>,

        /// List owner(s) of file(s) matching pattern
        #[arg(short, long)]
        owner: Option<String>,

        /// Print footprint for package file
        #[arg(short, long)]
        footprint: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root_path = cli.root.unwrap_or_default();

    match cli.command {
        Commands::Install { package, upgrade, force } => {
            let mut adder = PkgAdd::new(root_path);
            adder.run_install(package, upgrade, force)?;
        }
        Commands::Unpack { package, dir } => {
            let util = pkgutil::PkgUtil::new("pkgm unpack", root_path);
            util.pkg_unpack(&package, &dir)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            println!("Unpacked {} into {}", package.display(), dir.display());
        }
        Commands::Remove { package } => {
            let mut remover = PkgRm::new(root_path);
            remover.run_remove(&package)?;
        }
        Commands::Info { installed, list, owner, footprint } => {
            let mut info_util = PkgInfoUtil::new(root_path);
            info_util.run_info(installed, list, owner, footprint)?;
        }
    }

    Ok(())
}
