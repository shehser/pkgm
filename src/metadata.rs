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

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// Metadata stored inside the package archive (metadata.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkgMetadata {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
}

/// Representation of an installed package inside the JSON database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPkg {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub files: BTreeSet<String>,
}

/// Robust extraction of package name and version from filename.
pub fn parse_package_name(filename: &str) -> Result<(String, String), String> {
    let file_name_only = Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename);

    let stem = file_name_only
        .strip_suffix(".pkg.tar.gz")
        .or_else(|| file_name_only.strip_suffix(".tar.gz"))
        .unwrap_or(file_name_only);

    let (name, version) = match stem.rsplit_once('-') {
        Some((n, v)) => (n.to_string(), v.to_string()),
        None => (stem.to_string(), "0.0".to_string()),
    };

    Ok((name, version))
}
