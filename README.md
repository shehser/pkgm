# pkgm — Package Manager in Rust

`pkgm` is a lightweight Unix-like package manager written in Rust. It manages compressed software archives (`.pkg.tar.gz`), maintains a local JSON database of installed files, and handles file conflicts and package updates.

---

## Features

- **Install / Upgrade:** Install `.pkg.tar.gz` packages or upgrade existing ones.
- **Conflict Detection:** Automatic file conflict detection prior to installation.
- **Footprint Inspection:** Inspect permissions, users, and contents of package archives without extracting.
- **Unpack Utility:** Extract archive contents into any target directory without registering in DB.
- **File Ownership Query:** Find which installed package owns a specific file using regex matching.

---

## Project Structure

```text
pkgutil/
├── Cargo.toml
└── src/
    ├── main.rs       # CLI interface & argument parsing (clap)
    ├── metadata.rs   # Package metadata and name parsing
    ├── pkgadd.rs     # Package installation logic
    ├── pkginfo.rs    # Database & archive query logic
    ├── pkgrm.rs      # Package removal logic
    └── pkgutil.rs    # Core utilities (archive unpacking, DB operations)

## Building & Installation

### Prerequisites
- [Rust & Cargo](https://www.rust-lang.org/) (latest stable version)

### Build from Source
1. Clone the repository:
   ```bash
   git clone [https://github.com/your-username/pkgm.git](https://github.com/your-username/pkgm.git)
   cd pkgm
   ```

2. Build and install binary into your `~/.cargo/bin`:
   ```bash
   cargo install --path .
   ```

3. Or build release executable without installing:
   ```bash
   cargo build --release
   # Binary will be located at ./target/release/pkgm
   ```


## Usage Examples

### 1. Installation & Upgrade (`install`)
Install a package into a custom root directory (`<path>`):
```bash
pkgm -r <path> install <pkg_archive>
```

Upgrade an already installed package:
```bash
pkgm -r <path> install -u <pkg_archive>
```

Force installation (overwrite conflicting files):
```bash
pkgm -r <path> install -f <pkg_archive>
```

### 2. Database & Package Inspection (`info`)
List all installed packages:
```bash
pkgm -r <path> info -i
```

List files of an installed package:
```bash
pkgm -r <path> info -l <pkg_name>
```

List files inside an uninstalled package archive:
```bash
pkgm info -l <pkg_archive>
```

Find package owning a specific file pattern:
```bash
pkgm -r <path> info -o "<pattern>"
```

Print archive footprint details:
```bash
pkgm info -f <pkg_archive>
```

### 3. Unpack Archive (`unpack`)
Unpack archive without database registration:
```bash
pkgm unpack <pkg_archive> -d <path>
```

### 4. Package Removal (`remove`)
Remove an installed package:
```bash
pkgm -r <path> remove <pkg_name>
```
