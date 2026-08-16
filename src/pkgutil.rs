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

const DB_JSON: &str = "var/lib/pkgm/pkgdb.json";
const DB_LOCK: &str = "var/lib/pkgm/pkgdb.lock";
const META_FILE: &str = "metadata.json";
const LDCONFIG: &str = "/usr/bin/ldconfig";
const REPO_FILE: &str = "etc/pkgm/repo.txt";
const CACHE_DIR: &str = "var/cache/pkgm";
const REPO_CACHE: &str = "var/cache/pkgm/repo.json";
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
            .use_preconfigured_tls(webpki_roots::TLS_SERVER_ROOTS)
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
            self.packages = serde_json::from_reader(BufReader::new(f)).unwrap_or_default();
        } else {
            self.packages.clear();
        }
        Ok(())
    }

    pub fn db_commit(&self) -> Result<()> {
        let path = self.root.join(DB_JSON);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
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

    pub fn pkg_install(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let f = File::open(path)?;
        let mut archive = Archive::new(GzDecoder::new(f));
        let root = if self.root.exists() {
            fs::canonicalize(&self.root)?
        } else {
            self.root.clone()
        };

        for entry in archive.entries()? {
            let mut entry = entry?;
            self.unpack_entry(&mut entry, &root)?;
        }
        Ok(())
    }

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

    fn extract_nested(&self, entry_path: &Path, dest_path: &Path) -> Result<()> {
        let s = entry_path.to_string_lossy();
        if !(s.ends_with(".tar.gz") || s.ends_with(".tgz")) {
            return Ok(());
        }
        let f = File::open(dest_path)?;
        let mut nested = Archive::new(GzDecoder::new(f));
        let root = if self.root.exists() {
            fs::canonicalize(&self.root)?
        } else {
            self.root.clone()
        };
        for entry in nested.entries()? {
            let mut entry = entry?;
            self.unpack_entry(&mut entry, &root)?;
        }
        fs::remove_file(dest_path)?;
        Ok(())
    }

    pub fn run_ldconfig(&self) -> Result<()> {
        let ldconfig_path = self.root.join(LDCONFIG.trim_start_matches('/'));
        if !ldconfig_path.exists() {
            return Ok(());
        }
        let status = Command::new(&ldconfig_path)
            .arg("-r")
            .arg(&self.root)
            .status()?;
        if !status.success() {
            bail!("ldconfig failed");
        }
        Ok(())
    }

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
        let mut repos = self.repo_list()?;
        let trimmed_url = url.trim().trim_end_matches('/').to_string();
        if !repos.contains(&trimmed_url) {
            repos.push(trimmed_url);
            if let Some(parent) = self.repo_path().parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(self.repo_path(), repos.join("\n"))?;
        }
        Ok(())
    }

    pub fn repo_remove(&self, url: &str) -> Result<()> {
        let repos = self.repo_list()?;
        let trimmed_target = url.trim().trim_end_matches('/');
        let updated: Vec<String> = repos.into_iter().filter(|r| r != trimmed_target).collect();

        if updated.is_empty() {
            if self.repo_path().exists() {
                fs::remove_file(self.repo_path())?;
            }
        } else {
            fs::write(self.repo_path(), updated.join("\n"))?;
        }
        Ok(())
    }

    pub fn repo_list(&self) -> Result<Vec<String>> {
        if !self.repo_path().exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(self.repo_path())?;
        let repos = content
            .lines()
            .map(|l| l.trim().trim_end_matches('/').to_string())
            .filter(|l| !l.is_empty())
            .collect();
        Ok(repos)
    }

    pub fn repo_update(&self) -> Result<()> {
        let repos = self.repo_list()?;
        if repos.is_empty() {
            return Ok(());
        }

        let mut combined_index: HashMap<String, serde_json::Value> = HashMap::new();
        let mut total_pkgs = 0;

        for repo_url in repos {
            let index_url = format!("{}/index.json", repo_url);
            let resp = match self.http.get(&index_url).send() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Warning: Failed to fetch {}: {}", index_url, e);
                    continue;
                }
            };

            if !resp.status().is_success() {
                eprintln!("Warning: HTTP {} for {}", resp.status(), index_url);
                continue;
            }

            let text = match resp.text() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Warning: Failed to read response from {}: {}", index_url, e);
                    continue;
                }
            };

            let json: serde_json::Value = match serde_json::from_str(&text) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("Warning: Invalid JSON from {}: {}", index_url, e);
                    continue;
                }
            };

            let packages = if let Some(pkgs) = json.get("packages") {
                pkgs.as_object()
            } else {
                json.as_object()
            };

            if let Some(pkgs) = packages {
                for (name, val) in pkgs {
                    combined_index.insert(name.clone(), val.clone());
                    total_pkgs += 1;
                }
            }
        }

        let cache_path = self.root.join(REPO_CACHE);
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut final_map = HashMap::new();
        final_map.insert("packages", serde_json::to_value(&combined_index)?);
        fs::write(&cache_path, serde_json::to_string_pretty(&final_map)?)?;

        println!("Repository cache updated ({} packages loaded)", total_pkgs);
        Ok(())
    }

    pub fn resolve_package(&self, name: &str) -> Result<(String, String, Option<String>)> {
        let cache_path = self.root.join(REPO_CACHE);
        if !cache_path.exists() {
            bail!("Repository cache not found. Run 'pkgm repo update'.");
        }

        let content = fs::read_to_string(&cache_path)?;
        let index: serde_json::Value = serde_json::from_str(&content)?;

        let packages = index.get("packages")
            .and_then(|p| p.as_object())
            .ok_or_else(|| anyhow::anyhow!("Invalid repository cache format"))?;

        let pkg_info = packages.get(name)
            .ok_or_else(|| anyhow::anyhow!("Package '{}' not found in repositories", name))?;

        let version = pkg_info["version"].as_str()
            .ok_or_else(|| anyhow::anyhow!("Version missing for {}", name))?
            .to_string();

        let url = pkg_info["url"].as_str()
            .ok_or_else(|| anyhow::anyhow!("URL missing for {}", name))?
            .to_string();

        let checksum = pkg_info["checksum"].as_str().map(|s| s.to_string());

        Ok((version, url, checksum))
    }

    pub fn search(&self, query: &str) -> Result<Vec<(String, String, String)>> {
        let cache_path = self.root.join(REPO_CACHE);
        if !cache_path.exists() {
            bail!("Repository cache not found. Run 'pkgm repo update'.");
        }

        let content = fs::read_to_string(&cache_path)?;
        let index: serde_json::Value = serde_json::from_str(&content)?;

        let packages = match index.get("packages").and_then(|p| p.as_object()) {
            Some(p) => p,
            None => return Ok(Vec::new()),
        };

        let mut results = Vec::new();
        let query_lower = query.to_lowercase();

        for (name, info) in packages {
            if name.to_lowercase().contains(&query_lower) {
                let version = info["version"].as_str().unwrap_or("0.0.1").to_string();
                let desc = info["description"].as_str().unwrap_or("").to_string();
                results.push((name.clone(), version, desc));
            }
        }

        Ok(results)
    }

    pub fn check_updates(&self) -> Result<Vec<(String, String, String)>> {
        let mut updates = Vec::new();
        for (name, installed) in &self.packages {
            if let Ok((latest_version, _, _)) = self.resolve_package(name) {
                if installed.version != latest_version {
                    updates.push((name.clone(), installed.version.clone(), latest_version));
                }
            }
        }
        Ok(updates)
    }

    pub fn clean_cache(&self) -> Result<()> {
        let dir = self.root.join(CACHE_DIR);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

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
            bail!("HTTP {} for {}", resp.status(), url);
        }
        let bytes = resp.bytes()?;
        fs::write(&cached_path, &bytes)?;
        debug!("Downloaded and cached: {}", cached_path.display());
        Ok(cached_path)
    }
}
