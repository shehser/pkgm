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

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

pub struct Database {
    pub db_path: PathBuf,
}

impl Database {
    pub fn open(root_dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let db_dir = root_dir.join("db");
        let db_path = db_dir.join("pkgdb");
        let lock_path = db_dir.join("pkgdb.lock");

        // Ensure parent directories exist
        fs::create_dir_all(&db_dir)?;

        // Lock file access
        let _lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)
            .map_err(|e| format!("Failed to lock DB at {:?}: {}", lock_path, e))?;

        // Initialize database file if missing
        if !db_path.exists() {
            File::create(&db_path)?;
        }

        Ok(Database { db_path })
    }
}
