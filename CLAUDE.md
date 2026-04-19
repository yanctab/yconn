# yconn — SSH Connection Manager

## Project Overview

`yconn` is a CLI tool for managing SSH connections across teams and projects. It uses a layered
config system inspired by git and ssh, supports key-based and password-based auth, and is designed
to be shareable in DevOps environments without ever exposing credentials.

When a Docker image is configured, `yconn` re-invokes itself inside a container where SSH keys
can be pre-baked — removing the need to distribute keys to individual developer machines.

---

## Groups

A group is a named set of connections. The active group determines which config filename is
loaded from each layer. The default group is named `connections`, which maps to the filename
`connections.yaml`. Switching to a group named `work` causes `work.yaml` to be loaded from
each layer instead.

Groups allow clean separation of concerns — for example `work`, `private`, `client-acme` —
without mixing all connections into one file.

### Active group

The active group is stored in `~/.config/yconn/session.yml` as a YAML file. This file is
read on every invocation and applies globally until changed. When the file is absent, the
default group `connections` is used.

```yaml
# ~/.config/yconn/session.yml
active_group: work
```

The file is intentionally structured as YAML rather than a plain string so that additional
session-scoped state can be added in future without breaking existing installs. Any unknown
fields are ignored on read, so forward compatibility is preserved.

```
yconn group list              # show all groups found across all layers
yconn group use work          # write "work" to ~/.config/yconn/session.yml
yconn group use connections   # switch back to the default group explicitly
yconn group clear             # remove active_group from ~/.config/yconn/session.yml, revert to default
yconn group current           # print the active group name and its source file paths
```

`yconn group use <name>` warns if no config file for that group exists in any layer, but does
not block — the group is set so the user can immediately follow up with `yconn init` or by
creating the file manually.

---

## Session File

`~/.config/yconn/session.yml` holds user session state that persists across invocations.
It is never committed to git and is scoped to the local user only.

Current schema:

```yaml
active_group: work    # which group is active; omit or leave blank for default (connections)
```

The file is designed for forward compatibility — new keys can be added in future versions
without breaking existing tooling. `yconn` must ignore unknown keys rather than erroring.
All keys are optional; an empty or absent file is valid and treated as all-defaults.

---

## Config Layer System

Configs are loaded from three locations and merged. **Higher priority wins on name collision**
at the connection level — a connection defined in a higher-priority layer completely replaces
any connection with the same name in a lower-priority layer.

The filename loaded from each layer is determined by the active group: `<group>.yaml`.

| Priority | Path (default group) | Path (group "work") | Intended use |
|---|---|---|---|
| 1 (highest) | `.yconn/connections.yaml` | `.yconn/work.yaml` | Team/project-specific, lives in git |
| 2 | `~/.config/yconn/connections.yaml` | `~/.config/yconn/work.yaml` | User's private connections |
| 3 (lowest) | `/etc/yconn/connections.yaml` | `/etc/yconn/work.yaml` | Org-wide defaults, sysadmin-managed |

A layer that has no file for the active group is silently skipped — not all layers need to
define entries for every group.

**Project config discovery** works by walking upward from the current working directory (like
git), checking each parent for a `.yconn/<group>.yaml`, stopping at `$HOME` or filesystem
root. This means running from deep inside a project tree will find the config at the repo root.

**System layer override (`YCONN_SYSTEM_CONFIG_DIR`):** the system layer directory defaults to
`/etc/yconn` but can be overridden by setting the `YCONN_SYSTEM_CONFIG_DIR` environment
variable. When set, its value replaces `/etc/yconn` for both reads (config loading) and writes
(`yconn install --layer system`, `yconn connections add --layer system`,
`yconn users add --layer system`). When unset, behavior is unchanged. This override exists
primarily so the end-to-end functional test harness can isolate the system layer in a temp
directory; it is also useful for packaged installs that ship configs in a non-standard location.

**Credential policy by layer:**
- `/etc/yconn/` and `.yconn/` (git-tracked) — must never contain credentials. Host, user, auth
  type, key name references, and docker config only.
- `~/.config/yconn/` — may reference local key paths and is the only layer where credential
  references belong.

---

## Configuration File Format

