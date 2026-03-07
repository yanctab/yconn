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
  - Acceptance: `render_list`, `render_show`, `render_config`, `render_group_list`, `render_group_current` all produce correct output; unit tests cover each formatter; `make test` passes
  - Depends on: Verify make package runs in CI
  - Modify: src/display/mod.rs
  - Create: none
  - Reuse: none
  - Risks: column widths must handle variable-length data without wrapping; shadowed-entry dimming requires ANSI codes that must not appear when stdout is not a TTY

- [x] **Implement config module** [core] L
  - Acceptance: all three layers load and merge correctly; upward directory walk finds project config; shadowed entries retained with source tracking; all config-merge unit tests and integration scenarios pass; `make test` passes
  - Depends on: Implement display module
  - Modify: src/config/mod.rs
  - Create: none
  - Reuse: none
  - Risks: upward walk must stop at `$HOME` and filesystem root; missing layer files must be silently skipped; `docker` block from user layer must be ignored with a warning; higher-priority layer entries must completely replace lower-priority entries of the same name

- [x] **Implement group module** [core] M
  - Acceptance: `session.yml` is read/written correctly; unknown keys are ignored; default group is `connections`; `yconn group list` scans all layers correctly; all group unit tests pass; `make test` passes
  - Depends on: Implement config module
  - Modify: src/group/mod.rs
  - Create: none
  - Reuse: src/config/mod.rs (layer paths)
  - Risks: session file must be forward-compatible — unknown keys silently ignored on read; `yconn group use <unknown>` must warn but still write; empty or absent session file must be treated as all-defaults

- [x] **Implement connect module** [core] M
  - Acceptance: SSH args are constructed correctly for key and password auth; `execvp` replaces the process; all SSH argument integration tests pass; `make test` passes
  - Depends on: Implement group module
  - Modify: src/connect/mod.rs
  - Create: none
  - Reuse: src/config/mod.rs:Connection (field access), src/display/mod.rs:Renderer::warn
  - Risks: password must never be passed as a CLI argument; key file path must be validated before exec; process replacement must be used (not spawn) so terminal behaviour works

- [x] **Implement docker module** [core] M
  - Acceptance: container detection works via `/.dockerenv` and `CONN_IN_DOCKER`; mount list is built correctly; `docker run` args are constructed in the right order; integration tests for docker bootstrap scenarios pass; `make test` passes
  - Depends on: Implement connect module
  - Modify: src/docker/mod.rs
  - Create: none
  - Reuse: src/connect/mod.rs (exec pattern), src/config/mod.rs:DockerConfig
  - Risks: `docker` block from user-layer config must be ignored; `--pull` flag placement must match docker CLI expectations; user-supplied `args` must appear after yconn's own injected args and before the image name

- [x] **Implement security module** [core] S
  - Acceptance: file-permission warnings fire for overly permissive config files; credential field detection fires for git-trackable layers; `docker` block in user layer triggers a warning; all security unit tests pass; `make test` passes
  - Depends on: Implement docker module
  - Modify: src/security/mod.rs
  - Create: none
  - Reuse: src/display/mod.rs:Renderer::warn, src/config/mod.rs:Layer
  - Risks: all warnings must be non-blocking; credential field detection must only fire for `.yconn/` and `/etc/yconn/` layers, not `~/.config/yconn/`

- [x] **Implement CLI commands** [cli] L
  - Acceptance: all commands (`list`, `connect`, `show`, `add`, `edit`, `remove`, `init`, `config`, `group *`) are wired to their handler modules; global flags (`--all`, `--verbose`) are threaded through; `make test` passes end to end
  - Depends on: Implement security module
  - Modify: src/cli/mod.rs, src/main.rs, src/commands/mod.rs, src/commands/add.rs, src/commands/edit.rs, src/commands/remove.rs, src/commands/init.rs, src/commands/list.rs, src/commands/show.rs, src/commands/config.rs, src/commands/connect.rs, src/commands/group.rs
  - Create: none
  - Reuse: all handler modules implemented in prior tasks
  - Risks: `--layer` flag is per-subcommand, not global; `--all` and `--verbose` are global and must be passed through clap's `global = true`; `yconn group use <unknown>` must warn but not error

