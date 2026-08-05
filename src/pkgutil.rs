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
//
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;
use tar::{Archive, EntryType};
use users::{get_group_by_gid, get_user_by_uid};

use crate::metadata::{parse_package_name, InstalledPkg, PkgMetadata};

pub const PKG_DIR: &str = "db";
pub const PKG_DB_JSON: &str = "db/pkgdb.json";
pub const META_FILE: &str = "metadata.json";
pub const LDCONFIG: &str = "/usr/bin/ldconfig";
pub const LDCONFIG_CONF: &str = "/etc/ld.so.conf";

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
    pub utilname: String,
    pub packages: Packages,
    pub root: PathBuf,
}

impl PkgUtil {
    pub fn new(name: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self {
            utilname: name.into(),
            packages: HashMap::new(),
            root: root.into(),
        }
    }

    pub fn db_open(&mut self, path: impl AsRef<Path>) -> Result<(), String> {
        let root_path = path.as_ref().to_path_buf();
        let db_path = root_path.join(PKG_DB_JSON);
        self.root = root_path;

        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("could not create db directory {}: {e}", parent.display()))?;
        }

        if !db_path.exists() {
            self.packages = HashMap::new();
            return Ok(());
        }

        let file = File::open(&db_path)
            .map_err(|e| format!("could not open {}: {e}", db_path.display()))?;

        let reader = BufReader::new(file);
        self.packages = serde_json::from_reader(reader)
            .map_err(|e| format!("corrupted database {}: {e}", db_path.display()))?;

        Ok(())
    }

    pub fn db_commit(&self) -> io::Result<()> {
        let dbfilename = self.root.join(PKG_DB_JSON);
        let dbfilename_new = dbfilename.with_extension("tmp");

        if let Some(parent) = dbfilename.parent() {
            fs::create_dir_all(parent)?;
        }

        let file_new = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(&dbfilename_new)?;

        serde_json::to_writer_pretty(&file_new, &self.packages)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        file_new.sync_all()?;
        fs::rename(&dbfilename_new, &dbfilename)?;

        Ok(())
    }

    pub fn db_add_pkg(&mut self, pkg: InstalledPkg) {
        self.packages.insert(pkg.name.clone(), pkg);
    }

    pub fn db_find_pkg(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    pub fn db_rm_pkg(&mut self, name: &str) {
        let Some(pkg) = self.packages.remove(name) else {
            return;
        };
        let mut files = pkg.files;

        for other_pkg in self.packages.values() {
            for file in &other_pkg.files {
                files.remove(file);
            }
        }

        for file in files.iter().rev() {
            remove_path(&self.utilname, &self.root, file);
        }
    }

    pub fn db_find_conflicts(&self, name: &str, files: &BTreeSet<String>) -> BTreeSet<String> {
        let mut conflicts = BTreeSet::new();

        for (pkg_name, pkg_info) in &self.packages {
            if pkg_name != name {
                conflicts.extend(files.intersection(&pkg_info.files).cloned());
            }
        }

        for file_path in files {
            if self.root.join(file_path).exists() {
                conflicts.insert(file_path.clone());
            }
        }

        if let Some(owned_pkg) = self.packages.get(name) {
            for file in &owned_pkg.files {
                conflicts.remove(file);
            }
        }

        conflicts
    }

    pub fn pkg_open(&self, filename: impl AsRef<Path>) -> io::Result<(PkgMetadata, BTreeSet<String>)> {
        let path = filename.as_ref();
        let file = File::open(path)?;
        let mut archive = Archive::new(GzDecoder::new(file));

        let mut meta: Option<PkgMetadata> = None;
        let mut files = BTreeSet::new();

        for entry in archive.entries()? {
            let entry = entry?;
            let path_buf = entry.path()?;
            let path_str = path_buf.to_string_lossy().into_owned();

            if path_str == META_FILE {
                meta = serde_json::from_reader(entry).ok();
            } else {
                files.insert(path_str);
            }
        }

        let meta = match meta {
            Some(m) => m,
            None => {
                let file_str = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
                let (name, version) = parse_package_name(file_str)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

                PkgMetadata {
                    name,
                    version,
                    description: Some("Auto-generated metadata".to_string()),
                }
            }
        };

        Ok((meta, files))
    }

    pub fn pkg_install(&self, filename: impl AsRef<Path>) -> Result<(), String> {
        let pkg_path = filename.as_ref();
        let file = File::open(pkg_path)
            .map_err(|e| format!("could not open {}: {e}", pkg_path.display()))?;

        let mut archive = Archive::new(GzDecoder::new(file));
        let absroot = fs::canonicalize(&self.root)
            .map_err(|e| format!("could not resolve root path {}: {e}", self.root.display()))?;

        let entries = archive
            .entries()
            .map_err(|e| format!("could not read archive {}: {e}", pkg_path.display()))?;

        for entry_res in entries {
            let mut entry = match entry_res {
                Ok(e) => e,
                Err(err) => {
                    eprintln!("{}: error reading archive entry: {err}", self.utilname);
                    continue;
                }
            };

            let entry_path = match entry.path() {
                Ok(p) => p.to_path_buf(),
                Err(e) => {
                    eprintln!("{}: invalid path in archive: {e}", self.utilname);
                    continue;
                }
            };

            if entry_path.to_string_lossy() == META_FILE {
                continue;
            }

            let real_filename = absroot.join(&entry_path);

            if let Some(parent) = real_filename.parent() {
                let _ = fs::create_dir_all(parent);
            }

            entry.set_unpack_xattrs(true);
            entry.set_preserve_permissions(true);
            entry.set_preserve_mtime(true);

            if let Err(err) = entry.unpack(&real_filename) {
                eprintln!("{}: could not install {}: {err}", self.utilname, entry_path.display());
            }

            // Unpack nested tar.gz if present inside package
            let path_str = entry_path.to_string_lossy();
            if path_str.ends_with(".tar.gz") || path_str.ends_with(".tgz") {
                if let Ok(nested_file) = File::open(&real_filename) {
                    let mut nested_archive = Archive::new(GzDecoder::new(nested_file));
                    let _ = nested_archive.unpack(&absroot);
                    let _ = fs::remove_file(&real_filename);
                }
            }
        }

        Ok(())
    }

    pub fn pkg_unpack(&self, filename: impl AsRef<Path>, target_dir: impl AsRef<Path>) -> Result<(), String> {
        let pkg_path = filename.as_ref();
        let target = target_dir.as_ref();

        let file = File::open(pkg_path)
            .map_err(|e| format!("could not open {}: {e}", pkg_path.display()))?;

        let mut archive = Archive::new(GzDecoder::new(file));

        if !target.exists() {
            fs::create_dir_all(target)
                .map_err(|e| format!("could not create target directory {}: {e}", target.display()))?;
        }

        let entries = archive
            .entries()
            .map_err(|e| format!("could not read archive {}: {e}", pkg_path.display()))?;

        for entry_res in entries {
            let mut entry = match entry_res {
                Ok(e) => e,
                Err(err) => {
                    eprintln!("{}: error reading entry: {err}", self.utilname);
                    continue;
                }
            };

            let entry_path = match entry.path() {
                Ok(p) => p.to_path_buf(),
                Err(e) => {
                    eprintln!("{}: invalid path in archive: {e}", self.utilname);
                    continue;
                }
            };

            if entry_path.to_string_lossy() == META_FILE {
                continue;
            }

            let unpack_location = target.join(&entry_path);

            if let Some(parent) = unpack_location.parent() {
                let _ = fs::create_dir_all(parent);
            }

            entry.set_unpack_xattrs(true);
            entry.set_preserve_permissions(true);
            entry.set_preserve_mtime(true);

            if let Err(err) = entry.unpack(&unpack_location) {
                eprintln!("{}: could not unpack {}: {err}", self.utilname, entry_path.display());
            }
        }

        Ok(())
    }

    pub fn pkg_footprint(&self, filename: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
        let file = File::open(filename)?;
        let mut archive = Archive::new(GzDecoder::new(file));

        let mut entries_data = Vec::new();
        let mut hardlink_target_modes = HashMap::new();

        for entry_res in archive.entries()? {
            let entry = entry_res?;
            let header = entry.header();

            let path = entry.path()?.to_string_lossy().into_owned();
            if path == META_FILE {
                continue;
            }

            let mode = header.mode()?;
            let entry_type = header.entry_type();
            let link_name = header.link_name()?.map(|p| p.to_string_lossy().into_owned());

            if !entry_type.is_hard_link() {
                hardlink_target_modes.insert(path.clone(), mode);
            }

            entries_data.push(FootprintEntry {
                path,
                mode,
                uid: header.uid()?,
                gid: header.gid()?,
                size: header.size()?,
                entry_type,
                link_name,
            });
        }

        for item in &entries_data {
            if item.entry_type.is_symlink() {
                print!("lrwxrwxrwx");
            } else if item.entry_type.is_hard_link() {
                let mode = item
                    .link_name
                    .as_ref()
                    .and_then(|target| hardlink_target_modes.get(target))
                    .copied()
                    .unwrap_or(item.mode);
                print!("{}", mode_to_string(mode));
            } else {
                print!("{}", mode_to_string(item.mode));
            }

            print!("\t");

            if let Some(user) = get_user_by_uid(item.uid as u32) {
                print!("{}", user.name().to_string_lossy());
            } else {
                print!("{}", item.uid);
            }

            print!("/");

            if let Some(group) = get_group_by_gid(item.gid as u32) {
                print!("{}", group.name().to_string_lossy());
            } else {
                print!("{}", item.gid);
            }

            print!("\t{}", item.path);

            if item.entry_type.is_symlink() {
                if let Some(ref target) = item.link_name {
                    print!(" -> {target}");
                }
            } else if item.entry_type.is_file() && item.size == 0 {
                print!(" (EMPTY)");
            }

            println!();
        }

        Ok(())
    }

    pub fn ldconfig(&self) -> io::Result<()> {
        let ldconfig_conf = self.root.join(LDCONFIG_CONF);
        if ldconfig_conf.exists() {
            let etc_dir = self.root.join("etc");
            if !etc_dir.exists() {
                let _ = fs::create_dir_all(&etc_dir);
            }

            let status = Command::new(LDCONFIG)
                .arg("-r")
                .arg(&self.root)
                .status()?;

            if !status.success() {
                eprintln!("{}: {LDCONFIG} exited with status {status}", self.utilname);
            }
        }
        Ok(())
    }
}

