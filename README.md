# yconn

SSH connection manager CLI — manage named SSH connections across teams and projects with a layered, git-friendly config system.

Connections live in YAML files at three layers (project, user, system). Higher layers override lower ones on name collision. When a Docker image is configured, yconn re-invokes itself inside the container so SSH keys can be pre-baked into an image rather than distributed to individual machines.

## Documentation

- [Configuration reference](docs/configuration.md) — layer system, all config fields, Docker block, session file, credential policy
- [Examples](docs/examples.md) — copy-paste-ready scenarios: project-layer only, project + user layers, Docker-enabled setup, multi-group setup

## Installation

### Arch Linux (AUR)
```
yay -S yconn
```

### Debian / Ubuntu
Download the latest `.deb` from the [releases page](https://github.com/mans/yconn/releases) and install:
```
sudo dpkg -i yconn_0.1.0_amd64.deb
```

### From source
```
cargo build --release --target x86_64-unknown-linux-musl
```

The compiled binary is placed at `target/x86_64-unknown-linux-musl/release/yconn`.

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