- [x] **Write functional integration tests** [test] L
  - Acceptance: all config-priority, SSH-argument, group, and docker-bootstrap integration scenarios listed in CLAUDE.md pass; tests use real temp files on disk; exec is intercepted — no real SSH or Docker invocations; `make test` passes
  - Depends on: Implement CLI commands
  - Modify: tests/functional.rs
  - Create: none
  - Reuse: src/config/mod.rs:load_impl (call directly for config-layer tests), src/connect/mod.rs (intercept exec for SSH tests), src/docker/mod.rs (intercept exec for docker tests)
  - Risks: temp directories must be cleaned up after each test; exec interception must be compile-time swappable (feature flag or function pointer) so production code is not affected; docker tests must set/unset `CONN_IN_DOCKER` and mock `/.dockerenv` without touching the real filesystem

- [x] **Always print the SSH command to stderr before exec in `yconn connect`** [cli] S
  - Acceptance: every invocation of `yconn connect` prints the full SSH command to stderr immediately before `execvp`; the line is always printed regardless of `--verbose`; unit test in `src/commands/connect.rs` or `tests/functional.rs` captures stderr and asserts the printed line matches the expected `ssh …` invocation; `make test` passes
  - Depends on: Write functional integration tests
  - Modify: src/connect/mod.rs, src/display/mod.rs, tests/functional.rs
  - Create: none
  - Reuse: src/display/mod.rs:Renderer (add or reuse a stderr print method), tests/functional.rs:TestEnv (use existing harness to capture stderr)
  - Risks: output must go to stderr, not stdout, so it does not interfere with piped usage; the line must appear before exec replaces the process — ordering matters; functional test must capture stderr separately from stdout

- [x] **Collapse `yconn ssh-config generate` into `yconn ssh-config`** [cli] S
  - Acceptance: `yconn ssh-config` (no subcommand) writes Host blocks to `~/.ssh/yconn-connections` and updates `~/.ssh/config`; `yconn ssh-config --dry-run` prints to stdout without writing; the old `yconn ssh-config generate` subcommand no longer exists and produces an error if invoked; existing `ssh-config` functional tests are updated to use the new invocation form; `make test` passes
  - Depends on: Always print the SSH command to stderr before exec in `yconn connect`
  - Modify: src/cli/mod.rs, src/main.rs, src/commands/ssh_config.rs, tests/functional.rs
  - Create: none
  - Reuse: src/cli/mod.rs:Commands (replace SshConfig subcommand with top-level variant), src/commands/ssh_config.rs:run_generate (keep handler, update call site)
  - Risks: removing a subcommand is a breaking CLI change — ensure no other code path references the old `generate` variant after the rename; clap will automatically surface an error for unknown subcommands so no explicit guard is needed

- [x] **Add `users:` config section and `yconn user show|add|edit` commands; expand `${<key>}` templates in connection user fields** [core] L
  - Acceptance: (1) `users:` map is loaded from all three layers and merged with the same priority rules as `connections:`; (2) `${key}` tokens in a connection's `user` field are expanded to the value from the merged `users:` map before SSH exec; (3) `${user}` (the literal string "user") is further expanded to `$USER` env var as a second pass; (4) an unresolved `${key}` token emits a non-blocking warning and is left in the string; (5) `yconn user show` renders a table of all user entries with source and shadowing info; (6) `yconn user add` wizard prompts for key and value and writes to the `users:` section of the target layer's `connections.yaml`; (7) `yconn user edit <key>` opens the source file for that entry in `$EDITOR`; (8) inline `--user key:value` flags on `connect` and `ssh-config` shadow config-loaded entries for that invocation only; (9) `yconn show` displays raw unexpanded field values; (10) all new unit and functional tests pass; `make test` passes
  - Depends on: Always print the SSH command to stderr before exec in `yconn connect`, Collapse `yconn ssh-config generate` into `yconn ssh-config`
  - Modify: src/config/mod.rs, src/cli/mod.rs, src/main.rs, src/commands/mod.rs, src/commands/connect.rs, src/commands/ssh_config.rs, src/display/mod.rs, tests/functional.rs
  - Create: src/commands/user.rs
  - Reuse: src/config/mod.rs:RawFile (add `users: HashMap<String, String>` field with `#[serde(default)]`), src/config/mod.rs:LoadedConfig (add `users: HashMap<String, UserEntry>` where `UserEntry` carries value + layer + source_path + shadowed), src/config/mod.rs:merge_connections (follow same priority-merge pattern for `users:` map), src/config/mod.rs:Layer (reuse for `UserEntry.layer`), src/commands/add.rs:run_impl/build_entry/write_entry (follow wizard, YAML-write, and file-write patterns for `yconn user add`), src/commands/add.rs:layer_arg_to_layer/layer_path (reuse directly in user.rs), src/commands/edit.rs:run (follow open-in-$EDITOR pattern for `yconn user edit`), src/display/mod.rs:Renderer::warn (non-blocking warning for unresolved templates), src/commands/connect.rs:run (merge inline `--user` pairs into local users map copy, apply named user expansion, then `${user}` env expansion, before SSH exec), src/commands/ssh_config.rs:render_ssh_config (apply named user expansion then `${user}` env expansion using merged users map in the `User` line rendering path)
  - Risks: `${user}` (env-var expansion) must NOT be treated as a named entry lookup — guard by checking `key == "user"` and skipping the `users:` map for that token, leaving it for the `$USER` expansion step; named user expansion must happen before `${user}` expansion to avoid treating a `users:` value of `${user}` as an infinite loop — resolve one level only; inline `--user` pairs are merged into a local copy of the users map before expansion — they shadow config-loaded entries for that invocation only; `yconn user add` writes to the `users:` section of `connections.yaml` (not a separate file) — the YAML write helper must insert under `users:` analogously to how `add.rs` inserts under `connections:`; `yconn user show` must display shadowed entries similarly to `yconn list --all` — the display module needs a new render function or a generalised table helper; the `users:` map merge must be implemented separately from `merge_connections` since entries are plain strings not Connection structs, but the shadowing logic is identical; `--skip-user` and `--user` are mutually exclusive on `ssh-config` — clap's `conflicts_with` enforces this; parse each `--user` value by splitting on the first `:` — if no `:` is present, exit with a clear error message; unresolved template warning must fire independently on both the `connect` path and the `ssh-config` path; `yconn show` must not call the expansion path — it renders raw config field values

