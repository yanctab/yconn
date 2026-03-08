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

Connection names may use glob wildcards (`*`, `?`) or a numeric range suffix
(`[N..M]`, matching any integer in `[N, M]` after the literal prefix). When a
pattern matches, the SSH hostname is resolved from the entry's `host:` field: if it
contains `${name}`, only that token is replaced with the input (e.g.
`${name}.corp.com` → `web-prod-01.corp.com`); otherwise the entire host field is
replaced by the input. Exact name matches always win over patterns. If two different
patterns both match the same input, yconn exits with a conflict error.

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
  an exact connection name, then against glob and range patterns. When a
  pattern matches, the SSH hostname is derived from the entry's `host:` field:
  if it contains `${name}`, that token is replaced with the input; otherwise
  the input string is used directly as the hostname.

**connections show** *NAME*
: Print the resolved config for a connection. Credentials (key paths) are shown
  as configured; passwords are never stored and are not shown.

**connections show --dump**
: Print the fully merged `connections:` and `users:` maps as valid YAML to stdout.
  Active entries only — no shadowed rows. Mutually exclusive with providing a
  connection name.

**connections add**
: Interactive wizard that prompts for connection details and writes the entry to
  a chosen layer. Use **--layer** to target a specific layer.

**connections edit** *NAME*
: Open the source config file that defines *NAME* in **$EDITOR**.
  Use **--layer** to target a specific layer.

**connections remove** *NAME*
: Remove a connection. Prompts for which layer to target if the name exists in
  more than one layer. Use **--layer** to target a specific layer.

**connections init**
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

**ssh-config**
: Read all active connections (respecting the active group lock and layer merge,
  identical to **yconn list**) and write one `Host` block per non-wildcard,
  non-range connection to `~/.ssh/yconn-connections`. Updates `~/.ssh/config`
  idempotently by prepending `Include ~/.ssh/yconn-connections` if the line is
  not already present. Wildcard and range-pattern connection names are skipped
  with a comment. A summary line is printed showing the count of Host blocks
  written and the output file path.

  Flags:

  - **--dry-run** — print the generated file content and the `~/.ssh/config`
    change to stdout without writing any files.
  - **--user** *KEY:VALUE* — override or add a `users:` map entry for this
    invocation (repeatable). Mutually exclusive with **--skip-user**.
  - **--skip-user** — omit `User` lines from all generated Host blocks.
    Mutually exclusive with **--user**.

**users show**
: List all user key/value entries across all config layers. Displays KEY, VALUE,
  and SOURCE columns. Shadowed entries (overridden by a higher-priority layer)
  are shown dimmed with a `[shadowed]` tag. Use this to inspect what `${key}`
  templates will expand to.

**users add**
: Interactive wizard that prompts for a key and value and writes the entry to the
  `users:` section of the target layer's config file. Defaults to the user layer
  (`~/.config/yconn/connections.yaml`). Use **--layer** to target a specific layer.

**users edit** *KEY*
: Open the source config file that contains the named user entry in **$EDITOR**.
  Defaults to the active (highest-priority) entry. Use **--layer** to target a
  specific layer.

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

The **--layer** *system*|*user*|*project* flag applies only to **connections add**,
**connections edit**, **connections remove**, **users add**, and **users edit** — it is
a per-subcommand flag, not a global option.

# CONFIGURATION

## File format

Each config file is a YAML document with an optional **docker** block, an
optional **users** map, and a **connections** map:

```yaml
version: 1

docker:
  image: ghcr.io/myorg/yconn-keys:latest
  pull: always        # "always" | "missing" (default) | "never"
  args:
    - "--network=host"

users:
  testuser: "testusername"   # referenced as ${testuser} in connection user fields

connections:
  prod-web:
    host: 10.0.1.50
    user: ${testuser}   # expands to "testusername" at connect time
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

  "app[1..20]":
    host: "${name}.internal"   # app5 → app5.internal
    user: ops
    auth: key
    key: ~/.ssh/ops_key
    description: "App servers 1 through 20"
```

## `users:` map and `${key}` template expansion

The optional top-level **users:** map defines named string entries that can be referenced as
`${key}` templates in connection `user` fields. Layer merge follows the same project > user >
system priority as connections.

**Expansion rules** (applied at connect time and when generating SSH config):

1. Named entry lookup: `${key}` is replaced with the matching value from the merged `users:` map.
2. `${user}` env-var fallback: the special token `${user}` (lowercase literal) is NOT looked up
   in the `users:` map — it expands from the `$USER` environment variable. If `$USER` is unset,
   the literal `${user}` is passed through unchanged.
3. Unresolved templates: if a `${key}` token cannot be resolved, a warning is emitted to stderr
   and the literal token is passed through unchanged (non-blocking).

**`yconn connections show`** displays raw unexpanded field values — templates are never expanded in show output.

**Per-invocation overrides:** both **yconn connect** and **yconn ssh-config** accept
**--user** *KEY:VALUE* (repeatable) to override or add entries in the `users:` map for that
invocation only. **yconn ssh-config** also accepts **--skip-user** to omit `User` lines from
all generated Host blocks entirely. **--user** and **--skip-user** are mutually exclusive.

## Wildcard and range patterns

Connection names may use glob wildcards (`*` — any sequence, `?` — any single
character) or a numeric range suffix (`[N..M]`). A range pattern matches any
input whose suffix after the literal prefix is an integer in `[N, M]`
inclusive (e.g. `app[1..20]` matches `app1` through `app20`). Quote range
keys in YAML: `"app[1..20]"`. When a pattern matches the input to
**yconn connect**, the `host:` field is resolved: if it contains `${name}`,
only that token is replaced with the matched input; otherwise the entire host
field is replaced by the input. Two different patterns matching the same input
is a conflict and causes yconn to exit non-zero with a clear error.

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
yconn connections show bastion
```

Add a connection interactively to the project layer:

```
yconn connections add --layer project
```

Edit the config file that defines a connection:

```
yconn connections edit staging-db
```

Switch active group and verify:

```
yconn groups use work
yconn groups current
```

Show active config files and Docker status:

```
yconn config
```

Scaffold a project config at different locations:

```
yconn connections init                        # creates .yconn/connections.yaml (default)
yconn connections init --location dotfile     # creates .connections.yaml
yconn connections init --location plain       # creates connections.yaml
```

Generate SSH config (writes Host blocks to `~/.ssh/yconn-connections`):

```
yconn ssh-config
yconn ssh-config --dry-run
yconn ssh-config --user testuser:alice
yconn ssh-config --skip-user
```

Manage user key/value entries:

```
yconn users show
yconn users add
yconn users add --layer project
yconn users edit testuser
```

Connect with a per-invocation user override:

```
yconn connect prod-web --user testuser:alice
yconn connect staging --user user:alice
```

# FILES

`~/.config/yconn/session.yml`
: User session state — active group.

`~/.config/yconn/connections.yaml`
: User-level connection config. May reference local key paths and `users:` entries.

`~/.ssh/yconn-connections`
: Generated SSH Host blocks written by **yconn ssh-config**. Included from `~/.ssh/config`.

`~/.ssh/config`
: Standard SSH client config. **yconn ssh-config** prepends an `Include ~/.ssh/yconn-connections`
  line if absent.

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
