// Copyright (C) 2026 Yersultan Muapyqov
// SPDX-License-Identifier: GPL-2.0

use anyhow::{Context, Ok, Result, bail};
use flate2::read::GzDecoder;
use fs2::FileExt;
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tar::{Archive, Entry, EntryType};
use users::{get_group_by_gid, get_user_by_uid};
use sha256::try_digest;
use crate::metadata::{parse_package_name, InstalledPkg, PkgMetadata, Manifest, ManifestPkg, RepoIndex};
use tempfile::NamedTempFile;

use log::{debug, warn};

const DB_JSON: &str = "db/pkgdb.json";
const DB_LOCK: &str = "db/pkgdb.lock";
const META_FILE: &str = "metadata.json";
const LDCONFIG: &str = "/usr/bin/ldconfig";
const REPOS_FILE: &str = "repos.toml";
const CACHE_DIR: &str = "cache/pkgm/repos";
const PACKAGES_CACHE_DIR: &str = "cache/pkgm/packages";
const HTTP_TIMEOUT_SECS: u64 = 30;
const CACHE_TTL_SECS: u64 = 86400; // 24 hours

pub type Packages = HashMap<String, InstalledPkg>;

struct FootprintEntry {
    path: String,
    mode: u32,
    uid: u64,
    gid: u64,
    size: u64,
    entry_type: EntryType,
    link_name: Option<String>,
}

pub struct PkgUtil {
    pub packages: Packages,
    pub root: PathBuf,
    http: reqwest::blocking::Client,
    _db_lock: Option<File>,
    pub dry_run: bool,
    pub no_auto_update: bool,
}