- [x] **Update all documentation to reflect the current state of yconn** [docs] M
  - Acceptance: README.md, docs/configuration.md, docs/examples.md, and docs/man/yconn.1.md are all updated so every implemented feature is documented and no stale content remains; specifically: (1) README.md commands table adds `yconn ssh-config`, `yconn user show`, `yconn user add`, `yconn user edit`; global flags line removes `--layer` and `--no-color` (no longer global) and corrects their scope to per-subcommand; (2) docs/configuration.md gains a `users:` section documenting the top-level map format, the `key: "value"` entry syntax, layer merge priority, `${key}` template expansion in `user` fields, `${user}` → `$USER` env-var expansion, warning on unresolved templates, and the note that `yconn show` displays the raw unexpanded value; (3) docs/examples.md gains a `users: map and ${key} expansion` scenario with a realistic YAML snippet showing a `users:` block, a connection with `user: ${t1user}`, and `yconn connect`, `yconn connect --user t1user:alice`, and `yconn user show` invocations; (4) docs/man/yconn.1.md adds `ssh-config` command description (writes Host blocks to `~/.ssh/yconn-connections`, updates `~/.ssh/config`, flags `--dry-run`, `--user KEY:VALUE`, `--skip-user`), `user show|add|edit` command descriptions, a `users:` subsection under CONFIGURATION documenting the map format and `${key}` expansion rules, and removes `--layer` and `--no-color` from the global OPTIONS section (they are now per-subcommand); `make test` passes (no Rust changes required — docs only)
  - Depends on: Add `users:` config section and `yconn user show|add|edit` commands; expand `${<key>}` templates in connection user fields
  - Modify: README.md, docs/configuration.md, docs/examples.md, docs/man/yconn.1.md
  - Create: none
  - Reuse: src/cli/mod.rs:Commands::SshConfig (exact flags: dry_run, user_overrides, skip_user), src/cli/mod.rs:UserCommands (Show/Add/Edit variants and their --layer flags), src/config/mod.rs:RawFile::users (field name and type for YAML format docs), src/commands/user.rs:add_impl (wizard prompt labels "Key" and "Value" for accurate docs), src/display/mod.rs:user_list (informs yconn user show output format description)
  - Risks: README.md still lists `--layer system|user|project` and `--no-color` as global flags — those lines must be corrected to reflect that `--layer` is a per-subcommand flag for add/edit/remove only and `--no-color` no longer exists; docs/man/yconn.1.md OPTIONS section must remove `--layer` and `--no-color` entries and add per-subcommand flag documentation for `ssh-config` and `user` commands; the `users:` YAML section in docs must clearly distinguish `${key}` named-entry expansion from `${user}` env-var expansion and document the precedence (named map lookup first, then env-var for `${user}`); examples must note that `yconn show` does NOT expand templates — raw value is displayed

