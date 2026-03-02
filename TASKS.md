# Tasks: yconn

## Foundation
> All foundation tasks must be complete and verified before
> any implementation task is started.

- [x] **Verify make build** [foundation] S
  - Acceptance: `make build` exits 0 and produces `target/x86_64-unknown-linux-musl/release/yconn`
  - Depends on: nothing

- [x] **Verify make lint** [foundation] S
  - Acceptance: `make lint` exits 0, no warnings or errors
  - Depends on: Verify make build

- [x] **Verify make test** [foundation] S
  - Acceptance: `make test` exits 0 — all tests pass (stubs are fine at this stage)
  - Depends on: Verify make lint

- [x] **Set up GitHub repository** [foundation] S
  - Acceptance: remote configured, initial scaffold pushed to main, branch protection on main enabled
  - Depends on: Verify make test

- [x] **Push scaffold branch and open PR** [foundation] S
  - Acceptance: `chore/scaffold` pushed, PR opened, developer confirms PR is merged
  - Depends on: Set up GitHub repository

- [x] **Verify CI pipeline is live** [foundation] S
  - Acceptance: test PR opened, `ci.yml` runs end to end, all checks pass
  - Depends on: Push scaffold branch and open PR

- [x] **Verify make package runs in CI** [foundation] M
  - Acceptance: `release.yml` triggered by test tag `v0.0.1-test`, binary + `.deb` + `PKGBUILD` present in GitHub Release; test tag deleted after verification
  - Depends on: Verify CI pipeline is live

## Implementation
> Start only after all Foundation tasks are checked off.

- [x] **Implement display module** [core] L
  - Acceptance: `src/display/` renders `list` table (with `--all` shadowed entries), `show` detail block, `config` status block, `group list` table, `group current` block, `--verbose` log lines; supports `--no-color` flag; no other module writes to stdout directly; unit tests cover each output format
  - Depends on: Foundation complete

- [x] **Implement group module** [core] M
  - Acceptance: `src/group/` reads `~/.config/yconn/session.yml` (unknown keys ignored, missing file treated as default); writes `active_group` to session file; resolves active group name (default `connections`); scans all three layer directories to discover available group names; unit tests cover: read with key present, read with file absent, read with unknown keys, write, group discovery across layers
  - Depends on: Implement display module

- [x] **Implement security module** [core] M
  - Acceptance: `src/security/` checks config file permissions (warns if world-readable); detects credential fields (`password`, `passphrase`, etc.) in git-trackable layers (`.yconn/`) and emits a non-blocking warning; warns if `docker` block found in user-level config; key file existence and permission check before connect; all warnings routed through `display`; unit tests cover each warning scenario
  - Depends on: Implement display module

- [x] **Implement config module** [core] L
  - Acceptance: `src/config/` loads `<group>.yaml` from each of the three layers (project via upward walk stopping at `$HOME`/root, user, system); merges into a flat connection map with source tracking; higher-priority layer wins on name collision; shadowed entries retained for `--all`; extracts the resolved `docker` block (only from project/system layers — ignores user layer); delegates active group resolution to `group` module; delegates security validation to `security` module; unit tests cover: single layer, all three layers, name collision priority (all seven scenarios from CLAUDE.md), upward walk (finds at repo root, stops at home, no config found), docker block per layer, missing layer files silently skipped
  - Depends on: Implement group module, Implement security module

- [x] **Implement connect module** [core] M
  - Acceptance: `src/connect/` builds SSH invocation arguments from a resolved connection entry: `key` auth produces `ssh [-i key] [-p port] user@host`; `password` auth produces `ssh [-p port] user@host` (no `-i`, no password arg); replaces the current process via `execvp`; unit tests cover all four SSH arg scenarios from CLAUDE.md
  - Depends on: Implement config module