```yaml
version: 1

docker:
  image: ghcr.io/myorg/yconn-keys:latest   # if set, yconn will re-invoke itself inside this image
  pull: always                             # "always", "missing", or "never" — defaults to "missing"
  args:                                    # optional additional arguments appended to docker run
    - "--network=host"
    - "--env=MY_VAR=value"
    - "--volume=/opt/certs:/opt/certs:ro"

connections:
  prod-web:
    host: 10.0.1.50
    user: deploy
    port: 22                          # optional, defaults to 22
    auth:
      type: key                       # "key" or "password"
      key: ~/.ssh/prod_deploy_key     # required when type is "key"; inside docker, path is inside container
      generate_key: "vault read -field=private_key secret/ssh/prod > ${key}"  # optional; ${key} is expanded to auth.key when displayed
    description: "Primary production web server"
    link: https://wiki.internal/servers/prod-web   # optional

  staging-db:
    host: staging.internal
    user: dbadmin
    auth:
      type: password                  # SSH will prompt at runtime; password never stored
    description: "Staging database server — use with caution"
    link: https://wiki.internal/servers/staging-db

  bastion:
    host: bastion.example.com
    user: ec2-user
    port: 2222
    auth:
      type: key
      key: ~/.ssh/bastion_key
    description: "Bastion host — jump point for internal network"
```

Each connection entry requires an explicit host. There is no pattern or glob matching —
every host that should be reachable must have its own named entry in a config file.

### Top-level `docker` block

| Field | Required | Description |
|---|---|---|
| `image` | yes (to enable) | Docker image to re-invoke `yconn` inside. If absent, Docker mode is disabled. |
| `pull` | no | When to pull the image: `always`, `missing` (default), or `never` |
| `args` | no | List of additional arguments inserted into the `docker run` command before the image name |

`args` are appended after yconn's own arguments (mounts, env vars) and before the image name.
This allows extending the container with extra networks, volumes, environment variables, or any
other `docker run` flag without forking the config. The user is responsible for ensuring supplied
args are valid — yconn passes them through verbatim without validation.

The `docker` block is only meaningful in `/etc/yconn/` or `.yconn/` layers. If defined in
`~/.config/yconn/`, it is ignored with a warning — user-level config should not redirect
execution to an arbitrary Docker image.

### Connection field reference

| Field | Required | Description |
|---|---|---|
| `host` | yes | Hostname or IP address |
| `user` | yes | SSH login user |
| `port` | no | SSH port, defaults to 22 |
| `auth` | yes | YAML mapping with `type` (required), `key` (required when type=key), and `generate_key` (optional) |
| `auth.type` | yes | `key` or `password` |
| `auth.key` | if type=key | Path to private key file (resolved inside container when using Docker) |
| `auth.generate_key` | no | Shell command string (parsed and stored only — not executed in v1). The literal token `${key}` is expanded to `auth.key` when surfaced by `yconn show` (empty string when no key is defined). `yconn show --dump` preserves the raw unexpanded value. |
| `description` | yes | Human-readable description of the connection |
| `link` | no | URL for further documentation (wiki, runbook, etc.) |

---

## Docker Bootstrap Flow

When a `docker.image` is configured and `yconn` determines it is **not** already running inside
a container, it re-invokes itself inside Docker before doing anything else.

### Default docker invocation

The following is the exact `docker run` command `yconn` constructs by default. User-supplied
`args` from the config are appended after these and before the image name.

```
docker run
  --name yconn-connection-<pid>     # PID of the host yconn process — unique and traceable
  -i                                # keep stdin open for SSH password prompts
  -t                                # allocate a TTY so terminal behaviour works correctly
  --rm                              # remove container on exit
  -e CONN_IN_DOCKER=1               # re-invocation guard
  -v <yconn-binary>:<yconn-binary>:ro          # same binary runs inside container
  -v /etc/yconn:/etc/yconn:ro                  # system layer config
  -v ${HOME}/.config/yconn:${HOME}/.config/yconn  # user layer config + session.yml
  -w $(pwd)                         # preserve working dir so upward config walk finds project config
  [user args from config]
  <image>
  yconn <subcommand> <args>         # original command passed through verbatim
```

The project-level `.yconn/` config is not explicitly mounted — it is reached via the `-w $(pwd)`
working directory mount combined with the upward directory walk that `yconn` performs at startup.

All mounts except `~/.config/yconn` are read-only. The user config directory is mounted
read-write so that `session.yml` can be updated from inside the container (e.g. `yconn group use`
works correctly whether invoked inside or outside Docker).

### Re-invocation behavior

