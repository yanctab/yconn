# Configuration Reference

This document covers the full configuration system for yconn: the layer hierarchy, file format,
connection fields, Docker block, session file, and credential policy.

---

## Layer system

yconn loads configuration from up to three layers and merges them. Higher-priority layers win on
name collision — a connection defined in a higher-priority layer completely replaces any
connection with the same name in a lower-priority layer.

| Priority | Layer | Default path | Notes |
|---|---|---|---|
| 1 (highest) | project | `.yconn/connections.yaml` | Lives in git — no credentials |
| 2 | user | `~/.config/yconn/connections.yaml` | Private to the local user |
| 3 (lowest) | system | `/etc/yconn/connections.yaml` | Org-wide defaults, sysadmin-managed |

A layer that has no `connections.yaml` file is silently skipped.

### Project config discovery

The project layer is found by walking upward from the current working directory (like git),
checking each parent directory for a project config file. The walk stops at `$HOME` or the
filesystem root. This means running from deep inside a project tree will find the config at the
repo root.

Three filename conventions are checked in each directory, in priority order:

1. `.yconn/connections.yaml` — recommended; isolated in a subdirectory, git-trackable
2. `.connections.yaml` — dotfile convention; stays in the project root
3. `connections.yaml` — plain name; may conflict with other tools

The first match found in a directory wins; the walk then stops for that directory and moves up.

### Groups

Groups are inline tags on individual connections. Each connection entry can carry an optional
`group:` field. All connections live in `connections.yaml` regardless of their group.

The active group (set with `yconn group use <name>`) acts as a filter: when a group is locked,
`yconn list` shows only connections whose `group:` field matches. `yconn list --all` always
shows all connections regardless of any lock.

