// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-2.0

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkgMetadata {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPkg {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub files: BTreeSet<String>,
    pub checksum: Option<String>,
}

/// Parse name and version from filename: name-version.tar.gz
pub fn parse_package_name(filename: &str) -> (String, String) {
    let file_name_only = Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename);
    let stem = file_name_only
        .strip_suffix(".tar.gz")
        .or_else(|| file_name_only.strip_suffix(".tgz"))
        .unwrap_or(file_name_only);
    match stem.rsplit_once('-') {
        Some((n, v)) => (n.to_string(), v.to_string()),
        None => (stem.to_string(), "0.0.1".to_string()),
    }
}
