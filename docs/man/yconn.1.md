% YCONN(1) Version 0.1.0 | User Commands

# NAME

yconn - SSH connection manager CLI

# SYNOPSIS

**yconn** [*OPTIONS*] *COMMAND* [*ARGS*]

# DESCRIPTION

**yconn** is a CLI tool for managing SSH connections across teams and projects.
It uses a layered config system inspired by git and ssh, supports key-based and
password-based auth, and is designed to be shareable in DevOps environments
without ever exposing credentials.

Connections are defined in `connections.yaml` files and loaded from up to three
layers:

- **project** — `connections.yaml` (or `.connections.yaml` / `.yconn/connections.yaml`),
  discovered by walking up from the current directory (like git); lives in version
  control
- **user** — `~/.config/yconn/connections.yaml`; private to the local user
- **system** — `/etc/yconn/connections.yaml`; org-wide defaults managed by sysadmins

Higher-priority layers win on name collision. A layer that has no `connections.yaml`
is silently skipped.

Connection names may contain glob wildcards (`*`, `?`). When a wildcard pattern
matches, the SSH hostname is resolved from the entry's `host:` field: if it contains
`${name}`, only that token is replaced with the input (e.g. `${name}.corp.com` →
`web-prod-01.corp.com`); otherwise the entire host field is replaced by the input.
Exact name matches always win over wildcard patterns. If two different patterns both
match the same input, yconn exits with a conflict error.

When a **docker** block is present in the project or system config, yconn
re-invokes itself inside a container before doing anything else. This allows SSH
keys to be pre-baked into an image rather than distributed to developer machines.

# COMMANDS

**list**
: List all connections across all active config layers. Connections from
  higher-priority layers that shadow lower-priority ones are shown once.
  Use **--all** to also show shadowed entries.
  Use **--group** *NAME* to filter to connections whose `group:` field equals *NAME*.

**connect** *NAME*
: Connect to the named host by invoking SSH. Replaces the current process so
  terminal behavior (TTY, signals) works correctly. *NAME* is matched first as
  an exact connection name, then against wildcard patterns. When a wildcard
  pattern matches, the SSH hostname is derived from the entry's `host:` field:
  if it contains `${name}`, that token is replaced with the input; otherwise
  the input string is used directly as the hostname.

**show** *NAME*
: Print the resolved config for a connection. Credentials (key paths) are shown
  as configured; passwords are never stored and are not shown.

**add**
: Interactive wizard that prompts for connection details and writes the entry to
  a chosen layer. Use **--layer** to target a specific layer.

**edit** *NAME*
: Open the source config file that defines *NAME* in **$EDITOR**.
  Use **--layer** to target a specific layer.

**remove** *NAME*
: Remove a connection. Prompts for which layer to target if the name exists in
  more than one layer. Use **--layer** to target a specific layer.

**init**
: Scaffold a `connections.yaml` in the current directory. The **--location** flag
  controls where the file is placed:

  - **yconn** (default) — creates `.yconn/connections.yaml`; recommended for
    git-tracked project configs
  - **dotfile** — creates `.connections.yaml` in the current directory
  - **plain** — creates `connections.yaml` in the current directory (may conflict
    with other tools)

  Fails with a clear error if the target file already exists.

**config**
: Show which config files are active, their paths, connection counts, and Docker
  bootstrap status.

**group list**
: Show all unique group values found across connection entries in all loaded
  layers, and which layers contain connections with each group tag.

**group use** *NAME*
: Set *NAME* as the active group. Writes to `~/.config/yconn/session.yml`.
  A warning is emitted if no connections with that group value exist in any layer,
  but the group is still set.

**group clear**
: Remove `active_group` from `session.yml`, reverting to the default (no group
  filter — all connections shown).

**group current**
: Print the active group name and the resolved config file paths for each layer,
  indicating which files were found.

# OPTIONS

**--all**
: Include shadowed entries in the output of **yconn list**.

**--verbose**
: Print config loading decisions, merge resolution, and the full Docker
  invocation command before it is executed.

**--help**
: Print help and exit.

**--version**
: Print version and exit.

The **--layer** *system*|*user*|*project* flag applies only to **add**, **edit**,
and **remove** — it is a per-subcommand flag, not a global option.