See `yconn group --help` or [docs/examples.md](examples.md#inline-group-field-usage) for group
commands and examples.

---

## Config file format

Each config file is a YAML document with a `version` key, an optional `docker` block, an
optional `users` map, and a `connections` map.

```yaml
version: 1

docker:
  image: ghcr.io/myorg/yconn-keys:latest
  pull: missing   # "always" | "missing" (default) | "never"
  args:
    - "--network=host"
    - "--env=MY_VAR=value"
    - "--volume=/opt/certs:/opt/certs:ro"

users:
  t1user: "t1extmzigher"   # referenced as ${t1user} in connection user fields

connections:
  prod-web:
    host: 10.0.1.50
    user: deploy
    port: 22
    auth: key
    key: ~/.ssh/prod_deploy_key
    description: "Primary production web server"
    group: work
    link: https://wiki.internal/servers/prod-web

  staging-db:
    host: staging.internal
    user: dbadmin
    auth: password
    description: "Staging database server — use with caution"
    group: work
    link: https://wiki.internal/servers/staging-db

  bastion:
    host: bastion.example.com
    user: ec2-user
    port: 2222
    auth: key
    key: ~/.ssh/bastion_key
    description: "Bastion host — jump point for internal network"
```

---

## Connection fields

| Field | Required | Description |
|---|---|---|
| `host` | yes | Hostname or IP address. May contain glob wildcards (`*`, `?`) — see [Wildcard patterns](#wildcard-patterns) below. |
| `user` | yes | SSH login user |
| `port` | no | SSH port — defaults to `22` |
| `auth` | yes | `key` or `password` |
| `key` | if `auth: key` | Path to private key file. When using Docker, the path is resolved inside the container. |
| `description` | yes | Human-readable description of the connection |
| `group` | no | Inline group tag. Used to filter connections with `yconn group use` or `yconn list --group`. |
| `link` | no | URL for further documentation (wiki, runbook, etc.) |

---

## Wildcard patterns

Connection names (YAML keys in the `connections:` map) can use two kinds of patterns:

**Glob wildcards:**
- `*` — matches any sequence of characters
- `?` — matches any single character

**Numeric range** — `[N..M]` at the end of the name:
- matches any input whose suffix after the literal prefix parses as an integer in `[N, M]` inclusive

```yaml
connections:
  web-*:
    host: "${name}.corp.com"   # ${name} is replaced with the matched input
    user: deploy
    auth: key
    key: ~/.ssh/web_key
    description: "Any web server matching web-*"

  "server[1..10]":
    host: "${name}.internal"   # e.g. server5.internal
    user: ops
    auth: key
    key: ~/.ssh/ops_key
    description: "Servers 1 through 10"

  db-*:
    host: "${name}"            # equivalent to omitting the template — input used directly
    user: dbadmin
    auth: password
    description: "Any database server matching db-*"
```

When you run `yconn connect web-prod-01`, yconn matches the input against all known
patterns. The `host:` field is then resolved:

- If `host` contains `${name}`, only that token is replaced with the matched input.
  `host: ${name}.corp.com` with input `web-prod-01` → SSH target `web-prod-01.corp.com`.
- If `host` does not contain `${name}`, the entire field is replaced by the matched input.
  This is the legacy behaviour — a blank or placeholder host still works as before.

**Lookup rules:**

1. Exact name match wins immediately. No conflict check is performed. The `host:` field
   is used as-is — `${name}` is not expanded for exact matches.
2. If no exact match, all glob and range patterns are tested against the input.
3. Exactly one pattern must match. Its `host:` field is resolved as described above.
   If two different patterns (including a glob and a range) both match the same input,
   yconn exits with a conflict error naming each pattern and its source file.
4. Same-pattern names across layers follow normal priority rules (higher layer wins) and
   do not trigger conflict detection.

**Range pattern notes:**
- Quote the YAML key when the name contains `[` to avoid YAML parse issues: `"server[1..10]"`.
- An empty range (`end < start`) never matches any input.
- Only a numeric suffix is matched — `server01` matches `server[1..10]` (suffix `01` parses to `1`).

---

## `users:` map and template expansion

The optional top-level `users:` map defines named string entries that can be referenced as
`${key}` templates inside connection `user` fields.

```yaml
version: 1

users:
  t1user: "t1extmzigher"
  devops: "ops-team"

connections:
  prod-web:
    host: 10.0.1.50
    user: ${t1user}        # expands to "t1extmzigher" at connect time
    auth: key
    key: ~/.ssh/prod_key
    description: "Production web server"

  staging:
    host: staging.internal
    user: ${user}          # expands to the $USER environment variable
    auth: password
    description: "Staging server"
```

### Layer merge for `users:`

The `users:` map follows the same project > user > system priority as connections. A key defined
in a higher-priority layer shadows the same key in lower layers.

### `${key}` template expansion rules

When `yconn connect` or `yconn ssh-config` is invoked, the `user` field of each connection is
scanned for `${key}` tokens:

1. **Named entry lookup** — if `key` matches an entry in the merged `users:` map, the token is
   replaced with that entry's value.
2. **`${user}` env-var fallback** — the special token `${user}` (literal lowercase `user`) is
   NOT looked up in the `users:` map. Instead, it is replaced with the value of the `$USER`
   environment variable. If `$USER` is unset, the literal `${user}` is passed through unchanged.
3. **Unresolved templates** — if a `${key}` token remains after both steps (no map entry, no env
   var match), a warning is emitted to stderr and the literal token is passed through unchanged.
   This is non-blocking.

Named entry lookup always occurs before `${user}` env-var expansion. To override `${user}` for
a single invocation, use `--user user:<name>` (see below).

### `yconn show` displays raw values

`yconn show <name>` prints the `user` field value as-is, without any template expansion. The
raw config value (e.g. `${t1user}`) is shown, not the expanded value.

### Per-invocation overrides: `--user KEY:VALUE`

Both `yconn connect` and `yconn ssh-config` accept `--user KEY:VALUE` (repeatable) to override
or add entries in the `users:` map for that invocation only:

```bash
# Connect as "alice" regardless of what ${t1user} resolves to in config
yconn connect prod-web --user t1user:alice

# Override the ${user} env-var expansion for this invocation
yconn connect staging --user user:alice

# Apply multiple overrides at once
yconn connect prod-web --user t1user:alice --user devops:bob
```

The `yconn ssh-config` command also supports `--skip-user` to omit `User` lines entirely from
all generated Host blocks. `--skip-user` and `--user` are mutually exclusive.

### Managing `users:` entries

```bash
# List all user entries across all layers
yconn user show

# Add a user entry interactively (defaults to user layer)
yconn user add
yconn user add --layer project

# Open the source file for a named entry in $EDITOR
yconn user edit t1user
yconn user edit t1user --layer user
```

---

## Docker block

The `docker` block, when present, causes yconn to re-invoke itself inside a container before
connecting. This allows SSH keys to be pre-baked into an image rather than distributed to
developer machines.

```yaml
docker:
  image: ghcr.io/myorg/yconn-keys:latest
  pull: missing
  args:
    - "--network=host"
```

| Field | Required | Description |
|---|---|---|
| `image` | yes (to enable Docker mode) | Docker image to re-invoke yconn inside. If absent, Docker mode is disabled. |
| `pull` | no | When to pull the image: `always`, `missing` (default), or `never` |
| `args` | no | List of additional arguments appended to `docker run` before the image name |

`args` are inserted after yconn's own injected arguments and before the image name. This allows
extending the container with extra networks, volumes, environment variables, or any other
`docker run` flag without changing yconn itself. Arguments are passed through verbatim — yconn
does not validate them.

### Trusted layers for docker block

The `docker` block is **only** trusted from the **project** (`.yconn/`) and **system**
(`/etc/yconn/`) layers. If a `docker` block is found in the user layer (`~/.config/yconn/`),
it is ignored with a warning. This prevents a user config from silently redirecting execution
to an arbitrary Docker image.

---

## Docker bootstrap behaviour

When `docker.image` is configured and yconn is not already running inside a container, it
constructs and executes the following `docker run` command:

```
docker run
  --name yconn-connection-<pid>
  -i
  -t
  --rm
  -e CONN_IN_DOCKER=1
  -v <yconn-binary>:<yconn-binary>:ro
  -v /etc/yconn:/etc/yconn:ro
  -v ${HOME}/.config/yconn:${HOME}/.config/yconn
  -w $(pwd)
  [user args from docker.args]
  <image>
  yconn <subcommand> <args>
```

The original subcommand and arguments are passed through verbatim.

### What gets mounted

| Host path | Container path | Mode | Purpose |
|---|---|---|---|
| `yconn` binary | same absolute path | `ro` | Same binary runs inside the container |
| `/etc/yconn/` | `/etc/yconn/` | `ro` | System layer config |
| `~/.config/yconn/` | `~/.config/yconn/` | `rw` | User layer config and `session.yml` |
| `$(pwd)` | `$(pwd)` | `ro` | Working dir — enables upward walk to find project config |

The project-level `.yconn/` config is not explicitly mounted. It is reached via the `-w $(pwd)`
working directory mount combined with the upward directory walk that yconn performs at startup.

All mounts except `~/.config/yconn` are read-only. The user config directory is read-write so
that `session.yml` can be updated from inside the container (for example, `yconn group use`
works correctly whether invoked inside or outside Docker).

### Container detection

yconn considers itself inside a container when **any** of the following are true:

- The file `/.dockerenv` exists
- The environment variable `CONN_IN_DOCKER` is set to `1`

yconn sets `CONN_IN_DOCKER=1` in the environment of every container it starts. This prevents
infinite re-invocation even in images that do not create `/.dockerenv`.

### Verbose output

Pass `--verbose` to print the full `docker run` command before it is executed:

```
[yconn] Docker image configured: ghcr.io/myorg/yconn-keys:latest
[yconn] Not running inside container — bootstrapping into Docker
[yconn] Running: docker run \
         --name yconn-connection-84732 \
         -i -t --rm \
         -e CONN_IN_DOCKER=1 \
         -v /usr/local/bin/yconn:/usr/local/bin/yconn:ro \
         -v /etc/yconn:/etc/yconn:ro \
         -v /home/user/.config/yconn:/home/user/.config/yconn \
         -w /home/user/projects/acme \
         --network=host \
         ghcr.io/myorg/yconn-keys:latest \
         yconn connect prod-web
```

---

## Session file

`~/.config/yconn/session.yml` holds user session state that persists across invocations. It is
never committed to git and is scoped to the local user only.

```yaml
active_group: work
```

| Key | Description |
|---|---|
| `active_group` | The active group name. Omit or leave blank to use the default (show all untagged and tagged connections). |

The file is designed for forward compatibility — unknown keys are ignored rather than causing
errors. An empty or absent file is valid and treated as all-defaults.

---

## Credential policy

| Layer | Path | Credential policy |
|---|---|---|
| project | `.yconn/` | Must never contain credentials. Host, user, auth type, key name references, and docker config only. Git-tracked configs are scanned on load; a warning is emitted if credential fields are found. |
| user | `~/.config/yconn/` | May reference local key paths. This is the only layer where credential references belong. |
| system | `/etc/yconn/` | Must never contain credentials. Org-wide defaults managed by sysadmins. |

Passwords are never stored in any config file and are never passed as CLI arguments or
environment variables. SSH prompts natively when `auth: password` is configured. Key
passphrases are delegated entirely to `ssh-agent`.

All security warnings are non-blocking — yconn will still proceed after warning.

---

## See also

- [Examples](examples.md) — copy-paste-ready scenarios
- `man yconn` — full command reference
