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

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

use crate::metadata::InstalledPkg;
use crate::pkgutil::{PkgUtil};

pub struct PkgAdd {
    pub util: PkgUtil,
}

impl PkgAdd {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            util: PkgUtil::new("pkgm install", root),
        }
    }

    /// Execute package installation or upgrade workflow.
    pub fn run_install(&mut self, package: PathBuf, upgrade: bool, force: bool) -> Result<()> {
        self.util.db_open(&self.util.root.clone())
            .map_err(|e| anyhow!("{}", e))?;

        let (meta, files) = self.util.pkg_open(&package)
            .with_context(|| format!("Failed to read package archive {}", package.display()))?;

        let installed = self.util.db_find_pkg(&meta.name);

        if installed && !upgrade {
            return Err(anyhow!("Package {} is already installed. Use -u / --upgrade", meta.name));
        } else if !installed && upgrade {
            return Err(anyhow!("Package {} is not installed yet", meta.name));
        }

        let conflicts = self.util.db_find_conflicts(&meta.name, &files);
        if !conflicts.is_empty() && !force {
            eprintln!("Conflicting files found:");
            for f in &conflicts {
                eprintln!("  {f}");
            }
            return Err(anyhow!("Use -f / --force to overwrite conflicting files"));
        }

        if upgrade {
            self.util.db_rm_pkg(&meta.name);
        }

        self.util.pkg_install(&package)
            .map_err(|e| anyhow!("Extraction failed: {}", e))?;

        self.util.db_add_pkg(InstalledPkg {
            name: meta.name.clone(),
            version: meta.version,
            description: meta.description,
            files,
        });

        self.util.db_commit().context("Failed to commit package database")?;

        let _ = self.util.ldconfig();
        println!("Successfully installed {}", meta.name);

        Ok(())
    }
}
