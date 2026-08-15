# pkgm — Package Manager

`pkgm` is a lightweight package manager written in Rust. It supports installing, removing, updating packages, managing repositories, searching, verifying integrity, declarative synchronization, **dry‑run** previews, **checksum‑protected** downloads with caching, **parallel downloads**, and **dependency resolution** with version constraints.

## Features

- Install/upgrade **multiple packages** in one command
- Install from local files or remote repositories (HTTP/HTTPS)
- Safe removal – keeps files shared with other packages
- Repository management with cached indexes and **automatic refresh** (TTL 24h, can be disabled)
- Search packages by name or description
- Declarative sync with `pkgm.toml` (`apply` and `update`)
- Integrity verification with SHA‑256 checksums (supports `checksum` in manifest)
- **Package caching** – downloaded packages are stored and reused, with automatic corruption detection
- **Dry‑run mode** (`-n`) – preview all operations without making changes
- **Global flag `--no-auto-update`** – disable automatic repository updates
- **Parallel downloads** – multiple packages are downloaded concurrently (via threads)
- **Dependency resolution** – handles version constraints (`>=`, `<=`, `~`, `*`, exact versions), detects cycles and version conflicts
- Conflict handling – `--force` to overwrite, otherwise safe abort
- Footprint inspection – show detailed permissions, owners, symlinks, and hardlinks
- Unpack archives without touching the database
- File ownership query with regular expressions
- **Cache cleaning** – clear downloaded packages and repository indexes
- **JSON output** for automation (`--json` flag)
- **Configuration validation** (`check-config` command)
- **Check updates** (`checkupdates` command)
- **Logging** support via `env_logger` (`RUST_LOG=debug`)

## Installation

### From source

```bash
git clone https://github.com/shehser/pkgm.git
cd pkgm
cargo build --release
sudo cp target/release/pkgm /usr/local/bin/
```

### Via Cargo

```bash
cargo install --path .
```

### Direct binary download

Download the latest release from [GitHub Releases](https://github.com/shehser/pkgm/releases) and place it in your `PATH`.

## Configuration

### Repositories (`repos.toml`)

Located in the root directory (or `--root`). Example:

```toml
main = "https://repo.example.com"
custom = "http://myrepo.local"
```

### Manifest (`pkgm.toml`)

Declarative configuration for `apply` and `update`. Supports **dependencies**:

```toml
[packages]
nginx = {
  url = "https://repo.example.com/nginx-1.24.pkg.tar.gz",
  version = "1.24",
  checksum = "sha256...",
  dependencies = { openssl = ">=1.1", libc = "2.31" }
}

[profiles]
production = ["nginx"]
```

- `dependencies` is a map of package name → version constraint. Supported constraints:
  - Exact: `"1.2.3"`
  - Greater/equal: `">=1.2.3"`
  - Less/equal: `"<=2.0"`
  - Compatible: `"~1.2.3"` (allows patch updates)
  - Wildcard: `"1.2.*"` or `"1.*"`
  - Compound constraints are not supported (use multiple dependencies instead).

> **Note:** The `checksum` field is optional but recommended.

## Global Flags

- `-n, --dry-run` – show what would be done without making any changes
- `--no-auto-update` – disable automatic repository cache updates (useful in offline or CI environments)
- `--root <PATH>` – set an alternative root directory (default: current directory)
- `--json` – output results in JSON format (for commands: `info -i`, `search`, `checkupdates`, `repo list`)

## Commands

### Repository Management

```bash
pkgm repo add <name> <url>
pkgm repo list
pkgm repo update
pkgm repo remove <name>
```

> **Automatic updates:** `search`, `install`, and `checkupdates` will automatically run `repo update` if the cache is older than 24 hours, unless `--no-auto-update` is given.

### Search

```bash
pkgm search <query>
```

### Install

Install one or multiple packages (with dependency resolution):

```bash
pkgm install <pkg_name1> <pkg_name2> ...          # from repositories
pkgm install <pkg_archive>                        # from local file
pkgm install -u <pkg_name1> <pkg_name2>           # upgrade multiple packages
pkgm install -f <pkg_name>                        # force overwrite on conflicts
```

### Remove

```bash
pkgm remove <pkg_name>
```

### Info

```bash
pkgm info -i                        # list installed packages
pkgm info -l <pkg_name>             # files of installed package
pkgm info -l <pkg_archive>          # files inside an archive
pkgm info -o "<pattern>"            # owner by regex (e.g., "/usr/bin/.*")
pkgm info -f <pkg_archive>          # detailed footprint (permissions, owners, symlinks)
```

### Unpack

```bash
pkgm unpack <pkg_archive>           # unpack to current directory
pkgm unpack <pkg_archive> -d <path> # extract to custom directory
```

### Apply / Update (declarative sync)

```bash
pkgm apply                          # synchronize system with manifest (removes obsolete packages)
pkgm apply --profile <name>         # apply only the specified profile
pkgm update                         # update to manifest versions (without removing extras)
pkgm update --profile <name>        # update only the profile
```

### Verify

```bash
pkgm verify all                     # check all installed packages
pkgm verify <pkg_name>              # check a specific package
```

### Check Updates

```bash
pkgm checkupdates
```

### Clean

Clear caches for downloaded packages and/or repository indexes:

```bash
pkgm clean --packages               # remove downloaded package cache
pkgm clean --repos                  # remove repository index cache
pkgm clean                          # clean both caches
```

### Check Configuration

```bash
pkgm check-config                   # validate pkgm.toml syntax, URL availability, and checksums
pkgm check-config -c custom.toml    # with custom file
```

## Examples

```bash
# Install two packages from repositories (resolves dependencies automatically)
pkgm install nginx postgresql

# Upgrade specific packages
pkgm install -u nginx redis

# Dry‑run an upgrade (see what would happen)
pkgm -n install -u nginx

# Disable auto‑update and install a package
pkgm --no-auto-update install curl

# Clean up disk space used by caches
pkgm clean --packages

# Check for available updates
pkgm checkupdates

# Validate manifest
pkgm check-config
```

## Logging

Set `RUST_LOG=debug` to enable detailed logging:

```bash
RUST_LOG=debug pkgm install nginx
```

## License

**GPL-2.0**  
Copyright (C) 2026 Yersultan Muapyqov
