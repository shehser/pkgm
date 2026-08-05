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

use anyhow::{anyhow, Result};
use regex::Regex;
use std::path::{Path, PathBuf};
use crate::pkgutil::{PkgUtil};

pub struct PkgInfoUtil {
    pub util: PkgUtil,
}

impl PkgInfoUtil {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            util: PkgUtil::new("pkgm info", root),
        }
    }

    /// Query package database or package archive details.
    pub fn run_info(
        &mut self,
        installed: bool,
        list: Option<String>,
        owner: Option<String>,
        footprint: Option<PathBuf>,
    ) -> Result<()> {
        if let Some(fp_path) = footprint {
            self.util.pkg_footprint(&fp_path)
                .map_err(|e| anyhow!("Footprint error: {}", e))?;
            return Ok(());
        }
        self.util.db_open(&self.util.root.clone())
            .map_err(|e| anyhow!("{}", e))?;

        if installed {
            for (name, pkg) in &self.util.packages {
                println!("{} {}", name, pkg.version);
            }
        } else if let Some(target) = list {
            if self.util.db_find_pkg(&target) {
                if let Some(pkg) = self.util.packages.get(&target) {
                    for file in &pkg.files {
                        println!("{}", file);
                    }
                }
            } else if Path::new(&target).exists() {
                let (_, files) = self.util.pkg_open(&target)
                    .map_err(|e| anyhow!("{}", e))?;
                for file in &files {
                    println!("{}", file);
                }
            } else {
                return Err(anyhow!("{} is neither an installed package nor a valid archive", target));
            }
        } else if let Some(pattern) = owner {
            let preg = Regex::new(&pattern)
                .map_err(|_| anyhow!("Invalid regular expression '{}'", pattern))?;

            let mut result: Vec<(String, String)> = Vec::new();
            result.push(("Package".to_string(), "File".to_string()));
            let mut width = result[0].0.len();

            for (name, pkg) in &self.util.packages {
                for file in &pkg.files {
                    let absolute_file = format!("/{}", file);
                    if preg.is_match(&absolute_file) {
                        result.push((name.clone(), file.clone()));
                        if name.len() > width {
                            width = name.len();
                        }
                    }
                }
            }

            if result.len() > 1 {
                for (pkg, file) in &result {
                    println!("{:width$}  {}", pkg, file, width = width);
                }
            } else {
                println!("No owner(s) found for pattern: {}", pattern);
            }
        } else {
            return Err(anyhow!("No flag provided. Use --help to view options"));
        }

        Ok(())
    }
}
