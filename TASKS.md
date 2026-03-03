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

- [x] **Update README installation instructions for Arch and Debian pre-built packages** [docs] S
  - Acceptance: Arch Linux section replaces `yay -S yconn` with both a one-step `sudo pacman -U <github-release-url>` form and a two-step `wget`/`curl` download + `sudo pacman -U ./yconn-VERSION-1-x86_64.pkg.tar.zst` form; Debian/Ubuntu section is updated with a `wget`/`curl` download command and `sudo dpkg -i yconn_VERSION_amd64.deb` (or `sudo apt install ./yconn_VERSION_amd64.deb`) install step; all URLs use the correct `yanctab/yconn` repository (fixing the stale `mans/yconn` reference); version placeholders use a clearly-labelled variable (e.g. `VERSION=1.2.0`) so examples stay meaningful without going stale
  - Depends on: Add make install target
  - Modify: README.md
  - Create: none
  - Reuse: scripts/build-pkg.sh:OUTFILE (Arch filename pattern yconn-VERSION-1-x86_64.pkg.tar.zst), scripts/build-deb.sh:PKG (Debian filename pattern yconn_VERSION_amd64.deb)
  - Risks: version numbers in shell examples will go stale — use a `VERSION=x.y.z` variable assignment before the download command so users only need to update one line; pacman -U with a URL fetches and installs in one step — note this requires network access

- [x] **Add make coverage target and call it from make test** [test] S
  - Acceptance: `make coverage` runs `cargo llvm-cov --summary-only`, prints the coverage percentage to stdout, and exits 0; `make test` calls `$(MAKE) coverage` as its last step; the `## coverage` and `## test` help comments are updated to reflect this; CI workflow (`.github/workflows/ci.yml`) adds `llvm-tools-preview` to the `dtolnay/rust-toolchain` components and installs `cargo-llvm-cov` before running `make test` so CI continues to pass; if `cargo-llvm-cov` is not installed locally, `make coverage` fails with a clear message directing the user to run `cargo install cargo-llvm-cov`
  - Depends on: Update README installation instructions for Arch and Debian pre-built packages
  - Modify: Makefile, .github/workflows/ci.yml
  - Create: none
  - Reuse: Makefile:test (target to extend with coverage call), .github/workflows/ci.yml:dtolnay/rust-toolchain (add llvm-tools-preview component)
  - Risks: `cargo-llvm-cov` must be installed on both CI and local machines — CI step must run `cargo install cargo-llvm-cov --locked` before `make test`; the musl TARGET in the Makefile is not used by `make test` (plain `cargo test`), so llvm-cov should work without musl flags; adding coverage as last step of test means a cold dev machine without the tool will fail `make test` — acceptable if the error message is clear

- [x] **Add make setup target for developer environment bootstrap** [packaging] S
  - Acceptance: `make setup` installs all Rust toolchain prerequisites in one command: runs `rustup component add rustfmt clippy llvm-tools-preview`, `rustup target add x86_64-unknown-linux-musl`, and `cargo install cargo-llvm-cov --locked`; it also prints a note listing the required system packages (`musl-tools`, `pandoc`, `zstd`) with the apt install command for reference, since those cannot be installed without sudo in a portable way; `make help` shows the setup target with an accurate description; README.md Development section documents `make setup` as the first step for new contributors, followed by the system package install command
  - Depends on: Add make coverage target and call it from make test
  - Modify: Makefile, README.md
  - Create: none
  - Reuse: .github/workflows/release.yml (exact apt packages: musl-tools pandoc zstd; exact rustup target: x86_64-unknown-linux-musl), .github/workflows/ci.yml (rustfmt clippy components)
  - Risks: system package install cannot be automated portably — print the apt command as a hint rather than running it; all rustup and cargo install commands are idempotent so re-running setup is safe; cargo install may be slow on first run — acceptable for a one-time setup target

