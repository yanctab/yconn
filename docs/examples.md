# Examples

Copy-paste-ready scenarios for common yconn setups.

---

## Project-layer only

**When to use:** Your whole team works with the same set of servers and you want the connection
list checked into the project repository.

Create a project config file at the root of your repository:

```
your-project/
└── .yconn/
    └── connections.yaml
```

**`.yconn/connections.yaml`**

```yaml
version: 1

connections:
  prod-web:
    host: 10.0.1.50
    user: deploy
    port: 22
    auth: key
    key: ~/.ssh/prod_deploy_key
    description: "Primary production web server"
    link: https://wiki.internal/servers/prod-web

  staging-db:
    host: staging.internal
    user: dbadmin
    auth: password
    description: "Staging database server — use with caution"
    link: https://wiki.internal/servers/staging-db

  bastion:
    host: bastion.example.com
    user: ec2-user
    port: 2222
    auth: key
    key: ~/.ssh/bastion_key
    description: "Bastion host — jump point for internal network"
```

**Commands:**

```bash
# Scaffold the .yconn/ directory and connections.yaml in the current directory
yconn init

# List all connections
yconn list

# Show full details for a connection
yconn show prod-web

# Connect
yconn connect prod-web
yconn connect bastion
```

**Notes:**

- The `key` paths (e.g. `~/.ssh/prod_deploy_key`) are resolved on each developer's local
  machine. Every developer must have the referenced key files in place.
- Do not put passwords or key passphrases in this file — it lives in git. Key passphrases
  are handled by `ssh-agent`.
- Commit `.yconn/connections.yaml` to version control. All team members get the connection
  list automatically after `git pull`.

---

## Project + user layers

**When to use:** The team shares a project config, but you want to override one connection
locally — for example to use a different SSH key or a personal jump host.

**`.yconn/connections.yaml`** (committed to git — shared by the whole team)

```yaml
version: 1

connections:
  prod-web:
    host: 10.0.1.50
    user: deploy
    auth: key
    key: ~/.ssh/team_deploy_key
    description: "Primary production web server"

  staging-db:
    host: staging.internal
    user: dbadmin
    auth: password
    description: "Staging database server"
```

**`~/.config/yconn/connections.yaml`** (local to your machine — not in git)

```yaml
version: 1

connections:
  # Override the shared prod-web entry with your personal key
  prod-web:
    host: 10.0.1.50
    user: deploy
    auth: key
    key: ~/.ssh/my_personal_deploy_key
    description: "Primary production web server (personal key override)"

  # Add a private entry that only you need
  dev-vm:
    host: 192.168.1.5
    user: root
    auth: key
    key: ~/.ssh/id_rsa
    description: "Local development VM"
```

**Commands:**

```bash
# List connections — prod-web shows from user layer (your override wins)
yconn list

# See where prod-web comes from
yconn show prod-web
# Source: user (/home/you/.config/yconn/connections.yaml)

# See all entries including the shadowed team version
yconn list --all
# prod-web appears twice: user layer (active) and project layer (shadowed)

# Connect using your overridden key
yconn connect prod-web
```

**Notes:**

- The user layer (`~/.config/yconn/`) takes higher priority than the project layer
  (`.yconn/`). Any connection defined in both layers will use the user-layer values.
- The project layer entry for `prod-web` is still present — `yconn list --all` shows it
  with a `[shadowed]` tag.
- Unique connection names (like `dev-vm`) are simply merged in alongside project entries.

---

## Docker-enabled setup

**When to use:** You want SSH keys to live inside a Docker image rather than on developer
machines. Developers pull the image and connect without needing the key files locally.

**`.yconn/connections.yaml`** (committed to git)

```yaml
version: 1

docker:
  image: ghcr.io/myorg/yconn-keys:latest
  pull: missing   # pull if not already present locally
  args:
    - "--network=host"

connections:
  prod-web:
    host: 10.0.1.50
    user: deploy
    auth: key
    key: /keys/prod_deploy_key   # path inside the Docker image
    description: "Primary production web server"

  staging-db:
    host: staging.internal
    user: dbadmin
    auth: password
    description: "Staging database server"

  bastion:
    host: bastion.example.com
    user: ec2-user
    port: 2222
    auth: key
    key: /keys/bastion_key       # path inside the Docker image
    description: "Bastion host"
```

**Commands:**

```bash
# Connect — yconn detects the docker block, pulls the image if needed,
# and re-invokes itself inside the container
yconn connect prod-web

# See the full docker run command before execution
yconn connect --verbose prod-web

# Check Docker status and which image would be used
yconn config
```

**How it works:**

1. yconn starts on the host, loads the config, and finds `docker.image` set.
2. It checks whether it is already running inside a container (`/.dockerenv` or
   `CONN_IN_DOCKER=1`). It is not, so it builds a `docker run` command.
3. The container gets the yconn binary, all config layers, and the current working
   directory mounted. The original command (`connect prod-web`) is passed through.
4. Inside the container, yconn sees `CONN_IN_DOCKER=1` and skips the Docker step.
   It invokes SSH directly using the key at `/keys/prod_deploy_key` inside the image.