1. `yconn` starts on the host, loads config, finds `docker.image` defined
2. Checks whether it is running inside a container (see detection below)
3. If **not** inside a container: builds the `docker run` command above, passes through the
   original subcommand and arguments verbatim, and replaces itself with the docker process
4. If **inside** a container: proceeds normally — connects via SSH using keys available in
   the image

### What gets mounted

| Host path | Container path | Mode | Purpose |
|---|---|---|---|
| `yconn` binary | same absolute path | `ro` | Same binary runs inside container |
| `/etc/yconn/` | `/etc/yconn/` | `ro` | System layer config |
| `~/.config/yconn/` | `~/.config/yconn/` | `rw` | User layer config and `session.yml` |
| `$(pwd)` | `$(pwd)` | `ro` | Working dir — enables upward walk to find project config |

### Container detection

`yconn` considers itself to be inside a container if **any** of the following are true:
- The file `/.dockerenv` exists
- The environment variable `CONN_IN_DOCKER` is set to `1`

`yconn` sets `CONN_IN_DOCKER=1` in the environment when it invokes Docker, so even if
`/.dockerenv` is absent in a custom image, the re-invocation guard still works.

### Verbose output for Docker mode

When `--verbose` is passed, the full `docker run` command is printed before execution:

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
         --env=MY_VAR=value \
         ghcr.io/myorg/yconn-keys:latest \
         yconn connect prod-web
```

---

## CLI Commands

| Command | Description |
|---|---|
| `yconn list` | List all connections across all layers |
| `yconn connect <name>` | Connect to a named host |
| `yconn show <name>` | Show the resolved config for a connection (no secrets printed) |
| `yconn add` | Interactive wizard to add a connection to a chosen layer |
| `yconn edit <name>` | Open the connection's source config file in `$EDITOR` |
| `yconn remove <name>` | Remove a connection (prompts for layer if ambiguous) |
| `yconn init` | Scaffold a `<group>.yaml` in `.yconn/` in the current directory |
| `yconn config` | Show which config files are active, their paths, and Docker status |
| `yconn group list` | Show all groups found across all layers |
| `yconn group use <n>` | Set the active group (persisted to `~/.config/yconn/session.yml`) |
| `yconn group clear` | Remove `active_group` from `session.yml`, revert to default (`connections`) |
| `yconn group current` | Print the active group name and resolved config file paths |

Global flags:
- `--layer system|user|project` — target a specific layer for `add`, `edit`, `remove`
- `--all` — include shadowed entries in `yconn list`
- `--no-color` — disable colored output
- `--verbose` — print config loading decisions, merge resolution, and full Docker invocation

---

## `yconn list` Output Format

Standard output (active connections only):

```
NAME          HOST                  USER       PORT   AUTH      SOURCE    DESCRIPTION
──────────────────────────────────────────────────────────────────────────────────────────────
prod-web      10.0.1.50             deploy     22     key       project   Primary production web server
staging-db    staging.internal      dbadmin    22     password  user      Staging database server — use with caution
bastion       bastion.example.com   ec2-user   2222   key       system    Bastion host — jump point for internal network
dev-local     192.168.1.5           root       22     key       user      Local dev VM
```

With `--all`, shadowed entries appear dimmed with a `[shadowed]` tag:

```
NAME          HOST                  USER       PORT   AUTH      SOURCE    DESCRIPTION
──────────────────────────────────────────────────────────────────────────────────────────────
prod-web      10.0.1.50             deploy     22     key       project   Primary production web server
staging-db    staging.internal      dbadmin    22     password  user      Staging database server — use with caution
bastion       bastion.example.com   ec2-user   2222   key       project   Bastion host (project override)
bastion       bastion.example.com   ec2-user   22     key       system    Bastion host [shadowed]
dev-local     192.168.1.5           root       22     key       user      Local dev VM
```

`yconn show prod-web` output:

```
Connection: prod-web
  Host:        10.0.1.50
  User:        deploy
  Port:        22
  Auth:        key
  Key:         ~/.ssh/prod_deploy_key
  Description: Primary production web server
  Link:        https://wiki.internal/servers/prod-web
  Source:      project (/home/user/projects/acme/.yconn/connections.yaml)
```

`yconn config` output (with Docker configured, active group "work"):

```
Group:   work  (set in ~/.config/yconn/session.yml)