- [x] **Auto-install system dependencies in make setup using distro detection** [packaging] S
  - Acceptance: `scripts/install-deps.sh` detects the Linux distribution from `/etc/os-release` (`ID` and `ID_LIKE` fields) and installs the required system packages automatically — `sudo apt-get install -y musl-tools pandoc zstd` on Debian/Ubuntu/derivatives, `sudo pacman -S --noconfirm musl pandoc zstd` on Arch Linux; if the distro is unsupported the script exits 1 with a clear message listing the packages to install manually; `make setup` calls `scripts/install-deps.sh` instead of printing the static apt hint; `make test` passes
  - Depends on: Add make setup target for developer environment bootstrap
  - Modify: Makefile
  - Create: scripts/install-deps.sh
  - Reuse: scripts/build-pkg.sh (set -euo pipefail pattern), .github/workflows/release.yml (Debian package names: musl-tools pandoc zstd)
  - Risks: script requires sudo — will prompt for password interactively; Arch package name is `musl` not `musl-tools`; `/etc/os-release` ID_LIKE check needed for Ubuntu (ID=ubuntu, ID_LIKE=debian) and derivatives like Mint; script must be chmod +x

- [ ] **Replace per-file group system with inline `group` field on connections** [core] L
  - Acceptance: (1) config is always loaded from `connections.yaml` in each layer — group-named files (`work.yaml` etc.) are no longer loaded; (2) each connection supports an optional `group: <name>` YAML field, round-tripped through `RawConn` → `Connection`; (3) `yconn list` with no flags shows all connections when no group is locked, or only the locked group's connections when `yconn group use <name>` has been called; (4) `yconn list --all` always shows all connections regardless of any lock; (5) `yconn list --group <name>` explicitly filters to connections with that group value (`--group` takes precedence over the lock; `--all` overrides everything); (6) `yconn group use <name>` warns if no connections with that group value exist in any layer but still sets the lock; (7) `yconn group list` displays unique `group` values found across all connections in all loaded layers instead of scanning for files; (8) `yconn init` and `yconn add` always target `connections.yaml`; (9) unit and functional tests cover: group field in YAML round-trip, locked-group filtering, `--group` flag, `--all` override, `group list` from connection entries; `make test` passes
  - Depends on: Expand `~` in key paths before file existence and permission checks
  - Modify: src/config/mod.rs, src/group/mod.rs, src/commands/group.rs, src/commands/list.rs, src/commands/add.rs, src/commands/init.rs, src/cli/mod.rs, src/main.rs, tests/functional.rs
  - Create: none
  - Reuse: src/config/mod.rs:RawConn (follow `key`/`link` `#[serde(default)]` pattern for new `group` field), src/config/mod.rs:build_connection (single point to add group field mapping), src/cli/mod.rs:all (global flag pattern for `--group`), src/group/mod.rs:read_session_at/write_session_at (session.yml unchanged — still holds locked group name), tests/functional.rs:TestEnv (functional test infrastructure)
  - Risks: largest change in codebase — many unit tests assert on group-named files and must be updated; discover_groups() signature changes from dir-scanning to connection-scanning, all callers need updating; LoadedConfig.group currently drives file loading — its meaning must shift to "filter hint only" without affecting file path construction; existing work.yaml/private.yaml files in users' directories will be silently ignored after migration (consider emitting a one-time warning if non-connections.yaml files are detected); define clear flag precedence: --all beats everything, --group beats locked group, locked group beats default (show all)

- [ ] **Support multiple project config filename conventions with `yconn init --location`** [core] M
  - Acceptance: the upward walk checks three filename conventions per directory in priority order — `.yconn/connections.yaml` first, then `.connections.yaml`, then `connections.yaml` — stopping at the first match before moving up; `yconn init` (no flag) scaffolds `.yconn/connections.yaml` (default, backward compatible); `yconn init --location dotfile` scaffolds `.connections.yaml` in cwd; `yconn init --location plain` scaffolds `connections.yaml` in cwd; `yconn init --help` clearly lists all three `--location` values and the resulting file path for each; init fails with a clear error if the target file already exists (same as current behaviour); unit tests in `src/config/mod.rs` cover all three upward walk conventions (found, missing, priority order when multiple exist in the same directory); unit tests in `src/commands/init.rs` cover all three `--location` values; `make test` passes
  - Depends on: Replace per-file group system with inline `group` field on connections
  - Modify: src/config/mod.rs, src/commands/init.rs, src/cli/mod.rs, src/main.rs
  - Create: none
  - Reuse: src/cli/mod.rs:layer (Option<String> with manual parse pattern — follow same approach for --location), src/commands/init.rs:run_impl (extend to accept location parameter), src/config/mod.rs:upward_walk (extend to try multiple filenames per directory level)
  - Risks: priority order within one directory must be clearly defined and tested — if both `.connections.yaml` and `connections.yaml` exist, `.yconn/connections.yaml` wins; consider using clap ValueEnum for --location to get automatic valid-values display in --help (new pattern in codebase) rather than manual string parse; `connections.yaml` (plain) could collide with other tools — note this in --help; this task depends on the group refactoring task having already simplified upward_walk() to always use the `connections.yaml` filename

