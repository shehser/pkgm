# pkgm — Package Manager

`pkgm` is a lightweight package manager written in Rust. It supports installing, removing, updating packages, managing repositories, searching, verifying integrity, declarative synchronization, **dry‑run** previews, **checksum‑protected** downloads with caching, and **automatic repository updates** with an optional disable flag.

## Features

- Install/upgrade **multiple packages** in one command  
- Install from local files or remote repositories (HTTP/HTTPS)  
- Safe removal – keeps files shared with other packages  
- Repository management with cached indexes and **automatic refresh** (TTL 24h)  
- Search packages by name or description  
- Declarative sync with `pkgm.toml` (`apply` and `update`)  
- Integrity verification with SHA‑256 checksums (supports `checksum` in manifest)  
- **Package caching** – downloaded packages are stored and reused, with automatic corruption detection  
- **Dry‑run mode** (`-n`) – preview all operations without making changes  
- **Global flag `--no-auto-update`** – disable automatic repository updates  
- Conflict handling – `--force` to overwrite, otherwise safe abort  
- Footprint inspection – show detailed permissions, owners, symlinks, and hardlinks  
- Unpack archives without touching the database  
- File ownership query with regular expressions  
- **Cache cleaning** – clear downloaded packages and repository indexes  

## Installation

```bash
git clone https://github.com/shehser/pkgm.git
cd pkgm
cargo build --release
sudo cp target/release/pkgm /usr/local/bin/
```

Or install via Cargo:

```bash
cargo install --path .
```

## Configuration

### Repositories (`repos.toml`)

```toml
main = "https://repo.example.com"
custom = "http://myrepo.local"
```

### Manifest (`pkgm.toml`)

```toml
[packages]
nginx = { url = "https://repo.example.com/nginx-1.24.pkg.tar.gz", version = "1.24", checksum = "sha256..." }

[profiles]
production = ["nginx"]
```

> **Note:** The `checksum` field is optional but recommended.

## Global Flags

- `-n, --dry-run` – show what would be done without making any changes  
- `--no-auto-update` – disable automatic repository cache updates (useful in offline or CI environments)  
- `--root <PATH>` – set an alternative root directory (default: current directory)

## Commands

### Repository Management

```bash
pkgm repo add <name> <url>
pkgm repo list
pkgm repo update
pkgm repo remove <name>
```

> **Automatic updates:** `search` and `install` will automatically run `repo update` if the cache is older than 24 hours, unless `--no-auto-update` is given.

### Search

```bash
pkgm search <query>
```

### Install

Install one or multiple packages:

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

### Clean

Clear caches for downloaded packages and/or repository indexes:

```bash
pkgm clean --packages               # remove downloaded package cache
pkgm clean --repos                  # remove repository index cache
pkgm clean                          # clean both caches
```

## Examples

```bash
# Install two packages from repositories
pkgm install nginx postgresql

# Upgrade specific packages
pkgm install -u nginx redis

# Dry‑run an upgrade (see what would happen)
pkgm -n install -u nginx

# Disable auto‑update and install a package
pkgm --no-auto-update install curl

# Clean up disk space used by caches
pkgm clean --packages
```

## License

**GPL-2.0**  
Copyright (C) 2026 Yersultan Muapyqov