- [x] **Implement docker module** [core] M
  - Acceptance: `src/docker/` detects container via `/.dockerenv` existence and `CONN_IN_DOCKER=1` env var; builds `docker run` command with exact mounts from CLAUDE.md (binary ro, `/etc/yconn` ro, `~/.config/yconn` rw, `$(pwd)` ro); injects `CONN_IN_DOCKER=1`; appends user `args` before image name; honours `pull` field; names container `yconn-connection-<pid>`; replaces current process via `execvp`; `--verbose` output routed through `display`; unit tests cover all nine Docker bootstrap scenarios from CLAUDE.md
  - Depends on: Implement config module

- [x] **Implement read-only CLI commands** [cli] M
  - Acceptance: `yconn list` (with and without `--all`), `yconn show <name>`, `yconn config`, `yconn group list`, `yconn group current` all produce output matching the exact formats specified in CLAUDE.md; global flags `--no-color` and `--verbose` respected; missing connection name returns a clear error; integration tests exercise each command with a real temp config on disk
  - Depends on: Implement display module, Implement config module, Implement connect module, Implement docker module

- [x] **Implement connect command with Docker bootstrap** [cli] M
  - Acceptance: `yconn connect <name>` resolves the connection, runs the Docker bootstrap path when `docker.image` is configured and not already in a container, otherwise invokes SSH directly; unknown name returns a clear error; integration tests cover the Docker and non-Docker paths using exec interception (no real SSH or Docker invocations)
  - Depends on: Implement read-only CLI commands, Implement connect module, Implement docker module

- [x] **Implement group mutating commands** [cli] S
  - Acceptance: `yconn group use <name>` writes `active_group` to `session.yml` and warns if no config file for that group exists in any layer (but does not block); `yconn group clear` removes `active_group` from `session.yml`; integration tests cover all five group scenarios from CLAUDE.md
  - Depends on: Implement read-only CLI commands

- [x] **Implement mutating connection commands** [cli] M
  - Acceptance: `yconn add` interactive wizard prompts for all required fields and writes a valid entry to the chosen layer; `yconn edit <name>` opens the connection's source config file in `$EDITOR`; `yconn remove <name>` removes the entry and prompts for layer if the name is ambiguous; `yconn init` scaffolds a `<group>.yaml` in `.yconn/` of the current directory; `--layer` flag respected by `add`, `edit`, `remove`; integration tests cover add/remove round-trip and layer targeting
  - Depends on: Implement group mutating commands

- [x] **Finalize .deb packaging** [packaging] M
  - Acceptance: `packaging/deb/control` has real maintainer, description, and a `Depends: openssh-client` runtime dependency; `scripts/build-deb.sh` installs the binary to `/usr/bin`, the man page to `/usr/share/man/man1/`, and example config to `/usr/share/doc/yconn/`; `make package` produces a `.deb` that installs cleanly and `yconn --help` works after install
  - Depends on: Implement mutating connection commands

- [x] **Finalize AUR PKGBUILD** [packaging] M
  - Acceptance: `packaging/aur/PKGBUILD.template` has correct `pkgdesc`, `depends=(openssh)`, installs binary and man page; `scripts/build-aur.sh` produces a valid `dist/PKGBUILD`; template verified with `makepkg --printsrcinfo` producing a valid `.SRCINFO`
  - Depends on: Finalize .deb packaging

- [x] **Publish to crates.io** [packaging] S
  - Acceptance: `Cargo.toml` has required crates.io fields (`description`, `license`) plus discoverability fields (`repository`, `homepage`, `readme`, `keywords`, `categories`); `LICENSE` file present (MIT, 2026, yanctab); `make publish` target runs lint + test then `cargo publish`; `cargo publish --dry-run` exits 0; release CI (`release.yml`) runs `cargo publish` after GitHub Release step using `CARGO_REGISTRY_TOKEN` secret
  - Depends on: Finalize AUR PKGBUILD