- [ ] **Emit clear parse errors for badly formatted manually created config files** [core] S
  - Acceptance: when a manually created config file has a missing required connection field (e.g. `description`, `host`, `user`, or `auth`), `yconn` exits with a clear error that names the config file, the connection entry, and the missing field — not a raw serde_yaml internal error; when a config file contains invalid YAML syntax, the error names the file and the approximate location; when a config file is valid but has an empty or absent `connections` block, `yconn list` exits 0 and shows no entries without error; functional tests in `tests/functional.rs` cover: (1) manually created minimal valid project config — `yconn list` shows the entry, (2) manually created user layer config — `yconn list` shows the entry, (3) connection entry missing a required field — `yconn list` exits non-zero with a clear error message, (4) invalid YAML syntax — `yconn list` exits non-zero with file name in error, (5) valid but empty `connections` block — `yconn list` exits 0 with no output; `make test` passes
  - Depends on: Expand `~` in key paths before file existence and permission checks
  - Modify: src/config/mod.rs, tests/functional.rs
  - Create: none
  - Reuse: src/config/mod.rs:load_layer (where serde_yaml parse errors currently surface — improve message wrapping here), tests/functional.rs:TestEnv (functional test infrastructure), tests/functional.rs:conn_key/conn_password (YAML helpers for valid entries), src/commands/list.rs:simple_conn (minimal YAML helper pattern to follow)
  - Risks: serde_yaml error messages are opaque by default — wrapping with anyhow context at the parse site in load_layer() is the right approach; the improved error must include the file path (already available at the call site) and ideally the connection name and field, though serde_yaml may not always provide field-level detail; ensure the error path is exercised in functional tests by asserting on stderr content not just exit code

- [ ] **Show valid values for `--layer` and other fixed-choice flags in `--help` output** [cli] S
  - Acceptance: `yconn add --help`, `yconn edit --help`, and `yconn remove --help` all clearly show the valid values for `--layer` — either via clap `ValueEnum` (which auto-renders `[possible values: system, user, project]`) or via an explicit `value_name` and doc comment update; an invalid `--layer` value produces a clear error listing valid choices; the same `ValueEnum` pattern is introduced consistently so the planned `--location` flag for `yconn init` can follow it; no regression in existing behaviour; `make test` passes
  - Depends on: Expand `~` in key paths before file existence and permission checks
  - Modify: src/cli/mod.rs, src/commands/add.rs, src/commands/edit.rs, src/commands/remove.rs
  - Create: none
  - Reuse: src/cli/mod.rs:Commands (pattern for adding derive macros to CLI types), src/commands/add.rs:resolve_layer + src/commands/edit.rs:parse_layer + src/commands/remove.rs:parse_layer (to be simplified or removed if clap handles validation via ValueEnum), clap ValueEnum derive (already available via the "derive" feature in Cargo.toml)
  - Risks: switching --layer from Option<String> to Option<LayerArg> (a ValueEnum) changes the type flowing into command handlers — the three parse_layer()/resolve_layer() functions must be updated or replaced; ValueEnum is a new derive macro pattern in this codebase — introduce it cleanly in cli/mod.rs so future flags (--location) can follow the same pattern; clap renders possible values in alphabetical order by default — verify the output reads naturally for the user