# CONFIGURATION

## File format

Each config file is a YAML document with an optional **docker** block and a
**connections** map:

```yaml
version: 1

docker:
  image: ghcr.io/myorg/yconn-keys:latest
  pull: always        # "always" | "missing" (default) | "never"
  args:
    - "--network=host"

connections:
  prod-web:
    host: 10.0.1.50
    user: deploy
    port: 22          # optional, defaults to 22
    auth: key         # "key" | "password"
    key: ~/.ssh/prod_deploy_key
    description: "Primary production web server"
    group: work       # optional inline group tag
    link: https://wiki.internal/servers/prod-web

  web-*:
    host: "${name}.corp.com"   # ${name} is replaced with the matched input
    user: deploy
    auth: key
    key: ~/.ssh/web_key
    description: "Any web server matching web-*"
```

## Wildcard patterns

Connection names may use `*` (any sequence) and `?` (any single character).
When a wildcard pattern matches the input to **yconn connect**, the `host:`
field is resolved: if it contains `${name}`, only that token is replaced with
the matched input (e.g. `${name}.corp.com` → `web-prod-01.corp.com`);
otherwise the entire host field is replaced by the input. Two different
patterns matching the same input is a conflict and causes yconn to exit
non-zero with a clear error.

## Session file

The active group is persisted to `~/.config/yconn/session.yml`:

```yaml
active_group: work
```

An absent or empty session file is valid — no group filter is applied and all
connections are shown. Unknown keys are ignored for forward compatibility.

## Docker bootstrap

When **docker.image** is set and yconn is not already running inside a container,
it constructs a `docker run` command that mounts the yconn binary, all config
layers, and the current working directory (read-only, to enable the upward config
walk), then re-invokes itself with the original arguments inside the container.

Container detection: yconn considers itself inside a container when `/.dockerenv`
exists or the environment variable `CONN_IN_DOCKER=1` is set.

The **docker** block is trusted only from the **project** and **system** layers.
If present in the user layer, it is ignored with a warning.

# EXAMPLES

List connections from all layers:

```
yconn list
```

Filter connections by group:

```
yconn list --group work
```

Connect to a host:

```
yconn connect prod-web
```

Connect using a wildcard pattern (input becomes the SSH hostname):

```
yconn connect web-prod-01
```

Show all details for a connection (including shadowed):

```
yconn list --all
yconn show bastion
```

Add a connection interactively to the project layer:

```
yconn add --layer project
```

Edit the config file that defines a connection:

```
yconn edit staging-db
```

Switch active group and verify:

```
yconn group use work
yconn group current
```

Show active config files and Docker status:

```
yconn config
```

Scaffold a project config at different locations:

```
yconn init                        # creates .yconn/connections.yaml (default)
yconn init --location dotfile     # creates .connections.yaml
yconn init --location plain       # creates connections.yaml
```

# FILES

`~/.config/yconn/session.yml`
: User session state — active group.

`~/.config/yconn/connections.yaml`
: User-level connection config. May reference local key paths.

`/etc/yconn/connections.yaml`
: System-level connection config. Must not contain credentials.

`.yconn/connections.yaml`
: Project-level connection config (default location). Lives in version control.
  Must not contain credentials.

`.connections.yaml`
: Project-level connection config (dotfile convention). Found by the upward walk.

`connections.yaml`
: Project-level connection config (plain convention). Found by the upward walk.
  May conflict with other tools that use the same filename.

# ENVIRONMENT

`CONN_IN_DOCKER`
: Set to `1` by yconn when it bootstraps into Docker. Prevents infinite
  re-invocation.

`EDITOR`
: Editor used by **yconn edit**.

# SECURITY

Passwords are never stored in any config file and are never passed as CLI
arguments or environment variables — SSH prompts natively. Key passphrases are
delegated entirely to **ssh-agent**(1).

Config files in git-trackable locations (`.yconn/`) are scanned for credential
fields on load; a warning is emitted if any are found. All warnings are
non-blocking.

# AUTHOR

Mans

# SEE ALSO

**ssh**(1), **ssh_config**(5), **ssh-agent**(1)