**Building the keys image** (example Dockerfile):

```dockerfile
FROM alpine:3.21
RUN apk add --no-cache openssh-client
COPY keys/prod_deploy_key /keys/prod_deploy_key
COPY keys/bastion_key /keys/bastion_key
RUN chmod 600 /keys/prod_deploy_key /keys/bastion_key
```

**Notes:**

- The `docker` block is only trusted from the project (`.yconn/`) and system
  (`/etc/yconn/`) layers. A `docker` block in `~/.config/yconn/` is ignored with a warning.
- Key paths in the connection entries (e.g. `/keys/prod_deploy_key`) are resolved
  inside the container, not on the host.
- Pass `--network=host` in `docker.args` only when the container needs to reach hosts
  on the host network directly.

---

## Inline group field usage

**When to use:** You have logically separate sets of connections — for example `work` and
`private` — and want to switch between them cleanly. All connections live in one
`connections.yaml` file; a `group:` field on each entry determines which set it belongs to.

**`~/.config/yconn/connections.yaml`**

```yaml
version: 1

connections:
  work-web:
    host: 10.10.0.5
    user: deploy
    auth: key
    key: ~/.ssh/work_key
    description: "Work production web server"
    group: work

  work-db:
    host: 10.10.0.10
    user: dbadmin
    auth: password
    description: "Work database server"
    group: work

  home-server:
    host: 192.168.1.100
    user: mans
    auth: key
    key: ~/.ssh/id_ed25519
    description: "Home server"
    group: private

  vps:
    host: vps.example.com
    user: root
    auth: key
    key: ~/.ssh/vps_key
    description: "Personal VPS"
    group: private
```

**Commands:**

```bash
# List all available group values found across connections
yconn group list

# Show all connections (no group filter)
yconn list

# Switch to the work group — subsequent list/connect only shows work connections
yconn group use work

# List only work connections
yconn list

# Connect to a work server
yconn connect work-web

# Switch to the private group
yconn group use private

# List only private connections
yconn list

# Connect to the home server
yconn connect home-server

# Show connections from a specific group without changing the active group
yconn list --group work

# Show all connections regardless of active group
yconn list --all

# Check which group is currently active
yconn group current

# Revert to no group filter (show all connections)
yconn group clear
```

**Notes:**

- All connections live in `connections.yaml`. The `group:` field is just a tag — no
  separate files per group.
- The active group is stored in `~/.config/yconn/session.yml` and persists across
  terminal sessions until changed.
- Connections without a `group:` field are always shown when no group filter is active.
  When a group is locked, only tagged connections matching the group are shown.
- `yconn list --all` always overrides any group filter and shows every connection.
- `yconn group use <name>` warns if no connections with that group value exist in any
  layer, but it still sets the group.

---

## Wildcard pattern usage

**When to use:** You manage many similarly-named hosts (e.g. a fleet of web servers) and
want a single connection entry to cover all of them.

**`.yconn/connections.yaml`**

```yaml
version: 1

connections:
  web-prod-*:
    host: "${name}.corp.com"   # ${name} expands to the matched input → web-prod-01.corp.com
    user: deploy
    auth: key
    key: ~/.ssh/web_prod_key
    description: "Production web fleet (web-prod-01, web-prod-02, ...)"

  db-staging-?:
    host: "${name}.db.internal"
    user: dbadmin
    auth: key
    key: ~/.ssh/db_staging_key
    description: "Staging database servers (db-staging-a, db-staging-b, ...)"

  "app[1..20]":
    host: "${name}.internal"   # app5 → app5.internal
    user: ops
    auth: key
    key: ~/.ssh/ops_key
    description: "App servers 1 through 20 (app1 … app20)"

  bastion:
    host: bastion.example.com
    user: ec2-user
    port: 2222
    auth: key
    key: ~/.ssh/bastion_key
    description: "Bastion host (exact match — takes priority over any pattern)"
```

**Commands:**

```bash
# Connect to web-prod-01 — matches web-prod-* pattern; SSH target is web-prod-01.corp.com
yconn connect web-prod-01

# Connect to web-prod-07 — same pattern, SSH target is web-prod-07.corp.com
yconn connect web-prod-07

# Connect to db-staging-a — matches db-staging-? pattern
yconn connect db-staging-a

# Connect to app5 — matches app[1..20] range; SSH target is app5.internal
yconn connect app5

# Connect to bastion — exact match wins over any wildcard pattern
yconn connect bastion

# Show which pattern covers a given input (shows pattern name in source field)
yconn show web-prod-01
```

**How pattern matching works:**

1. yconn first checks whether the input is an exact connection name. If found, it wins
   immediately — no pattern check is done.
2. All connection names are tested as patterns against the input. Two kinds are supported:
   - **Glob** — `*` matches any sequence of characters; `?` matches any single character.
   - **Numeric range** — `[N..M]` at the end of a name matches any input whose suffix after
     the literal prefix is an integer in `[N, M]` inclusive (e.g. `app[1..20]` matches
     `app1` through `app20`).
