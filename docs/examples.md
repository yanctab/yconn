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

## Multi-group setup

**When to use:** You have logically separate sets of connections — for example `work` and
`private` — and want to switch between them cleanly without mixing entries in one file.

**File layout:**

```
~/.config/yconn/
├── work.yaml
└── private.yaml
```

**`~/.config/yconn/work.yaml`**

```yaml
version: 1

connections:
  work-web:
    host: 10.10.0.5
    user: deploy
    auth: key
    key: ~/.ssh/work_key
    description: "Work production web server"

  work-db:
    host: 10.10.0.10
    user: dbadmin
    auth: password
    description: "Work database server"
```

**`~/.config/yconn/private.yaml`**

```yaml
version: 1

connections:
  home-server:
    host: 192.168.1.100
    user: mans
    auth: key
    key: ~/.ssh/id_ed25519
    description: "Home server"

  vps:
    host: vps.example.com
    user: root
    auth: key
    key: ~/.ssh/vps_key
    description: "Personal VPS"
```

**Commands:**

```bash
# List all available groups across all layers
yconn group list

# Switch to the work group
yconn group use work

# List connections — now shows work.yaml entries
yconn list

# Connect to a work server
yconn connect work-web

# Switch to the private group
yconn group use private

# List connections — now shows private.yaml entries
yconn list

# Connect to the home server
yconn connect home-server

# Check which group is currently active and which files are loaded
yconn group current

# Revert to the default group (connections.yaml)
yconn group clear
```

**Notes:**

- The active group is stored in `~/.config/yconn/session.yml` and persists across
  terminal sessions until changed.
- Each group is independent — switching groups changes which `.yaml` file is loaded
  from every layer. A group with no file in a given layer simply skips that layer.
- Groups can also be used at the project layer. For example, a repo with both
  `.yconn/work.yaml` and a team member's user-level `~/.config/yconn/work.yaml`
  will merge the two when the `work` group is active, with the project layer winning
  on any name collision.
- `yconn group use <name>` warns if no config file for that group exists in any layer,
  but it still sets the group so you can follow up by creating the file with `yconn init`.

---

## See also

- [Configuration reference](configuration.md) — full field reference and layer system
- `man yconn` — full command reference