- [ ] **Print full SSH command before execution when `--verbose` is set** [cli] S
  - Acceptance: `yconn connect <name> --verbose` prints `[yconn] Running: ssh [-i <key>] [-p <port>] <user>@<host>` to stderr immediately before replacing the process, using the same multi-line `[yconn] Running: …` format already used by the Docker path (`renderer.verbose_docker_cmd()`); without `--verbose` no extra output is printed; unit tests verify the verbose SSH command is correctly formatted for all four SSH arg scenarios (key auth default port, key auth custom port, password auth default port, password auth custom port); `make test` passes
  - Depends on: Expand `~` in key paths before file existence and permission checks
  - Modify: src/connect/mod.rs, src/commands/connect.rs
  - Create: none
  - Reuse: src/display/mod.rs:verbose_docker_cmd (same multi-line Running: format), src/docker/mod.rs:exec (reference pattern for verbose printing before execvp), src/connect/mod.rs:build_args (already returns the full arg list to print)
  - Risks: connect::exec() currently takes only conn: &Connection — adding renderer and verbose changes its signature and may affect callers; prefer printing the args in commands/connect.rs before calling connect::exec() to avoid signature churn; ensure the printed path reflects tilde expansion (if the ~ expansion task is implemented first the printed path will already be correct)

- [ ] **Show `link` field in `yconn list` output** [cli] S
  - Acceptance: `yconn list` includes a LINK column as the last column; the column is omitted entirely when no connection in the result set has a link (so users without links see no change); long URLs are truncated to a reasonable max width with a trailing `…`; `yconn list --all` also shows links correctly for shadowed rows; unit tests in `src/display/mod.rs` cover: link column present when at least one row has a link, column absent when no rows have a link, truncation of long URLs; `src/commands/list.rs` unit tests updated to pass the `link` field; functional test in `tests/functional.rs` runs `yconn list` against a config with a linked connection and asserts the URL appears in stdout
  - Depends on: Auto-install system dependencies in make setup using distro detection
  - Modify: src/display/mod.rs, src/commands/list.rs, tests/functional.rs
  - Create: none
  - Reuse: src/display/mod.rs:pad (column padding helper), src/display/mod.rs:ConnectionDetail (reference for how link is already rendered in show), src/commands/show.rs (link field mapping pattern), tests/functional.rs:TestEnv (functional test setup and run())
  - Risks: long URLs can break table alignment — enforce a max column width (e.g. 50 chars) with truncation; the column width array in render_list() is currently fixed-size ([usize; 7]) — adding a column requires widening it to [usize; 8] and updating all index references; all existing unit tests for render_list() must be updated to include the new `link: None` field on ConnectionRow

- [ ] **Expand `~` in key paths before file existence and permission checks** [core] S
  - Acceptance: `yconn connect <name>` does not emit a "key file does not exist" warning when the key path in config uses `~` and the file actually exists; the warning is still correctly emitted when the key file genuinely does not exist; key file permission check also operates on the expanded path; unit tests in `src/commands/connect.rs` cover a tilde key path where the file exists (no warning) and one where it does not (warning emitted); all existing tests continue to pass with `make test`
  - Depends on: Set 0o600 permissions on config files created by yconn
  - Modify: src/commands/connect.rs
  - Create: none
  - Reuse: src/config/mod.rs:dirs::home_dir (already available for home directory resolution), src/security/mod.rs:check_key_file (called after expansion), tests/functional.rs:write_key (key file test setup pattern)
  - Risks: only expand a leading `~` (i.e. `~/...`) — do not attempt `~user` expansion; if `dirs::home_dir()` returns None, skip expansion and let the path pass through unchanged rather than panicking; consider also expanding in `src/connect/mod.rs:build_args()` so verbose output shows the real path — but note that existing unit tests there assert tilde paths appear verbatim in SSH args and would need updating if expansion is added

- [ ] **Set 0o600 permissions on config files created by yconn** [core] S
  - Acceptance: files created by `yconn add`, `yconn init`, and `yconn group use`/`yconn group clear` are written with 0o600 permissions; running `yconn add` followed immediately by any `yconn` command that loads config produces no world-readable permission warning for the newly created file; unit tests in each affected module assert the written file has mode 0o600; all existing security module tests continue to pass with `make test`
  - Depends on: Fix YAML indentation bug in `yconn add` and add functional round-trip tests
  - Modify: src/commands/add.rs, src/commands/init.rs, src/group/mod.rs
  - Create: none
  - Reuse: src/security/mod.rs:file_mode (permission mode reading helper), tests/functional.rs:PermissionsExt+fs::set_permissions (existing pattern for mode assertions), src/security/mod.rs:make_file (test helper pattern for mode verification)
  - Risks: std::fs::write() does not accept a mode — use fs::set_permissions() immediately after each write (small TOCTOU window, acceptable for config files) or switch to OpenOptions with .mode(0o600); PermissionsExt::mode() is Unix-only — wrap permission-setting code in #[cfg(unix)] to keep non-Unix builds compiling; init.rs writes to the project layer (.yconn/) which is git-tracked and may legitimately be group-readable — decide whether 0o600 is correct there or whether project-layer files should use a less restrictive mode (e.g. 0o644)

