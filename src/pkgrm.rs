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
use crate::pkgutil::{PkgUtil};

pub struct PkgRm {
    pub util: PkgUtil,
}

impl PkgRm {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            util: PkgUtil::new("pkgm remove", root),
        }
    }

    /// Execute package removal workflow.
    pub fn run_remove(&mut self, package: &str) -> Result<()> {
        self.util.db_open(&self.util.root.clone())
            .map_err(|e| anyhow!("{}", e))?;

        if !self.util.db_find_pkg(package) {
            return Err(anyhow!("Package {} not installed", package));
        }

        self.util.db_rm_pkg(package);
        let _ = self.util.ldconfig();

        self.util.db_commit().context("Failed to commit package database")?;
        println!("Successfully removed {}", package);

        Ok(())
    }
}