- [x] **Display username in `yconn user show` output** [cli] S
  - Acceptance: `yconn user show` prints a `Username:` header line above the users table resolved by: (1) the value of the `user` key in the merged `users:` map if present, (2) the `$USER` environment variable if not in the map, or (3) an empty string if neither is available; the header is always printed (even when empty), separated from the table by a blank line; unit tests in `src/commands/user.rs` cover: `user` key present in users map uses map value, `user` key absent and `$USER` set uses env var, both absent shows empty string; functional test in `tests/functional.rs` runs `yconn user show` with a config containing `users: user: "alice"` and asserts stdout contains `Username: alice`; a second functional test runs with no `users:` map and `USER=bob` in the environment and asserts stdout contains `Username: bob`; `make test` passes
  - Depends on: Update all documentation to reflect the current state of yconn
  - Modify: src/commands/user.rs, src/display/mod.rs, tests/functional.rs
  - Create: none
  - Reuse: src/commands/user.rs:show (extend to resolve username before calling renderer), src/config/mod.rs:LoadedConfig::users (active users map — check for key `"user"`), src/config/mod.rs:expand_user_field (already resolves `${user}` via `$USER` env var — reuse its `$USER` lookup logic rather than duplicating it), src/display/mod.rs:Renderer::user_list (add a `username` parameter or add a new `print_username_header` method that prints the header line before the table), tests/functional.rs:user_show_lists_entries_with_source (follow structure for new functional tests), tests/functional.rs:TestEnv (set USER env var via env overrides for the env-var test case)
  - Risks: the `user` key in the `users:` map is a named lookup, not the `${user}` env-var expansion — the resolution must check `cfg.users.get("user")` directly (the map value), not call `expand_user_field`; `expand_user_field` handles template substitution in connection fields, which is a different code path — do not conflate; the `$USER` fallback should use `std::env::var("USER")` directly, same as the existing fallback in `expand_user_field`; `Renderer::user_list` currently takes `&[UserEntry]` — adding a `username: &str` parameter changes its signature and all call sites in tests must be updated, or alternatively add a separate `print_username_header` method called before `user_list`; the `user` entry in the map (if present) will also appear as a row in the table — that is correct and expected, the header and the table row are independent; TestEnv may need an explicit `USER` env var injection for the functional test since `$USER` is inherited from the test process environment and may already be set