Active config files (highest to lowest priority):
  [project]  /home/user/projects/acme/.yconn/work.yaml    (4 connections)
  [user]     /home/user/.config/yconn/work.yaml           (2 connections)
  [system]   /etc/yconn/work.yaml                         (not found)

Docker:
  Image:   ghcr.io/myorg/yconn-keys:latest
  Pull:    missing
  Source:  project
  Status:  will bootstrap into container on connect
```

`yconn group current` output:

```
Active group: work
Lock file:    ~/.config/yconn/session.yml

Resolved config files:
  [project]  /home/user/projects/acme/.yconn/work.yaml    ✓ found
  [user]     /home/user/.config/yconn/work.yaml           ✓ found
  [system]   /etc/yconn/work.yaml                         ✗ not found
```

`yconn group list` output:

```
GROUP          LAYERS
───────────────────────────────────────
connections    project, user, system
work           project, user
private        user
```

---

## Architecture

```
yconn/
├── CLAUDE.md
├── README.md
├── config/
│   └── connections.yaml         # example / documentation config
└── src/
    ├── cli                      # Entry point, command definitions, flag parsing
    ├── config                   # Layer loading, upward walk, merge logic
    ├── group                    # Active group resolution, session.yml read/write
    ├── connect                  # SSH argument construction and process invocation
    ├── docker                   # Container detection, mount resolution, docker invocation
    ├── security                 # Permission checks, credential field detection
    └── display                  # All output formatting and rendering