fn remove_path(utilname: &str, root: &Path, file: &str) {
    let trimmed = file.trim_start_matches(['.', '/']);
    if trimmed.is_empty() {
        return;
    }

    let path = root.join(file);
    if path.exists() {
        let res = if path.is_dir() {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        };

        if let Err(err) = res {
            let raw_err = err.raw_os_error();
            if raw_err != Some(libc::ENOTEMPTY) && raw_err != Some(libc::EINVAL) {
                eprintln!("{utilname}: could not remove {}: {err}", path.display());
            }
        }
    }
}

fn mode_to_string(mode: u32) -> String {
    let file_type = match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o020000 => 'c',
        0o060000 => 'b',
        _ => '-',
    };

    let perm = |m: u32| {
        [
            if m & 0o4 != 0 { 'r' } else { '-' },
            if m & 0o2 != 0 { 'w' } else { '-' },
            if m & 0o1 != 0 { 'x' } else { '-' },
        ]
    };

    let u = perm((mode >> 6) & 0o7);
    let g = perm((mode >> 3) & 0o7);
    let o = perm(mode & 0o7);

    format!(
        "{file_type}{}{}{}{}{}{}{}{}{}",
        u[0], u[1], u[2], g[0], g[1], g[2], o[0], o[1], o[2]
    )
}