- [ ] **Fix YAML indentation bug in `yconn add` and add functional round-trip tests** [core] S
  - Acceptance: `yconn add` writes a valid, parseable YAML entry (connection name at 2-space indent, fields at 4-space indent under `connections:`); `yconn show <name>` succeeds immediately after add without errors; `yconn edit <name>` is covered by a functional test using a mock `$EDITOR` that verifies the target file path is correct and the file remains parseable after the editor exits; functional tests in `tests/functional.rs` cover add round-trip (add → list/show verifies entry) and edit invocation; all pass with `make test`
  - Depends on: Auto-install system dependencies in make setup using distro detection
  - Modify: src/commands/add.rs, tests/functional.rs
  - Create: none
  - Reuse: tests/functional.rs:TestEnv (controlled temp environment), tests/functional.rs:conn_key/conn_password (YAML helpers), src/commands/add.rs:run_with_input (input simulation pattern), src/commands/remove.rs:remove_entry (indentation detection pattern)
  - Risks: fixing build_entry() to 4-space field indentation will break existing unit tests in add.rs that assert the current (buggy) 2-space format — those tests must be updated too; verify remove_entry() indent boundary detection (≤2 check) still works after the fix; edit opens $EDITOR so functional test must inject a mock editor script via PATH (same pattern as mock ssh in TestEnv)

- [ ] **Wildcard pattern matching for connection names with conflict detection** [core] M
  - Acceptance: (1) connection entries may use glob-style wildcards in their YAML key (e.g. `host*`, `*-prod`, `web-*-db`); `yconn connect <input>` matches the input against all known connection names/patterns and uses the matched input directly as the SSH hostname (no substitution — the matched input IS the host); (2) when exactly one pattern matches the input, connection proceeds normally using the matched entry's other fields (user, port, auth, key) with `host` replaced by the literal input; (3) when two or more *different* patterns from any layer both match the same input, `yconn` exits non-zero with a clear error naming each conflicting pattern and its source layer and file; (4) same-pattern shadowing across layers follows existing priority rules and does not trigger conflict detection; (5) exact-name entries are tried before pattern matching — an exact match always wins with no conflict check; (6) unit tests in `src/config/mod.rs` cover: single pattern matches input, no pattern matches input, two patterns conflict, exact name beats pattern, same pattern in two layers is shadowing not conflict; (7) functional tests in `tests/functional.rs` cover: `yconn connect` with a wildcard pattern match (mock ssh receives correct user@input-hostname args), `yconn connect` with a conflict exits non-zero with both pattern names in stderr; `make test` passes
  - Depends on: Implement connect command with Docker bootstrap
  - Modify: src/config/mod.rs, src/commands/connect.rs, Cargo.toml
  - Create: none
  - Reuse: src/config/mod.rs:LoadedConfig::find (extend to fall through to pattern scan after exact miss), src/config/mod.rs:Connection (reuse as-is; host field overridden with matched input at call site), src/config/mod.rs:merge_connections (all connections iterated for pattern scan), src/commands/connect.rs:plan (test helper pattern for asserting SSH args without exec), tests/functional.rs:TestEnv (controlled temp environment for functional tests), tests/functional.rs:conn_key/conn_password (YAML fixture helpers)
  - Risks: no glob crate is currently in Cargo.toml — add `wildmatch` or `glob` crate (wildmatch is lighter and has no path-separator semantics that would interfere with connection name patterns); conflict detection must compare pattern strings, not matched results, so two entries with identical patterns in different layers are shadowing (one wins by priority), not a conflict; the `host` field override at the call site in commands/connect.rs changes how Connection is used — verify docker bootstrap path also receives the input-as-host; `yconn list` output for wildcard entries should show the pattern name as-is (not expanded) — no changes to list rendering needed