```

### Module responsibilities

**cli** — Parses commands and flags, delegates entirely to other modules. No business logic here.

**config** — Loads each layer in priority order, performs the upward directory walk for project
config, merges layers into a flat connection map with source tracking, and retains shadowed
entries for `--all` display. Surfaces the resolved `docker` block if present. Delegates
active group resolution to the `group` module to determine which filename to load.

**group** — Reads and writes `~/.config/yconn/session.yml`. Resolves the active group name
(defaulting to `connections` when the file is absent). Scans all layer directories to discover
which groups have config files, used by `yconn group list`.

**connect** — Takes a resolved connection entry and builds the SSH invocation arguments. Executes
SSH by replacing the current process so terminal behavior works correctly. For `Auth::Password`,
the native SSH password prompt is used — no password is ever passed programmatically. Key
passphrases are handled entirely by the user's `ssh-agent`.

**docker** — Handles all Docker-related logic: container detection, building the mount list from
discovered config file paths and the binary's own path, constructing the `docker run` command,
and replacing the current process with Docker. Completely separate from `connect` — these are
two different execution paths.

**security** — Validates file permissions on config files and key files. Detects credential
fields in git-trackable config layers. Warns if `docker` block appears in user-level config.
All warnings are non-blocking.

**display** — All user-facing output lives here. No other module writes to stdout directly.
Supports rich formatted output with a plain text fallback for non-interactive environments.
`--verbose` output (config loading, merge decisions, docker command) is also routed here.

---

## Testing Strategy

### Unit tests
- Group resolution: active group read from `session.yml`, default when absent or key missing, warn on unknown group
- Session file: unknown keys ignored, empty file valid, missing file valid
- Group discovery: scans all layers for available group files
- Config merge logic: single layer, all three layers, name collisions, missing files
- Upward directory walk: finds config at repo root, stops at home, handles no-config-found
- Docker block merge: defined in project layer, defined in system layer, ignored in user layer
- Security checks: permission warnings, credential field detection per layer type

### Functional and integration tests

Two integration boundaries are tested by intercepting the final exec call and asserting on
exact arguments — **no real SSH connections or Docker invocations are made**. Config files
are written as real temporary files on disk so the full pipeline from file load → merge →
argument construction is exercised.

**Config priority scenarios:**

| Scenario | Config setup | Expected result |
|---|---|---|
| Project overrides user | same name in project + user | project layer values used |
| Project overrides system | same name in project + system | project layer values used |
| User overrides system | same name in user + system | user layer values used |
| Project overrides both | same name in all three layers | project layer values used |
| No collision, all layers | unique names in each layer | each resolves independently |
| Name only in system | absent from project + user | system layer values used |
| Name only in user | absent from project | user layer values used |

**SSH argument scenarios:**

| Scenario | Config | Expected SSH args |
|---|---|---|
| Key auth, default port | `auth: { type: key, key: ~/.ssh/id_rsa }` | `ssh -i ~/.ssh/id_rsa user@host` |
| Key auth, custom port | `auth: { type: key, key: ~/.ssh/id_rsa }`, `port: 2222` | `ssh -i ~/.ssh/id_rsa -p 2222 user@host` |
| Password auth | `auth: { type: password }` | `ssh user@host` (no `-i`, no password arg) |
| Password auth, custom port | `auth: { type: password }`, `port: 2222` | `ssh -p 2222 user@host` |

**Group scenarios:**

| Scenario | Setup | Expected result |
|---|---|---|
| No active group file | `session.yml` absent | `connections.yaml` loaded from each layer |
| Active group set | `session.yml` has `active_group: work` | `work.yaml` loaded from each layer |
| Active group, layer file missing | `work.yaml` absent in system layer | that layer silently skipped |
| Switch group | `yconn group use work` | `session.yml` written, subsequent commands use `work.yaml` |
| Clear group | `yconn group clear` | `active_group` removed from `session.yml`, reverts to `connections` |
| Use unknown group | no `work.yaml` in any layer | warning emitted, group still set |
| `yconn group list` | files present across layers | correct group names and layer presence shown |

**Docker bootstrap scenarios:**

| Scenario | Setup | Expected behavior |
|---|---|---|
| Docker image configured, not in container | `docker.image` set, `/.dockerenv` absent, `CONN_IN_DOCKER` unset | `docker run` invoked with correct mounts and args |
| Docker image configured, inside container via env var | `CONN_IN_DOCKER=1` | Docker skipped, SSH invoked directly |
| Docker image configured, inside container via file | `/.dockerenv` present | Docker skipped, SSH invoked directly |
| Docker image configured, `pull: always` | `docker.pull: always` | `docker run` includes `--pull always` |
| Docker args included | `docker.args` set | extra args appear in `docker run` between yconn args and image name |
| Docker args empty | `docker.args` absent | `docker run` built with no extra args |
| Docker block in user config | `docker.image` in `~/.config/yconn/` | Warning emitted, Docker block ignored |
| `args` present | `args: ["--network=host"]` | args appear in `docker run` command after injected args, before image name |
| No docker block | no `docker` key in any layer | SSH invoked directly on host |
| `--verbose` with Docker | Docker image configured, not in container | Full `docker run` command printed before exec |

---

## Security Model

- Credentials (passwords, passphrases) are never stored in any config file
- Passwords are never passed as CLI arguments or environment variables — SSH prompts natively
- Key passphrases are delegated to `ssh-agent` entirely
- `yconn show` never prints passwords or passphrases
- Git-trackable config files (`.yconn/`) are scanned for credential fields on load; warning emitted if found
- The `docker` block is only trusted from `/etc/yconn/` or `.yconn/` — not from user config
- Key files are validated for existence and appropriate permissions before connecting
- Config files with overly permissive permissions emit a warning on load
- All warnings are non-blocking

---

## Non-goals (v1)

- No GUI or TUI
- No built-in secret storage — delegate to ssh-agent or OS keychain
- No tunneling or port-forward management
- No support for passing passwords programmatically (intentional — avoids process list exposure)
- No support for Docker Compose or Podman in v1 (consider for v2)

---

## Task Tracking

Active work lives in `TASKS.md` at the repo root. Completed work is archived
in `HISTORY-TASKS.md` at the repo root.

### Why two files

`TASKS.md` is loaded into the Claude context window during normal work so
that the agent can see the current backlog, dependencies, and acceptance
criteria at a glance. Over time this file grows as tasks accumulate, and
loading the full history alongside the active backlog wastes context and
slows down planning.

`HISTORY-TASKS.md` exists to keep `TASKS.md` small enough to comfortably
load into context without also dragging in every previously completed
task. It is a write-only archive — entries are never edited after they
land there.

### Archival workflow

When `TASKS.md` grows too large to comfortably load into context:

1. Identify the oldest completed `[x]` entries at the top of `TASKS.md`.
2. Move them verbatim to the bottom of `HISTORY-TASKS.md`, preserving
   every sub-bullet (Acceptance, Depends on, Modify, Create, Reuse,
   Risks) exactly as written.
3. Delete the moved entries from `TASKS.md`.
4. Commit the move as a single `docs(tasks):` commit so the history is
   easy to audit.

No specific line-count threshold triggers archival — it is a judgment
call based on how readable `TASKS.md` remains and how much context it
consumes during agent runs.
