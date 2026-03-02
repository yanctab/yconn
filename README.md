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

Two-step (download first, then install):
```bash
VERSION=1.2.0
wget "https://github.com/yanctab/yconn/releases/download/v${VERSION}/yconn-${VERSION}-1-x86_64.pkg.tar.zst"
sudo pacman -U "yconn-${VERSION}-1-x86_64.pkg.tar.zst"
```

Or with curl:
```bash
VERSION=1.2.0
curl -LO "https://github.com/yanctab/yconn/releases/download/v${VERSION}/yconn-${VERSION}-1-x86_64.pkg.tar.zst"
sudo pacman -U "yconn-${VERSION}-1-x86_64.pkg.tar.zst"
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

Global flags: `--layer system|user|project`, `--all`, `--verbose`, `--no-color`

## Development

```
make build    # compile (musl static binary)
make test     # run tests
make lint     # cargo fmt --check + clippy
make docs     # generate docs/man/yconn.1 via pandoc
make release  # tag and trigger release pipeline
```

See `man yconn` for the full command reference after `make docs`.