- [x] **Rename `yconn user` command to `yconn users`** [cli] S
  - Acceptance: `yconn users show`, `yconn users add`, and `yconn users edit <key>` all work identically to the old `yconn user` variants; `yconn user` (old spelling) produces a clap "unrecognized subcommand" error; all references to the old command name are updated in source, tests, and docs; `make test` passes
  - Depends on: Display username in `yconn user show` output
  - Modify: src/cli/mod.rs, src/main.rs, src/commands/user.rs, src/display/mod.rs, tests/functional.rs, README.md, docs/configuration.md, docs/examples.md, docs/man/yconn.1.md
  - Create: none
  - Reuse: src/cli/mod.rs:Commands (rename `User` variant to `Users`), src/cli/mod.rs:UserCommands (no structural change — variant name stays, only the CLI-facing command name changes via clap's `name` attribute or enum rename), src/main.rs:Commands::User match arm (rename to Commands::Users)
  - Risks: the Rust enum variant `Commands::User` and `UserCommands` type name must be kept or renamed consistently — if renamed to `Commands::Users` every match arm and import in main.rs must be updated; clap derives the CLI name from the variant name by default (lowercased), so renaming the variant to `Users` automatically changes the CLI token to `users` with no extra annotation needed; doc comments inside `src/commands/user.rs` that reference `yconn user` must be updated to `yconn users` to avoid misleading future readers; functional tests that invoke the binary with `["user", "show"]` etc. must be updated to `["users", "show"]`; no YAML config format changes — only the CLI surface changes

- [x] **Remove `yconn:` prefix from SSH config comments** [cli] S
  - Acceptance: `render_ssh_config` emits comments as `# description: …`, `# auth: …`, `# link: …`, and `# user: … (unresolved)` — the `yconn: ` substring no longer appears in any generated output; unit tests in `src/commands/ssh_config.rs` are updated so all `assert!(out.contains("# yconn: …"))` assertions become `assert!(out.contains("# …"))` and a negative assertion `assert!(!out.contains("# yconn:"))` is added to the link test; `make test` passes
  - Depends on: Rename `yconn user` command to `yconn users`
  - Modify: src/commands/ssh_config.rs
  - Create: none
  - Reuse: src/commands/ssh_config.rs:render_ssh_config (the four format strings on lines 63–75 are the only change sites)
  - Risks: none — pure string literal change with no logic impact; verify no external tooling parses the `# yconn:` prefix from generated files before stripping it

- [x] **Add `yconn show --dump` flag to print the fully merged config** [cli] S
  - Acceptance: (1) `yconn show --dump` (no connection name required) prints the fully merged `connections:` and `users:` maps to stdout as valid YAML after all layers have been loaded and merged — active entries only, no shadowed rows; (2) the output is machine-readable YAML with a top-level `connections:` key (each entry serialised with all its resolved fields) and a top-level `users:` key (each entry as a flat `key: value` map); (3) `yconn show --dump` and `yconn show <name>` are mutually exclusive — if both a name and `--dump` are supplied clap surfaces an error; (4) the `--dump` flag is only valid on the `show` subcommand, not global; (5) unit tests in `src/commands/show.rs` cover: dump with connections only, dump with users only, dump with both, dump with empty config; (6) a functional test in `tests/functional.rs` writes a project config with at least two connections and a users map, runs `yconn show --dump`, and asserts stdout is valid YAML containing all connection names and user keys; `make test` passes
  - Depends on: Remove `yconn:` prefix from SSH config comments
  - Modify: src/cli/mod.rs, src/main.rs, src/commands/show.rs, src/display/mod.rs, tests/functional.rs
  - Create: none
  - Reuse: src/cli/mod.rs:Commands::Show (add `dump: bool` field and make `name` an `Option<String>`; use clap `required_unless_present` or a manual guard to require exactly one of name or --dump), src/config/mod.rs:LoadedConfig::connections (iterate to build serialisable connection map), src/config/mod.rs:LoadedConfig::users (iterate `HashMap<String, UserEntry>` to build flat key→value map), src/config/mod.rs:Connection (field names define the YAML key names in dump output), src/display/mod.rs:Renderer (add a new `dump` method that serialises to YAML and prints to stdout), tests/functional.rs:TestEnv (use existing harness — write project config, run binary, assert stdout contains expected YAML fragments)
  - Risks: `name` in `Commands::Show` is currently a required positional `String` — changing it to `Option<String>` requires updating every match arm and call site in `src/main.rs` and all unit tests in `src/commands/show.rs` that call `run(cfg, renderer, name)`; clap's `required_unless_present` or a post-parse guard must enforce that exactly one of name or `--dump` is given — without this, bare `yconn show` gives a confusing error; the YAML serialisation must use `serde_yaml` (already a dependency via config loading) rather than hand-rolling strings — add `#[derive(Serialize)]` to a dump-specific struct or map type rather than modifying the existing `Connection` struct (which uses `Deserialize` only); `yconn show --dump` must not call the template expansion path — dump shows raw config field values identically to `yconn show <name>`; the `users:` section in dump output must be a plain `key: value` map (not the internal `UserEntry` struct with layer/source fields) — build a `HashMap<String, String>` from `cfg.users` before serialising; `connections:` entries in dump must include all fields including optional ones (`key`, `link`, `port`, `group`) — omit fields with `None` or default values using `#[serde(skip_serializing_if)]` to keep output clean

- [x] **Add `-F /dev/null` flag to SSH invocations to bypass `~/.ssh/config`** [core] S
  - Acceptance: `build_args` in `src/connect/mod.rs` inserts `-F /dev/null` as the first flag immediately after `"ssh"` for all connection types (key auth, password auth, default port, custom port); all existing unit tests in `src/connect/mod.rs` and `src/commands/connect.rs` that assert exact SSH arg vectors are updated to include `-F /dev/null` at position 1; a new unit test asserts that `-F /dev/null` is always present regardless of auth type or port; `make test` passes
  - Depends on: Add `yconn show --dump` flag to print the fully merged config
  - Modify: src/connect/mod.rs, src/commands/connect.rs, tests/functional.rs
  - Create: none
  - Reuse: src/connect/mod.rs:build_args (insert `-F` and `/dev/null` after the initial `"ssh"` push, before any other flag), src/commands/connect.rs:test_connect_key_auth_default_port_ssh_args (update expected vec), src/commands/connect.rs:test_connect_password_auth_ssh_args (update expected vec)
  - Risks: every call site that assembles or asserts exact SSH arg vectors must be updated — grep for `vec!["ssh"` and `assert_eq!(args` across all test files before submitting; the stderr "Connecting:" line printed by `renderer.print_connecting` will now include `-F /dev/null` — any functional tests that assert the exact connecting-line format must be updated; `-F /dev/null` suppresses `~/.ssh/config` entirely, meaning any `Include`, `IdentityFile`, `ServerAliveInterval`, or other user config directives will be ignored — document this trade-off clearly in a code comment in `build_args`; the `ssh-config` subcommand generates Host blocks for use inside `~/.ssh/config` and does not call `build_args` — it is unaffected by this change
