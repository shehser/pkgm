// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-2.0

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
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

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub packages: HashMap<String, ManifestPkg>,
    #[serde(default)]
    pub profiles: HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ManifestPkg {
    pub url: String,
    pub version: String,
    #[serde(default)]
    pub checksum: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoPkg {
    pub version: String,
    pub description: Option<String>,
    pub url: String,
    pub checksum: Option<String>,
    #[serde(default)]
    pub dependencies: HashMap<String, String>, 
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoIndex {
    pub packages: HashMap<String, RepoPkg>,
}

pub fn parse_package_name(filename: &str) -> (String, String) {
    let file_name_only = Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename);

    let stem = file_name_only
        .strip_suffix(".pkg.tar.gz")
        .or_else(|| file_name_only.strip_suffix(".tar.gz"))
        .unwrap_or(file_name_only);

    match stem.rsplit_once('-') {
        Some((n, v)) => (n.to_string(), v.to_string()),
        None => (stem.to_string(), "0.0".to_string()),
    }
}