- [x] **Automate version bump in make release target** [packaging] S
  - Acceptance: `make release` runs `git fetch --tags`, finds the latest semver tag, increments its minor component (resetting patch to 0), updates `version = "..."` in `Cargo.toml`, runs `cargo update -p yconn` to sync `Cargo.lock`, commits with message `yconn v<new-version>`, creates `v<new-version>` tag, and pushes commit + tag to origin; the `## release` help comment in the Makefile is updated to describe the new behaviour; `make help` output reflects this; no manual version edits are needed before running `make release`
  - Depends on: Publish to crates.io
  - Modify: Makefile
  - Create: none
  - Reuse: Makefile:VERSION (grep+sed extraction pattern), Makefile:release (target to be replaced)
  - Risks: `sed -i` syntax differs between GNU and BSD/macOS — use `sed -i ''` guard or restrict to Linux; minor bump must reset patch to 0; `git fetch --tags` requires network access — the target should fail fast if fetch fails

- [x] **Create docs directory with configuration reference and examples** [docs] M
  - Acceptance: `docs/configuration.md` covers the full config file format (all fields, credential policy, Docker block, layer priority) extracted from README.md; `docs/examples.md` contains at least four complete, copy-paste-ready scenarios (project-layer only, project + user layers, Docker-enabled setup, multi-group setup) each with realistic YAML snippets and the exact `yconn` commands to use them; README.md is trimmed to a concise overview + installation + quick start that links to the new docs pages; all relative links between README.md and docs/ resolve correctly on GitHub
  - Depends on: Automate version bump in make release target
  - Modify: README.md
  - Create: docs/configuration.md, docs/examples.md
  - Reuse: config/connections.yaml (example YAML to draw from), docs/man/yconn.1.md (existing reference content to avoid duplicating)
  - Risks: README must remain a useful standalone entry point on GitHub — do not over-strip it; avoid duplicating man page content verbatim; relative links (e.g. `[Configuration](docs/configuration.md)`) must use paths relative to the repo root to work on GitHub

- [x] **Add make install target** [packaging] S
  - Acceptance: `make install` depends on `build`, installs the binary to `$(PREFIX)/bin/$(BINARY)` (default `PREFIX=/usr/local`) using `install -Dm755`, and installs the man page to `$(PREFIX)/share/man/man1/$(BINARY).1` if `docs/man/$(BINARY).1` exists; the `## install` help comment notes that `sudo` is needed for system-level `PREFIX` paths; `make install` exits 0 and `yconn --help` works afterwards; README.md from-source section is updated to show `make install` as the installation step
  - Depends on: Create docs directory with configuration reference and examples
  - Modify: Makefile, README.md
  - Create: none
  - Reuse: Makefile:BINARY (name extraction), Makefile:build (dependency), packaging/aur/PKGBUILD.template (install -Dm755 pattern)
  - Risks: man page install must be conditional on `docs/man/$(BINARY).1` existing (make docs may not have been run); help comment should mention `sudo make install` for /usr/local; do not hardcode /usr/local — use PREFIX variable

- [ ] **Update README installation instructions for Arch and Debian pre-built packages** [docs] S
  - Acceptance: Arch Linux section replaces `yay -S yconn` with both a one-step `sudo pacman -U <github-release-url>` form and a two-step `wget`/`curl` download + `sudo pacman -U ./yconn-VERSION-1-x86_64.pkg.tar.zst` form; Debian/Ubuntu section is updated with a `wget`/`curl` download command and `sudo dpkg -i yconn_VERSION_amd64.deb` (or `sudo apt install ./yconn_VERSION_amd64.deb`) install step; all URLs use the correct `yanctab/yconn` repository (fixing the stale `mans/yconn` reference); version placeholders use a clearly-labelled variable (e.g. `VERSION=1.2.0`) so examples stay meaningful without going stale
  - Depends on: Add make install target
  - Modify: README.md
  - Create: none
  - Reuse: scripts/build-pkg.sh:OUTFILE (Arch filename pattern yconn-VERSION-1-x86_64.pkg.tar.zst), scripts/build-deb.sh:PKG (Debian filename pattern yconn_VERSION_amd64.deb)
  - Risks: version numbers in shell examples will go stale — use a `VERSION=x.y.z` variable assignment before the download command so users only need to update one line; pacman -U with a URL fetches and installs in one step — note this requires network access
