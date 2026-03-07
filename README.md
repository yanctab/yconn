# yconn

SSH connection manager CLI — manage named SSH connections across teams and projects with a layered, git-friendly config system.

Connections live in YAML files at three layers (project, user, system). Higher layers override lower ones on name collision. When a Docker image is configured, yconn re-invokes itself inside the container so SSH keys can be pre-baked into an image rather than distributed to individual machines.

## Documentation

- [Configuration reference](docs/configuration.md) — layer system, all config fields, Docker block, session file, credential policy
- [Examples](docs/examples.md) — copy-paste-ready scenarios: project-layer only, project + user layers, Docker-enabled setup, multi-group setup

## Installation

### Arch Linux

One-step (pacman fetches and installs directly from the release URL):
```bash
VERSION=1.2.0
sudo pacman -U "https://github.com/yanctab/yconn/releases/download/v${VERSION}/yconn-${VERSION}-1-x86_64.pkg.tar.zst"
```

### Debian / Ubuntu

```bash
VERSION=1.2.0
wget "https://github.com/yanctab/yconn/releases/download/v${VERSION}/yconn_${VERSION}_amd64.deb"
sudo apt install "./yconn_${VERSION}_amd64.deb"
```

### From source

```bash
git clone https://github.com/yanctab/yconn.git
cd yconn
make build
sudo make install          # installs to /usr/local/bin/yconn
# or: make install PREFIX=~/.local   # installs to ~/.local/bin/yconn (no sudo needed)
```

The man page is also installed if you have run `make docs` first.

### Cargo

```bash
cargo install yconn
```

## Quick start

```bash
# Scaffold a project config in the current directory
yconn init

# Add a connection interactively
yconn add

# List all connections
yconn list

# Connect
yconn connect prod-web

# Switch to a named group
yconn group use work
```

## Commands

| Command | Description |
|---|---|
| `yconn list` | List all connections across all layers |
| `yconn connect <name>` | Connect to a named host |
| `yconn show <name>` | Show resolved config for a connection (no secrets printed) |
| `yconn add` | Interactive wizard to add a connection to a chosen layer |
| `yconn edit <name>` | Open the connection's source config file in `$EDITOR` |
| `yconn remove <name>` | Remove a connection (prompts for layer if ambiguous) |
| `yconn init` | Scaffold a `<group>.yaml` in `.yconn/` in the current directory |
| `yconn config` | Show active config files, their paths, and Docker status |
| `yconn group list` | Show all groups found across all layers |
| `yconn group use <name>` | Set the active group |
| `yconn group clear` | Revert to the default group (`connections`) |
| `yconn group current` | Print the active group name and resolved config file paths |
| `yconn ssh-config` | Write Host blocks to `~/.ssh/yconn-connections` and update `~/.ssh/config` |
| `yconn user show` | List all user key/value entries across all layers |
| `yconn user add` | Interactive wizard to add a user entry to a chosen layer |
| `yconn user edit <key>` | Open the source config file for a user entry in `$EDITOR` |

Global flags: `--all`, `--verbose`

Per-subcommand flags: `--layer system|user|project` (for `add`, `edit`, `remove`, `user add`, `user edit`)

## Development

```bash
# First-time setup (installs Rust components and cargo-llvm-cov)
make setup
# System packages also required:
sudo apt-get install -y musl-tools pandoc zstd

make build    # compile (musl static binary)
make test     # run tests
make lint     # cargo fmt --check + clippy
make docs     # generate docs/man/yconn.1 via pandoc
make release  # tag and trigger release pipeline
```

See `man yconn` for the full command reference after `make docs`.
