// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-2.0

use anyhow::{Result, bail};
use semver::{Version, VersionReq};
use std::collections::{HashMap, HashSet};

use crate::pkgutil::PkgUtil;

pub type PkgVersion = Version;

/// Parse a version constraint into a semver VersionReq.
fn parse_constraint(constraint: &str) -> Result<VersionReq> {
    let constraint = constraint.trim().replace('*', "x");
    VersionReq::parse(&constraint)
        .map_err(|e| anyhow::anyhow!("invalid constraint '{}': {}", constraint, e))
}

/// Resolve dependencies for a root package.
pub fn resolve_dependencies(
    util: &PkgUtil,
    root_name: &str,
    root_version: &str,
) -> Result<HashMap<String, PkgVersion>> {
    let mut resolved = HashMap::new();
    let mut visited = HashSet::new();
    let root_ver = Version::parse(root_version)?;
    resolve_rec(util, root_name, root_ver, &mut resolved, &mut visited)?;
    Ok(resolved)
}

fn resolve_rec(
    util: &PkgUtil,
    name: &str,
    version: PkgVersion,
    resolved: &mut HashMap<String, PkgVersion>,
    visited: &mut HashSet<String>,
) -> Result<()> {
    // Check for cycles
    if visited.contains(name) {
        bail!("Circular dependency detected: {}", name);
    }

    // Check if already resolved with same version
    if let Some(existing) = resolved.get(name) {
        if *existing == version {
            return Ok(());
        } else {
            bail!("Version conflict for {}: requested {}, already resolved {}", name, version, existing);
        }
    }

    // Mark as visiting
    visited.insert(name.to_string());

    // Get package info from repository
    let info = util.get_package_info(name)?;
    let pkg_ver = Version::parse(&info.version)?;
    if pkg_ver != version {
        bail!("Version mismatch for {}: requested {}, available {}", name, version, pkg_ver);
    }

    // Resolve dependencies
    for (dep_name, dep_constraint) in &info.dependencies {
        let req = parse_constraint(dep_constraint)?;
        let available_versions = util.get_available_versions(dep_name);
        let best_version = available_versions
            .into_iter()
            .filter(|v| req.matches(v))
            .max()
            .ok_or_else(|| anyhow::anyhow!(
                "No version of {} satisfies constraint {}",
                dep_name, dep_constraint
            ))?;
        resolve_rec(util, dep_name, best_version, resolved, visited)?;
    }

    // Mark as resolved
    resolved.insert(name.to_string(), version);
    visited.remove(name);
    Ok(())
}
