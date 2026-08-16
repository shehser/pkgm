// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-2.0

use anyhow::{Result, bail};
use flate2::read::GzDecoder;
use fs2::FileExt;
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tar::{Archive, Entry};
use crate::metadata::{parse_package_name, InstalledPkg, PkgMetadata};
use tempfile::NamedTempFile;
use log::debug;

const DB_JSON: &str = "db/pkgdb.json";
const DB_LOCK: &str = "db/pkgdb.lock";
const META_FILE: &str = "metadata.json";
const LDCONFIG: &str = "/usr/bin/ldconfig";
const REPO_FILE: &str = "repo.txt";
const CACHE_DIR: &str = "cache/pkgm";
const HTTP_TIMEOUT_SECS: u64 = 30;

pub type Packages = HashMap<String, InstalledPkg>;

pub struct PkgUtil {
    pub packages: Packages,
    pub root: PathBuf,
    http: reqwest::blocking::Client,
    _db_lock: Option<File>,
    pub dry_run: bool,
}

impl PkgUtil {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self {
            packages: HashMap::new(),
            root: root.into(),
            http,
            _db_lock: None,
            dry_run: false,
        }
    }

    pub fn set_dry_run(&mut self, dry: bool) {
        self.dry_run = dry;
    }

    /// Open package database with appropriate lock.
    /// Shared lock for readonly, exclusive for writes.
    pub fn db_open(&mut self, readonly: bool) -> Result<()> {
        let path = self.root.join(DB_JSON);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let lock_path = self.root.join(DB_LOCK);
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&lock_path)?;

        if readonly {
            lock_file.try_lock_shared()?;
        } else {
            lock_file.try_lock_exclusive()?;
        }
        self._db_lock = Some(lock_file);

        if path.exists() {
            let f = File::open(&path)?;
            self.packages = serde_json::from_reader(BufReader::new(f))?;
        } else {
            self.packages.clear();
        }
        Ok(())
    }

    /// Atomic database commit using temporary file.
    pub fn db_commit(&self) -> Result<()> {
        let path = self.root.join(DB_JSON);
        fs::create_dir_all(path.parent().unwrap())?;
        let tmp = NamedTempFile::new_in(path.parent().unwrap())?;
        serde_json::to_writer_pretty(tmp.as_file(), &self.packages)?;
        tmp.persist(&path)?;
        Ok(())
    }

    pub fn db_find_package(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    pub fn db_add_package(&mut self, pkg: InstalledPkg) {
        self.packages.insert(pkg.name.clone(), pkg);
    }

    /// Remove package and delete files not owned by other packages.
    pub fn db_remove_package(&mut self, name: &str) {
        let Some(pkg) = self.packages.remove(name) else { return };
        let mut files = pkg.files;
        for other in self.packages.values() {
            for f in &other.files {
                files.remove(f);
            }
        }
        for path in files.iter().rev() {
            self.remove_path(path);
        }
    }

    /// Find conflicting files with installed packages or existing files on disk.
    pub fn db_find_conflicts(&self, name: &str, files: &BTreeSet<String>) -> BTreeSet<String> {
        let mut conflicts = BTreeSet::new();
        for (pkg, info) in &self.packages {
            if pkg != name {
                conflicts.extend(files.intersection(&info.files).cloned());
            }
        }
        for f in files {
            if self.root.join(f).exists() {
                conflicts.insert(f.clone());
            }
        }
        if let Some(owned) = self.packages.get(name) {
            for f in &owned.files {
                conflicts.remove(f);
            }
        }
        conflicts
    }

    /// Open archive and extract metadata + file list.
    pub fn pkg_open(&self, path: impl AsRef<Path>) -> Result<(PkgMetadata, BTreeSet<String>)> {
        let path = path.as_ref();
        let f = File::open(path)?;
        let mut archive = Archive::new(GzDecoder::new(f));
        let mut meta = None;
        let mut files = BTreeSet::new();

        for entry in archive.entries()? {
            let entry = entry?;
            let p = entry.path()?;
            let s = p.to_string_lossy();
            if s == META_FILE {
                meta = Some(serde_json::from_reader(entry)?);
            } else {
                files.insert(s.into_owned());
            }
        }

        let meta = match meta {
            Some(m) => m,
            None => {
                let stem = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                let (name, version) = parse_package_name(stem);
                PkgMetadata {
                    name,
                    version,
                    description: Some("auto".into()),
                }
            }
        };
        Ok((meta, files))
    }

    /// Install package into managed root.
    pub fn pkg_install(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let f = File::open(path)?;
        let mut archive = Archive::new(GzDecoder::new(f));
        let root = fs::canonicalize(&self.root)?;

        for entry in archive.entries()? {
            let mut entry = entry?;
            self.unpack_entry(&mut entry, &root)?;
        }
        Ok(())
    }

    /// Unpack a single archive entry, rejecting unsafe paths.
    fn unpack_entry<R: Read>(&self, entry: &mut Entry<R>, dest_dir: &Path) -> Result<()> {
        let p = entry.path()?.to_path_buf();
        if p.to_string_lossy() == META_FILE {
            return Ok(());
        }
        if p.is_absolute() || p.components().any(|c| c == std::path::Component::ParentDir) {
            anyhow::bail!("unsafe path: {}", p.display());
        }
        let dest = dest_dir.join(&p);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        entry.set_unpack_xattrs(true);
        entry.set_preserve_permissions(true);
        entry.set_preserve_mtime(true);
        entry.unpack(&dest)?;
        self.extract_nested(&p, &dest)?;
        Ok(())
    }

    /// Extract nested tarballs and remove intermediate file.
    fn extract_nested(&self, entry_path: &Path, dest_path: &Path) -> Result<()> {
        let s = entry_path.to_string_lossy();
        if !(s.ends_with(".tar.gz") || s.ends_with(".tgz")) {
            return Ok(());
        }
        let f = File::open(dest_path)?;
        let mut nested = Archive::new(GzDecoder::new(f));
        let root = fs::canonicalize(&self.root)?;
        for entry in nested.entries()? {
            let mut entry = entry?;
            self.unpack_entry(&mut entry, &root)?;
        }
        fs::remove_file(dest_path)?;
        Ok(())
    }

    /// Run ldconfig inside managed root (no-op if missing).
    pub fn run_ldconfig(&self) -> Result<()> {
        if !Path::new(LDCONFIG).exists() {
            return Ok(());
        }
        let status = Command::new(LDCONFIG)
            .arg("-r")
            .arg(&self.root)
            .status()?;
        if !status.success() {
            bail!("ldconfig failed");
        }
        Ok(())
    }

    /// Safe path removal using trimmed path to avoid root escape.
    fn remove_path(&self, file: &str) {
        let trimmed = file.trim_start_matches(['.', '/']);
        if trimmed.is_empty() {
            return;
        }
        let path = self.root.join(trimmed);
        if !path.exists() {
            return;
        }
        let res = if path.is_dir() {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        };
        if let Err(e) = res {
            let code = e.raw_os_error();
            if code != Some(libc::ENOTEMPTY) && code != Some(libc::EINVAL) {
                eprintln!("warning: cannot remove {}: {}", path.display(), e);
            }
        }
    }

    // ---- Repository management ----

    fn repo_path(&self) -> PathBuf {
        self.root.join(REPO_FILE)
    }

    pub fn repo_add(&self, url: &str) -> Result<()> {
        fs::write(self.repo_path(), url)?;
        Ok(())
    }

    pub fn repo_remove(&self) -> Result<()> {
        if self.repo_path().exists() {
            fs::remove_file(self.repo_path())?;
        }
        Ok(())
    }

    pub fn repo_list(&self) -> Result<Option<String>> {
        if !self.repo_path().exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(self.repo_path())?;
        Ok(Some(content.trim().to_string()))
    }

    // ---- Package resolution ----

    /// Resolve package from repository: returns (version, url, checksum)
    pub fn resolve_package(&self, name: &str) -> Result<(String, String, Option<String>)> {
        let repo_url = self.repo_list()?
            .ok_or_else(|| anyhow::anyhow!("no repository configured"))?;

        let resp = self.http.get(&format!("{}/index.json", repo_url)).send()?;
        if !resp.status().is_success() {
            bail!("failed to fetch index from {}", repo_url);
        }
        let index: HashMap<String, serde_json::Value> = serde_json::from_reader(resp.text()?.as_bytes())?;
        let pkg_info = index.get(name)
            .ok_or_else(|| anyhow::anyhow!("package {} not found", name))?;
        let version = pkg_info["version"].as_str().unwrap_or("0.0.1").to_string();
        let url = format!("{}/{}-{}.tar.gz", repo_url, name, version);
        let checksum = pkg_info["checksum"].as_str().map(|s| s.to_string());
        Ok((version, url, checksum))
    }

    // ---- Search ----

    pub fn search(&self, query: &str) -> Result<Vec<(String, String, String)>> {
        let repo_url = self.repo_list()?
            .ok_or_else(|| anyhow::anyhow!("no repository configured"))?;

        let resp = self.http.get(&format!("{}/index.json", repo_url)).send()?;
        if !resp.status().is_success() {
            bail!("failed to fetch index");
        }
        let index: HashMap<String, serde_json::Value> = serde_json::from_reader(resp.text()?.as_bytes())?;
        let mut results = Vec::new();
        let query_lower = query.to_lowercase();
        for (name, info) in index {
            if name.to_lowercase().contains(&query_lower) {
                let version = info["version"].as_str().unwrap_or("0.0.1").to_string();
                let desc = info["description"].as_str().unwrap_or("").to_string();
                results.push((name, version, desc));
            }
        }
        Ok(results)
    }

    // ---- Check updates ----

    pub fn check_updates(&self) -> Result<Vec<(String, String, String)>> {
        let mut updates = Vec::new();
        for (name, installed) in &self.packages {
            let (latest_version, _, _) = self.resolve_package(name)?;
            if installed.version != latest_version {
                updates.push((name.clone(), installed.version.clone(), latest_version));
            }
        }
        Ok(updates)
    }

    // ---- Cache management ----

    pub fn clean_cache(&self) -> Result<()> {
        let dir = self.root.join(CACHE_DIR);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    // ---- Download ----

    pub fn download_package(&self, url: &str, name: &str, version: &str) -> Result<PathBuf> {
        let cache_dir = self.root.join(CACHE_DIR).join("packages");
        fs::create_dir_all(&cache_dir)?;
        let cached_path = cache_dir.join(format!("{}-{}.tar.gz", name, version));

        if cached_path.exists() {
            debug!("Using cached package: {}", cached_path.display());
            return Ok(cached_path);
        }

        let resp = self.http.get(url).send()?;
        if !resp.status().is_success() {
            bail!("HTTP {}", resp.status());
        }
        let bytes = resp.bytes()?;
        fs::write(&cached_path, &bytes)?;
        debug!("Downloaded and cached: {}", cached_path.display());
        Ok(cached_path)
    }

}
