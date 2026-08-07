# pkgm — Package Manager

`pkgm` is a lightweight package manager written in Rust. It supports installing, removing, updating packages, managing repositories, searching, verifying integrity, and declarative synchronization.

## Features

- Install/upgrade packages from local files or remote repositories
- Remove packages safely (keeps shared files)
- Repository management (HTTP/HTTPS)
- Search packages by name or description
- Declarative sync with `pkgm.toml` (`apply` and `update`)
- Verify integrity with SHA256 checksums
- Footprint inspection with permissions and owners
- Unpack archives without touching database
- File ownership query with regex

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
nginx = { url = "https://repo.example.com/nginx-1.24.pkg.tar.gz", version = "1.24" }

[profiles]
production = ["nginx"]
```

## Commands

### Repository Management

```bash
pkgm repo add <name> <url>
pkgm repo list
pkgm repo update
pkgm repo remove <name>
```

### Search

```bash
pkgm search <query>
```

### Install

```bash
pkgm install <pkg_name>             # from repository
pkgm install <pkg_archive>          # from local file
pkgm install -u <pkg_archive>       # upgrade
pkgm install -f <pkg_archive>       # force overwrite
```

### Remove

```bash
pkgm remove <pkg_name>
```

### Info

```bash
pkgm info -i                        # list installed
pkgm info -l <pkg_name>             # files of installed
pkgm info -l <pkg_archive>          # files in archive
pkgm info -o "<pattern>"            # owner by regex
pkgm info -f <pkg_archive>          # footprint with permissions
```

### Unpack

```bash
pkgm unpack <pkg_archive>
pkgm unpack <pkg_archive> -d <path> # extract to custom dir
```

### Apply / Update

```bash
pkgm apply                          # sync with manifest (removes extras)
pkgm apply --profile <name>         # use profile
pkgm update                         # update to manifest versions (no removal)
```

### Verify

```bash
pkgm verify all
pkgm verify <pkg_name>
```

## License

GPL-3.0-or-later

**Version:** 0.0.2
```