3. The `host:` field is resolved for the matched entry:
   - If `host` contains `${name}`, only that token is replaced with the matched input.
     `host: ${name}.corp.com` + input `web-prod-01` → SSH target `web-prod-01.corp.com`.
   - If `host` does not contain `${name}`, the entire field is replaced by the matched input
     (legacy behaviour — blank or placeholder hosts still work as before).
4. If two different patterns both match the same input (including a glob and a range),
   yconn exits with a conflict error naming each pattern and its source file. Resolve this
   by making your patterns non-overlapping.

**Notes:**

- Use `host: "${name}.corp.com"` to append a domain suffix to every matched input.
- Use `host: "${name}"` or leave host blank/placeholder for bare-hostname behaviour.
- Quote range-pattern YAML keys that contain `[`: `"app[1..20]"`.
- Pattern entries appear in `yconn list` with their raw pattern name (e.g. `app[1..20]`)
  in the NAME column.
- Same-pattern names across layers follow normal priority rules (higher layer wins) and
  do not trigger conflict detection.

---

## Multi-location init

**When to use:** You want to scaffold a `connections.yaml` at a specific location to match
your project conventions — the default `.yconn/` subdirectory, a dotfile, or a plain file.

The three `--location` values and their resulting paths:

| `--location` value | Resulting file path | Notes |
|---|---|---|
| `yconn` (default) | `.yconn/connections.yaml` | Isolated in subdirectory; git-trackable; recommended |
| `dotfile` | `.connections.yaml` | Hidden file in project root |
| `plain` | `connections.yaml` | Plain file in project root; may conflict with other tools |

**Commands:**

```bash
# Default — creates .yconn/connections.yaml
yconn init

# Dotfile convention — creates .connections.yaml in the current directory
yconn init --location dotfile

# Plain — creates connections.yaml in the current directory
yconn init --location plain
```

**Resulting file trees:**

```
# yconn init (default)
your-project/
└── .yconn/
    └── connections.yaml

# yconn init --location dotfile
your-project/
└── .connections.yaml

# yconn init --location plain
your-project/
└── connections.yaml
```

**Upward walk priority:**

When yconn searches upward from the working directory, it checks all three conventions
in each directory in this order:

1. `.yconn/connections.yaml`
2. `.connections.yaml`
3. `connections.yaml`

The first match in a given directory wins. The walk then moves up to the parent
directory and checks again.

**Notes:**

- All three conventions are recognised by the upward walk — you can mix conventions
  across different projects.
- `yconn init` fails with a clear error if the target file already exists.
- After running `yconn init`, edit the scaffolded file and run `yconn list` to verify.

---

## `users:` map and `${key}` expansion

**When to use:** You want to define a short alias for an SSH username that varies per developer or
per environment, and reference it in connection entries without repeating the actual value
everywhere.

**`~/.config/yconn/connections.yaml`** (user layer — private to your machine)

```yaml
version: 1

users:
  t1user: "t1extmzigher"   # your personal username on the prod cluster

connections:
  prod-web:
    host: 10.0.1.50
    user: ${t1user}           # expands to "t1extmzigher" at connect time
    auth: key
    key: ~/.ssh/prod_key
    description: "Production web server"

  staging:
    host: staging.internal
    user: ${user}             # expands to the $USER environment variable
    auth: password
    description: "Staging server (uses your local $USER)"
```

**Commands:**

```bash
# Connect — yconn expands ${t1user} to "t1extmzigher" before invoking SSH
yconn connect prod-web

# Override the users: entry for this invocation only (connect as alice instead)
yconn connect prod-web --user t1user:alice

# Override the ${user} env-var expansion for this invocation
yconn connect staging --user user:alice

# Inspect raw config values — yconn show does NOT expand templates
yconn show prod-web
# User: ${t1user}   ← raw value, not expanded

# List all user entries across all layers (with source and shadowing info)
yconn users show

# Add a new user entry interactively (defaults to user layer)
yconn users add

# Add to the project layer instead
yconn users add --layer project

# Edit the source file for a named entry
yconn users edit t1user

# Generate SSH config — expands ${t1user} in User lines
yconn ssh-config

# Generate SSH config but skip all User lines
yconn ssh-config --skip-user

# Generate SSH config overriding t1user for this run
yconn ssh-config --user t1user:alice
```

**Notes:**

- `${key}` expansion looks up `key` in the merged `users:` map (project > user > system priority).
- `${user}` (lowercase, literal) is special — it expands from the `$USER` environment variable,
  not from a `users:` map entry. Named map lookup always happens first; `${user}` env-var
  expansion is a separate fallback step.
- If a `${key}` token cannot be resolved after all expansion steps, a warning is emitted to
  stderr and the literal template string is passed through unchanged to SSH.
- `yconn show` prints raw unexpanded field values — it never expands templates.
- `--user KEY:VALUE` overrides apply for one invocation only; they are not persisted to any
  config file.

---

## See also

- [Configuration reference](configuration.md) — full field reference and layer system
- `man yconn` — full command reference