impl PkgUtil {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self {
            packages: HashMap::new(),
            root: root.into(),
            http,
            _db_lock: None,
            dry_run: false,
            no_auto_update: false,
        }
    }

    pub fn set_dry_run(&mut self, dry: bool) {
        self.dry_run = dry;
    }

    pub fn set_no_auto_update(&mut self, val: bool) {
        self.no_auto_update = val;
    }

    pub fn db_open(&mut self, readonly: bool) -> Result<()> {
        let path = self.root.join(DB_JSON);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create db dir")?;
        }

        let lock_path = self.root.join(DB_LOCK);
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("open lock file {}", lock_path.display()))?;

        if readonly {
            lock_file.try_lock_shared()
                .with_context(|| "DB is locked for writing by another process")?;
        } else {
            lock_file.try_lock_exclusive()
                .with_context(|| "DB is locked by another pkgm process")?;
        }
        self._db_lock = Some(lock_file);

        if path.exists() {
            let f = File::open(&path).with_context(|| format!("open {}", path.display()))?;
            self.packages = serde_json::from_reader(BufReader::new(f))
                .with_context(|| format!("parse {}", path.display()))?;
        } else {
            self.packages.clear();
        }
        Ok(())
    }

    pub fn db_commit(&self) -> Result<()> {
        let path = self.root.join(DB_JSON);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create db dir")?;
        }
        let tmp = NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))
            .context("create temporary db file")?;
        serde_json::to_writer_pretty(tmp.as_file(), &self.packages)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            .context("serialize db")?;

        tmp.as_file().sync_all().context("sync db")?;
        tmp.persist(&path)
            .map_err(|e| {
                eprintln!("warning : failed to commit database: {}" ,e);
                e
            })
            .context("commit db rename")?;
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
        let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut archive = Archive::new(GzDecoder::new(f));
        let mut meta = None;
        let mut files = BTreeSet::new();

        for entry in archive.entries().context("read archive")? {
            let entry = entry.context("corrupt archive entry")?;
            let p = entry.path().context("invalid path")?;
            let s = p.to_string_lossy();
            if s == META_FILE {
                meta = Some(
                    serde_json::from_reader(entry)
                        .with_context(|| format!("parse {} inside archive", META_FILE))?,
                );
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
        let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut archive = Archive::new(GzDecoder::new(f));
        let root = fs::canonicalize(&self.root).context("resolve root")?;

        for entry in archive.entries().context("read archive")? {
            let mut entry = entry.context("corrupt entry")?;
            self.unpack_entry(&mut entry, &root)?;
        }
        Ok(())
    }

    pub fn pkg_unpack(&self, path: impl AsRef<Path>, target: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let target = target.as_ref();
        let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut archive = Archive::new(GzDecoder::new(f));
        if !target.exists() {
            fs::create_dir_all(target).context("create target dir")?;
        }

        for entry in archive.entries().context("read archive")? {
            let mut entry = entry.context("corrupt entry")?;
            self.unpack_entry(&mut entry, target)?;
        }
        Ok(())
    }

    fn unpack_entry<R: Read>(&self, entry: &mut Entry<R>, dest_dir: &Path) -> Result<()> {
        let p = entry.path().context("invalid entry path")?.to_path_buf();
        if p.to_string_lossy() == META_FILE {
            return Ok(());
        }
        if p.is_absolute() || p.components().any(|c| c == std::path::Component::ParentDir) {
            anyhow::bail!(
                "refusing to unpack archive with unsafe path: {}",
                p.display()
            );
        }
        let dest = dest_dir.join(&p);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).context("create parent directory")?;
        }

        entry.set_unpack_xattrs(true);
        entry.set_preserve_permissions(true);
        entry.set_preserve_mtime(true);

        entry.unpack(&dest).with_context(|| format!("failed to unpack {}", dest.display()))?;
        self.extract_nested(&p, &dest)?;
        Ok(())
    }

    fn extract_nested(&self, entry_path: &Path, dest_path: &Path) -> Result<()> {
        let s = entry_path.to_string_lossy();
        if !(s.ends_with(".tar.gz") || s.ends_with(".tgz")) {
            return Ok(());
        }

        let f = File::open(dest_path).with_context(|| format!("open nested tar {}", dest_path.display()))?;
        let mut nested = Archive::new(GzDecoder::new(f));
        let root = fs::canonicalize(&self.root).context("resolve root")?;
        for entry in nested.entries().context("read nested archive")? {
            let mut entry = entry.context("corrupt nested entry")?;
            self.unpack_entry(&mut entry, &root)?;
        }
        fs::remove_file(dest_path).with_context(|| format!("remove nested archive {}", dest_path.display()))?;
        Ok(())
    }

    pub fn pkg_footprint(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut archive = Archive::new(GzDecoder::new(f));
        let mut entries = Vec::new();
        let mut hardlink_modes = HashMap::new();

        for entry in archive.entries().context("read archive")? {
            let entry = entry.context("corrupt entry")?;
            let header = entry.header();
            let p = entry.path().context("invalid path")?;
            let s = p.to_string_lossy().into_owned();
            if s == META_FILE {
                continue;
            }
            let mode = header.mode().context("read mode")?;
            let entry_type = header.entry_type();
            let link = header.link_name().context("read link")?.map(|p| p.to_string_lossy().into_owned());
            if !entry_type.is_hard_link() {
                hardlink_modes.insert(s.clone(), mode);
            }
            entries.push(FootprintEntry {
                path: s,
                mode,
                uid: header.uid().context("read uid")?,
                gid: header.gid().context("read gid")?,
                size: header.size().context("read size")?,
                entry_type,
                link_name: link,
            });
        }

        for e in &entries {
            self.print_footprint_entry(e, &hardlink_modes);
        }
        Ok(())
    }

    fn print_footprint_entry(&self, e: &FootprintEntry, hardlink_modes: &HashMap<String, u32>) {
        let mode = if e.entry_type.is_symlink() {
            "lrwxrwxrwx".to_string()
        } else if e.entry_type.is_hard_link() {
            let m = e.link_name.as_ref().and_then(|t| hardlink_modes.get(t)).copied().unwrap_or(e.mode);
            mode_to_string(m)
        } else {
            mode_to_string(e.mode)
        };
        print!("{}\t", mode);

        let user = get_user_by_uid(e.uid as u32)
            .map(|u| u.name().to_string_lossy().into_owned())
            .unwrap_or_else(|| e.uid.to_string());
        let group = get_group_by_gid(e.gid as u32)
            .map(|g| g.name().to_string_lossy().into_owned())
            .unwrap_or_else(|| e.gid.to_string());
        print!("{}/{}", user, group);
        print!("\t{}", e.path);

        if e.entry_type.is_symlink() {
            if let Some(ref t) = e.link_name {
                print!(" -> {}", t);
            }
        } else if e.entry_type.is_file() && e.size == 0 {
            print!(" (EMPTY)");
        }
        println!();
    }

    pub fn run_ldconfig(&self) -> Result<()> {
        if !Path::new(LDCONFIG).exists() {
            return Ok(());
        }
        let status = Command::new(LDCONFIG)
            .arg("-r")
            .arg(&self.root)
            .status()
            .context("run ldconfig")?;
        if !status.success() {
            bail!("ldconfig exited with status {}", status);
        }
        Ok(())
    }

    fn remove_path(&self, file: &str) {
        let trimmed = file.trim_start_matches(['.', '/']);
        if trimmed.is_empty() {
            return;
        }
        let path = self.root.join(file);
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

    fn read_manifest_packages(&self, config_path: &Path, profile: Option<&str>) -> Result<HashMap<String, ManifestPkg>> {
        let content = fs::read_to_string(config_path)
            .with_context(|| format!("read config {}", config_path.display()))?;
        let manifest: Manifest = toml::from_str(&content)
            .with_context(|| "parse config")?;

        match profile {
            Some(prof) => {
                let names = manifest.profiles.get(prof)
                    .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", prof))?;
                let mut selected = HashMap::new();
                for name in names {
                    if let Some(spec) = manifest.packages.get(name) {
                        selected.insert(name.clone(), spec.clone());
                    }
                }
                Ok(selected)
            }
            None => Ok(manifest.packages),
        }
    }

    fn sync_packages(&mut self, packages: HashMap<String, ManifestPkg>, remove_obsolete: bool) -> Result<()> {
        self.db_open(self.dry_run)?;

        if self.dry_run {
            println!("[DRY RUN] Would synchronize packages:");
            if remove_obsolete {
                for name in self.packages.keys() {
                    if !packages.contains_key(name) {
                        println!("  - remove {}", name);
                    }
                }
            }
            for (name, spec) in &packages {
                if let Some(installed) = self.packages.get(name) {
                    if installed.version != spec.version {
                        println!("  - upgrade {} {} -> {}", name, installed.version, spec.version);
                    } else {
                        println!("  - {} already up-to-date", name);
                    }
                } else {
                    println!("  - install {} {}", name, spec.version);
                }
            }
            return Ok(());
        }

        if remove_obsolete {
            let installed_names: Vec<String> = self.packages.keys().cloned().collect();
            for name in installed_names {
                if !packages.contains_key(&name) {
                    println!("Removing: {}", name);
                    self.db_remove_package(&name);
                }
            }
        }

        for (name, spec) in &packages {
            if self.db_find_package(name) {
                if let Some(installed) = self.packages.get(name) {
                    if installed.version == spec.version {
                        continue;
                    }
                    println!("Upgrading: {} {} -> {}", name, installed.version, spec.version);
                    self.db_remove_package(name);
                }
            }

            println!("Installing: {} {}", name, spec.version);
            let pkg_path = if is_remote_url(&spec.url) {
                self.download_package(&spec.url, name, &spec.version, spec.checksum.as_deref())?
            } else {
                PathBuf::from(&spec.url)
            };

            if let Some(expected) = &spec.checksum {
                if let Err(e) = self.verify_checksum(&pkg_path, expected) {
                    if is_remote_url(&spec.url) {
                        let _ = fs::remove_file(&pkg_path);
                    }
                    return Err(e);
                }
            }

            let (meta, files) = self.pkg_open(&pkg_path)?;
            let conflicts = self.db_find_conflicts(name, &files);
            if !conflicts.is_empty() {
                eprintln!("Conflicts detected for {}:", name);
                for f in &conflicts {
                    eprintln!("  {}", f);
                }
                if is_remote_url(&spec.url) {
                    let _ = fs::remove_file(&pkg_path);
                }
                bail!("aborting due to conflicts (use --force in install command)");
            }

            self.pkg_install(&pkg_path)?;
            self.db_add_package(InstalledPkg {
                name: meta.name.clone(),
                version: meta.version,
                description: meta.description,
                files,
                checksum: spec.checksum.clone(),
            });

            if is_remote_url(&spec.url) {
                let _ = fs::remove_file(&pkg_path);
            }
            println!("Installed: {}", name);
        }

        self.db_commit()?;
        let _ = self.run_ldconfig();
        Ok(())
    }

    pub fn pkg_apply(&mut self, config_path: &Path, profile: Option<&str>) -> Result<()> {
        let packages = self.read_manifest_packages(config_path, profile)?;
        self.sync_packages(packages, true)?;
        if !self.dry_run {
            println!("System synchronized successfully!");
        }
        Ok(())
    }

    pub fn pkg_update(&mut self, config_path: &Path, profile: Option<&str>) -> Result<()> {
        let packages = self.read_manifest_packages(config_path, profile)?;
        self.sync_packages(packages, false)?;
        if !self.dry_run {
            println!("All packages up to date!");
        }
        Ok(())
    }

    // ---- Repository management ----

    fn repos_path(&self) -> PathBuf {
        self.root.join(REPOS_FILE)
    }

    fn cache_dir(&self) -> PathBuf {
        self.root.join(CACHE_DIR)
    }

    fn packages_cache_dir(&self) -> PathBuf {
        self.root.join(PACKAGES_CACHE_DIR)
    }

    // Path to the timestamp file that records the last successful repo update.
    fn cache_timestamp_path(&self) -> PathBuf {
        self.cache_dir().join(".last_update")
    }

    // Returns true if the repository cache is older than CACHE_TTL_SECS or missing.
    fn is_repo_cache_stale(&self) -> bool {
        let path = self.cache_timestamp_path();
        if !path.exists() {
            return true;
        }
        let last = fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now.saturating_sub(last) > CACHE_TTL_SECS
    }

    // Update the repository cache automatically if it is stale and auto-update is not disabled.
    pub fn ensure_repo_cache_fresh(&self) -> Result<()> {
        if self.no_auto_update {
            return Ok(());
        }
        if self.is_repo_cache_stale() {
            println!("Repository cache is stale. Updating automatically...");
            self.repo_update()?;
        }
        Ok(())
    }

    pub fn clean_package_cache(&self) -> Result<()> {
        let dir = self.packages_cache_dir();
        if dir.exists() {
            fs::remove_dir_all(&dir).context("remove packages cache")?;
            println!("Packages cache cleared.");
        } else {
            println!("Package cache already empty.");
        }
        Ok(())
    }

    pub fn clean_repo_cache(&self) -> Result<()> {
        let dir = self.cache_dir();
        if dir.exists() {
            fs::remove_dir_all(&dir).context("remove repos cache")?;
            println!("Repository cache cleared.");
        } else {
            println!("Repo cache already empty.");
        }
        Ok(())
    }

    fn read_repos(&self) -> Result<HashMap<String, String>> {
        let path = self.repos_path();
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let repos: HashMap<String, String> = toml::from_str(&content)
            .with_context(|| "parse repos.toml")?;
        Ok(repos)
    }

    fn write_repos(&self, repos: &HashMap<String, String>) -> Result<()> {
        let path = self.repos_path();
        let content = toml::to_string_pretty(repos)
            .with_context(|| "serialize repos")?;
        fs::write(&path, content)
            .with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    pub fn repo_add(&self, name: &str, url: &str) -> Result<()> {
        let mut repos = self.read_repos()?;
        if repos.contains_key(name) {
            bail!("repository '{}' already exists", name);
        }
        repos.insert(name.to_string(), url.to_string());
        self.write_repos(&repos)?;
        println!("Repository '{}' added.", name);
        Ok(())
    }

    pub fn repo_remove(&self, name: &str) -> Result<()> {
        let mut repos = self.read_repos()?;
        if repos.remove(name).is_none() {
            bail!("repository '{}' not found", name);
        }
        self.write_repos(&repos)?;
        let cache_file = self.cache_dir().join(format!("{}.json", name));
        if cache_file.exists() {
            fs::remove_file(&cache_file)?;
        }
        println!("Repository '{}' removed.", name);
        Ok(())
    }

    pub fn repo_list(&self) -> Result<()> {
        let repos = self.read_repos()?;
        if repos.is_empty() {
            println!("No repositories configured.");
        } else {
            println!("Repositories:");
            for (name, url) in &repos {
                println!("  {}: {}", name, url);
            }
        }
        Ok(())
    }

    pub fn repo_update(&self) -> Result<()> {
        let repos = self.read_repos()?;
        if repos.is_empty() {
            bail!("no repositories configured");
        }
        let cache_dir = self.cache_dir();
        fs::create_dir_all(&cache_dir).context("create cache dir")?;

        for (name, base_url) in &repos {
            println!("Updating '{}' from {}", name, base_url);
            let index_url = format!("{}/index.json", base_url.trim_end_matches('/'));
            let resp = self.http.get(&index_url).send()
                .with_context(|| format!("failed to fetch {}", index_url))?;
            if !resp.status().is_success() {
                bail!("HTTP {} for {}", resp.status(), index_url);
            }
            let bytes = resp.bytes().context("read body")?;
            let _index: RepoIndex = serde_json::from_slice(&bytes)
                .with_context(|| format!("invalid index JSON from {}", index_url))?;
            let cache_file = cache_dir.join(format!("{}.json", name));
            fs::write(&cache_file, bytes)
                .with_context(|| format!("write {}", cache_file.display()))?;
            println!("Repository '{}' updated ({} packages)", name, _index.packages.len());
        }

        // Write the timestamp after a successful update of all repositories.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(self.cache_timestamp_path(), now.to_string())
            .context("write cache timestamp")?;

        Ok(())
    }
    
    pub fn check_updates(&self) -> Result<()> {
        let mut updates = Vec::new();
        for (name,installed) in &self.packages {
            let (_, latest_version, _) = self.resolve_package(name)?;
            if installed.version != latest_version {
                updates.push((name.clone(), installed.version.clone(), latest_version));
            }
        }
        if updates.is_empty() {
            println!("All packages are up to date.");
        } else {
            println!("Packages with updates available:");
            for (name, current, latest) in &updates {
                println!("  {} {} -> {}", name, current, latest);
            }
        }
        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<()> {
        self.ensure_repo_cache_fresh()?;
        let cache_dir = self.cache_dir();
        if !cache_dir.exists() {
            println!("No cache found. Run 'pkgm repo update' first.");
            return Ok(());
        }
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for entry in fs::read_dir(&cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let repo_name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let content = fs::read_to_string(&path)?;
            let index: RepoIndex = serde_json::from_str(&content)
                .with_context(|| format!("parse {}", path.display()))?;
            for (pkg_name, info) in index.packages {
                let name_match = pkg_name.to_lowercase().contains(&query_lower);
                let desc_match = info.description.as_ref()
                    .map(|d| d.to_lowercase().contains(&query_lower))
                    .unwrap_or(false);
                if name_match || desc_match {
                    results.push((
                        repo_name.clone(),
                        pkg_name,
                        info.version,
                        info.description.unwrap_or_default(),
                    ));
                }
            }
        }

        if results.is_empty() {
            println!("No packages match '{}'", query);
        } else {
            println!("Search results for '{}':", query);
            for (repo, name, version, desc) in results {
                println!("  {} {} ({}): {}", repo, name, version, desc);
            }
        }
        Ok(())
    }

    /// Look up a package in repository caches and return its URL, version, and optional checksum.
    pub fn resolve_package(&self, name: &str) -> Result<(String, String, Option<String>)> {
        self.ensure_repo_cache_fresh()?;
        let cache_dir = self.cache_dir();
        if !cache_dir.exists() {
            bail!("repository cache not found. Run 'pkgm repo update'.");
        }
        for entry in fs::read_dir(&cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let content = fs::read_to_string(&path)?;
            let index: RepoIndex = serde_json::from_str(&content)
                .with_context(|| format!("parse {}", path.display()))?;
            if let Some(pkg) = index.packages.get(name) {
                return Ok((pkg.url.clone(), pkg.version.clone(), pkg.checksum.clone()));
            }
        }
        bail!("package '{}' not found in any repository", name);
    }

    pub fn verify(&self, package_name: &str) -> Result<()> {
        let packages = if package_name == "all" {
            self.packages.clone()
        } else {
            let pkg = self.packages.get(package_name)
                .ok_or_else(|| anyhow::anyhow!("package '{}' not installed", package_name))?;
            let mut map = HashMap::new();
            map.insert(package_name.to_string(), pkg.clone());
            map
        };

        for (name, pkg) in &packages {
            println!("verifying package: {}", name);
            let mut all_ok = true;
            for file in &pkg.files {
                let path = Path::new(&self.root).join(file);
                if !path.exists() {
                    eprintln!("  missing file: {}", path.display());
                    all_ok = false;
                    continue;
                }

                if let Some(expected) = &pkg.checksum {
                    let actual = Self::compute_file_sha256(&path)?;
                    if actual == *expected {
                        println!("  OK: {}", file);
                    } else {
                        println!("  FAILED: {} (checksum mismatch)", file);
                        all_ok = false;
                    }
                } else {
                    println!("  OK: {} (no checksum to verify)", file);
                }
            }
            if all_ok {
                println!("{}: OK", name);
            } else {
                println!("{}: FAILED", name);
            }
        }
        Ok(())
    }

    fn compute_file_sha256(path: &Path) -> Result<String> {
        try_digest(path)
            .with_context(|| format!("failed to compute sha256 for {}", path.display()))
    }

    pub fn verify_checksum(&self, path: &Path, expected: &str) -> Result<()> {
        let actual = Self::compute_file_sha256(path)?;
        if actual != expected {
            bail!(
                "checksum mismatch for {}: expected {}, got {}",
                path.display(),
                expected,
                actual
            );
        }
        Ok(())
    }

    pub fn download_package(
        &self,
        url: &str,
        name: &str,
        version: &str,
        expected_checksum: Option<&str>,
    ) -> Result<PathBuf> {
        let cache_dir = self.packages_cache_dir();
        fs::create_dir_all(&cache_dir).context("create package cache dir")?;

        let cached_path = cache_dir.join(format!("{}-{}.pkg.tar.gz", name, version));

        if cached_path.exists() {
            if let Some(checksum) = expected_checksum {
                if self.verify_checksum(&cached_path, checksum).is_ok() {
                    debug!("Using cached package: {}", cached_path.display());
                    return Ok(cached_path);
                } else {
                    warn!("Cached package corrupted, redownloading: {}", cached_path.display());
                    let _ = fs::remove_file(&cached_path);
                }
            } else {
                return Ok(cached_path);
            }
        }

        let temp_file = tempfile::NamedTempFile::new_in(&cache_dir)
            .context("create temp file for download")?;
        let temp_path = temp_file.path().to_path_buf();

        let resp = self.http.get(url).send().context("download failed")?;
        if !resp.status().is_success() {
            bail!("HTTP {} for {}", resp.status(), url);
        }
        let bytes = resp.bytes().context("read response")?;
        fs::write(&temp_path, &bytes).context("write temp file")?;

        if let Some(checksum) = expected_checksum {
            self.verify_checksum(&temp_path, checksum)?;
        }

        fs::rename(&temp_path, &cached_path)
            .with_context(|| format!("move to cache: {}", cached_path.display()))?;

        debug!("Downloaded and cached: {}", cached_path.display());
        Ok(cached_path)
    }
}

fn is_remote_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn mode_to_string(mode: u32) -> String {
    let ft = match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o020000 => 'c',
        0o060000 => 'b',
        _ => '-',
    };
    let fmt = |m: u32| -> [char; 3] {
        [
            if m & 0o4 != 0 { 'r' } else { '-' },
            if m & 0o2 != 0 { 'w' } else { '-' },
            if m & 0o1 != 0 { 'x' } else { '-' },
        ]
    };
    let u = fmt((mode >> 6) & 0o7);
    let g = fmt((mode >> 3) & 0o7);
    let o = fmt(mode & 0o7);
    format!("{}{}{}{}{}{}{}{}{}{}", ft, u[0], u[1], u[2], g[0], g[1], g[2], o[0], o[1], o[2])
}
