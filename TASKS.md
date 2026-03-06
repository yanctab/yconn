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

- [x] **Implement config loading and merge** [core] L
  - Acceptance: loads all three layers in priority order, upward walk finds project config, merge resolves name collisions correctly, shadowed entries retained; unit tests cover all merge scenarios; `make test` passes
  - Depends on: Implement display module

- [x] **Implement group module** [core] M
  - Acceptance: reads/writes `session.yml`, resolves active group, scans layers for available groups; unit tests cover all group scenarios from the testing strategy; `make test` passes
  - Depends on: Implement config loading and merge

- [x] **Implement list command** [cli] S
  - Acceptance: `yconn list` renders all active connections; `yconn list --all` includes shadowed entries; unit and functional tests pass; `make test` passes
  - Depends on: Implement group module

- [x] **Implement show command** [cli] S
  - Acceptance: `yconn show <name>` renders all non-secret fields for a named connection; exits non-zero with clear error if name not found; unit and functional tests pass; `make test` passes
  - Depends on: Implement list command

- [x] **Implement config command** [cli] S
  - Acceptance: `yconn config` renders active group, layer paths, connection counts, and Docker status; unit and functional tests pass; `make test` passes
  - Depends on: Implement show command

- [x] **Implement group subcommands** [cli] M
  - Acceptance: `yconn group list`, `yconn group use <name>`, `yconn group clear`, `yconn group current` all behave as specified; session.yml is written correctly; unit and functional tests pass; `make test` passes
  - Depends on: Implement config command

- [x] **Implement connect command with Docker bootstrap** [core] L
  - Acceptance: `yconn connect <name>` builds correct SSH args for key and password auth; Docker bootstrap path correctly invokes `docker run` with all required mounts; container detection works via `CONN_IN_DOCKER` env var and `/.dockerenv`; all SSH and Docker functional test scenarios from the testing strategy pass; `make test` passes
  - Depends on: Implement group subcommands

- [x] **Implement add, edit, remove, init commands** [cli] M
  - Acceptance: `yconn add` wizard writes valid YAML; `yconn edit <name>` opens correct file in `$EDITOR`; `yconn remove <name>` removes the entry cleanly; `yconn init` scaffolds a valid config file; unit and functional tests cover each command; `make test` passes
  - Depends on: Implement connect command with Docker bootstrap

- [x] **Implement security module** [core] M
  - Acceptance: file permission warnings emitted for world-readable config files; credential field detection warns on git-trackable layers; docker block in user config emits warning; all checks are non-blocking; unit tests cover all security check scenarios; `make test` passes
  - Depends on: Implement add, edit, remove, init commands

- [x] **Auto-install system dependencies in make setup using distro detection** [core] S
  - Acceptance: `make setup` detects the Linux distro (Debian/Ubuntu vs Arch vs other) and installs `musl-tools` (or equivalent) and `openssh-client` without prompting; on an unsupported distro it prints a clear message listing the packages needed and exits 0 rather than failing; `make build` succeeds immediately after `make setup` on a clean Debian/Ubuntu or Arch environment; the Makefile change is covered by a comment explaining the detection logic; `make test` still passes
  - Depends on: Implement security module

- [x] **Fix YAML indentation bug in `yconn add` and add functional round-trip tests** [core] S
  - Acceptance: `yconn add` writes a valid, parseable YAML entry (connection name at 2-space indent, fields at 4-space indent under `connections:`); `yconn show <name>` succeeds immediately after add without errors; `yconn edit <name>` is covered by a functional test using a mock `$EDITOR` that verifies the target file path is correct and the file remains parseable after the editor exits; functional tests in `tests/functional.rs` cover add round-trip (add → list/show verifies entry) and edit invocation; all pass with `make test`
  - Depends on: Auto-install system dependencies in make setup using distro detection
  - Modify: src/commands/add.rs, tests/functional.rs
  - Create: none
  - Reuse: tests/functional.rs:TestEnv (controlled temp environment), tests/functional.rs:conn_key/conn_password (YAML helpers), src/commands/add.rs:run_with_input (input simulation pattern), src/commands/remove.rs:remove_entry (indentation detection pattern)
  - Risks: fixing build_entry() to 4-space field indentation will break existing unit tests in add.rs that assert the current (buggy) 2-space format — those tests must be updated too; verify remove_entry() indent boundary detection (≤2 check) still works after the fix; edit opens $EDITOR so functional test must inject a mock editor script via PATH (same pattern as mock ssh in TestEnv)

- [x] **Set 0o600 permissions on config files created by yconn** [core] S
  - Acceptance: files created by `yconn add`, `yconn init`, and `yconn group use`/`yconn group clear` are written with 0o600 permissions; running `yconn add` followed immediately by any `yconn` command that loads config produces no world-readable permission warning for the newly created file; unit tests in each affected module assert the written file has mode 0o600; all existing security module tests continue to pass with `make test`
  - Depends on: Fix YAML indentation bug in `yconn add` and add functional round-trip tests
  - Modify: src/commands/add.rs, src/commands/init.rs, src/group/mod.rs
  - Create: none
  - Reuse: src/security/mod.rs:file_mode (permission mode reading helper), tests/functional.rs:PermissionsExt+fs::set_permissions (existing pattern for mode assertions), src/security/mod.rs:make_file (test helper pattern for mode verification)
  - Risks: std::fs::write() does not accept a mode — use fs::set_permissions() immediately after each write (small TOCTOU window, acceptable for config files) or switch to OpenOptions with .mode(0o600); PermissionsExt::mode() is Unix-only — wrap permission-setting code in #[cfg(unix)] to keep non-Unix builds compiling; init.rs writes to the project layer (.yconn/) which is git-tracked and may legitimately be group-readable — decide whether 0o600 is correct there or whether project-layer files should use a less restrictive mode (e.g. 0o644)

- [x] **Expand `~` in key paths read from config** [core] S
  - Acceptance: a `key: ~/...` value in any config layer has its leading `~` replaced with the real home directory before being passed to SSH (`-i` flag) or displayed in `yconn show`; `yconn show` output for a key-auth connection displays the expanded path; the SSH args functional tests that use key auth receive an expanded path (no literal `~`); unit tests in `src/config/mod.rs` or `src/commands/show.rs` cover: `~/foo` expands to `<home>/foo`, a path without `~` is unchanged, an empty key string is unchanged; `make test` passes
  - Depends on: Set 0o600 permissions on config files created by yconn
  - Modify: src/config/mod.rs, tests/functional.rs
  - Create: none
  - Reuse: dirs::home_dir() (already in Cargo.toml via the dirs crate), src/config/mod.rs:RawConn (key field is the expansion site), tests/functional.rs:TestEnv (HOME is already overridden — use that value as the expected expansion target)
  - Risks: only expand a leading `~` (i.e. `~/...`) — do not attempt `~user` expansion; if `dirs::home_dir()` returns None, skip expansion and let the path pass through unchanged rather than panicking; consider also expanding in `src/connect/mod.rs:build_args()` so verbose output shows the real path — but note that existing unit tests there assert tilde paths appear verbatim in SSH args and would need updating if expansion is added

- [x] **Wildcard pattern matching for connection names with conflict detection** [core] M
  - Acceptance: (1) connection entries may use glob-style wildcards in their YAML key (e.g. `host*`, `*-prod`, `web-*-db`); `yconn connect <input>` matches the input against all known connection names/patterns and uses the matched input directly as the SSH hostname (no substitution — the matched input IS the host); (2) when exactly one pattern matches the input, connection proceeds normally using the matched entry's other fields (user, port, auth, key) with `host` replaced by the literal input; (3) when two or more *different* patterns from any layer both match the same input, `yconn` exits non-zero with a clear error naming each conflicting pattern and its source layer and file; (4) same-pattern shadowing across layers follows existing priority rules and does not trigger conflict detection; (5) exact-name entries are tried before pattern matching — an exact match always wins with no conflict check; (6) unit tests in `src/config/mod.rs` cover: single pattern matches input, no pattern matches input, two patterns conflict, exact name beats pattern, same pattern in two layers is shadowing not conflict; (7) functional tests in `tests/functional.rs` cover: `yconn connect` with a wildcard pattern match (mock ssh receives correct user@input-hostname args), `yconn connect` with a conflict exits non-zero with both pattern names in stderr; `make test` passes
  - Depends on: Implement connect command with Docker bootstrap
  - Modify: src/config/mod.rs, src/commands/connect.rs, Cargo.toml
  - Create: none
  - Reuse: src/config/mod.rs:LoadedConfig::find (extend to fall through to pattern scan after exact miss), src/config/mod.rs:Connection (reuse as-is; host field overridden with matched input at call site), src/config/mod.rs:merge_connections (all connections iterated for pattern scan), src/commands/connect.rs:plan (test helper pattern for asserting SSH args without exec), tests/functional.rs:TestEnv (controlled temp environment for functional tests), tests/functional.rs:conn_key/conn_password (YAML fixture helpers)
  - Risks: no glob crate is currently in Cargo.toml — add `wildmatch` or `glob` crate (wildmatch is lighter and has no path-separator semantics that would interfere with connection name patterns); conflict detection must compare pattern strings, not matched results, so two entries with identical patterns in different layers are shadowing (one wins by priority), not a conflict; the `host` field override at the call site in commands/connect.rs changes how Connection is used — verify docker bootstrap path also receives the input-as-host; `yconn list` output for wildcard entries should show the pattern name as-is (not expanded) — no changes to list rendering needed

- [x] **Remove the LINK column from `yconn list` output** [display] S
  - Acceptance: `yconn list` and `yconn list --all` no longer show a LINK column or any link URLs; `yconn show <name>` still renders the Link: line when the field is set; `link: Option<String>` is removed from `ConnectionRow` in `src/display/mod.rs` and the `show_link` branch, `LINK_HEADER`, `LINK_MAX`, and `truncate_link()` are deleted from `render_list()`; the `link` field mapping is removed from `src/commands/list.rs`; all link-related unit tests in `src/display/mod.rs` (`test_list_link_*`) are deleted; the two functional tests in `tests/functional.rs` that assert on the LINK column (`list_shows_link_column_when_connection_has_link`, `list_omits_link_column_when_no_connection_has_link`) and the `conn_key_with_link` helper are removed; `make test` passes
  - Depends on: Wildcard pattern matching for connection names with conflict detection
  - Modify: src/display/mod.rs, src/commands/list.rs, tests/functional.rs
  - Create: none
  - Reuse: src/display/mod.rs:ConnectionDetail (link field and render_show remain untouched — link stays in show output), src/display/mod.rs:pad (column padding helper, unaffected)
  - Risks: col[] array in render_list() is sized [usize; 8] after the "Show link field" task — shrink back to [usize; 7] and verify all index references are updated; removing link from ConnectionRow is a breaking change to the struct — all construction sites (commands/list.rs) must be updated or the build will fail; confirm no other callers construct ConnectionRow with a link field

- [x] **Clean up `yconn list`: remove `--layer` and global `--no-color`, add `--group <name>` filter** [cli] S
  - Acceptance: `yconn list --help` no longer shows `--layer`; `--layer` still appears in `yconn add --help`, `yconn edit --help`, and `yconn remove --help` and works correctly via `LayerArg`; `--no-color` is removed from `Cli` entirely and from `--help` output; `yconn list --group <name>` filters output to connections whose `group` field equals `<name>`, applying `LoadedConfig::effective_group_filter` with `--all` still overriding everything; `yconn list --group unknown` exits 0 and prints an empty table; unit tests in `src/commands/list.rs` cover: `--group` filter returns only matching connections, `--group` with `--all` shows all connections, `--group` with no matches returns empty list; functional test in `tests/functional.rs` runs `yconn list --group work` and asserts only work-group connections appear in stdout; `make test` passes
  - Depends on: Remove the LINK column from `yconn list` output
  - Modify: src/cli/mod.rs, src/main.rs, src/commands/list.rs, tests/functional.rs
  - Create: none
  - Reuse: src/config/mod.rs:LoadedConfig::effective_group_filter (precedence logic: --all beats --group beats locked group), src/config/mod.rs:LoadedConfig::filtered_connections (the actual per-connection filter), src/cli/mod.rs:LayerArg (move from global Cli to Add/Edit/Remove subcommand fields — same ValueEnum, new attachment point), tests/functional.rs:TestEnv (functional test infrastructure), src/config/mod.rs:conn_with_group (YAML fixture helper for group-tagged connections)
  - Risks: moving `--layer` off global `Cli` and onto `Add`/`Edit`/`Remove` subcommand structs changes how `cli.layer` is accessed in `src/main.rs` — each arm of the match must destructure its own `layer` field; removing `no_color` from `Cli` means `Renderer::new(!cli.no_color)` in `main.rs` must change to `Renderer::new(true)` or a TTY-detection heuristic; `--all` remains on `Cli` as a global flag and must still be passed to `list::run()`; `list::run()` signature gains a `group: Option<&str>` parameter — all call sites (main.rs, unit tests) must be updated

- [x] **Update docs and man page: reflect recent changes and extend examples** [docs] M
  - Acceptance: `docs/configuration.md` is updated to replace all per-file group references with the inline `group:` field model (always `connections.yaml`, group is a field on each connection entry), document the `group:` field in the connection fields table, and remove the stale "Groups and filenames" file-mapping table; `docs/man/yconn.1.md` is updated to reflect `yconn init --location [yconn|dotfile|plain]` with the resulting paths for each value, remove the claim "There is no pattern or glob matching" from connection descriptions and document wildcard pattern matching semantics (input IS the host, conflict detection), update `--layer` scope to add/edit/remove only, and update `yconn group list` description to scan connection `group:` field values not files; `docs/examples.md` gains three new scenarios: (1) wildcard pattern usage with a realistic YAML snippet and `yconn connect web-prod-01` style invocation, (2) inline group field usage showing multiple connections tagged with `group: work` or `group: private` in one `connections.yaml` with `yconn group use` and `yconn list` commands, (3) multi-location init showing all three `--location` values and the resulting file paths; existing multi-group example in examples.md is updated to show the inline `group:` field model instead of separate `.yaml` files; `make test` passes (no Rust changes required — docs only)
  - Depends on: Clean up `yconn list`: remove `--layer` and global `--no-color`, add `--group <name>` filter
  - Modify: docs/configuration.md, docs/examples.md, docs/man/yconn.1.md
  - Create: none
  - Reuse: src/cli/mod.rs:InitLocation (exact variant names: Yconn/Dotfile/Plain and resulting paths), src/config/mod.rs:RawConn (group field definition and serde default pattern), src/config/mod.rs:find_with_wildcard (wildcard semantics: exact beats pattern, input IS the host, conflict detection on distinct patterns), src/commands/init.rs:resolve_target (exact file paths per --location value)
  - Risks: docs/configuration.md still describes the old per-file group model in "Groups and filenames" — that entire subsection must be replaced, not just amended; docs/man/yconn.1.md FILES section lists `~/.config/yconn/<group>.yaml` paths which are now always `connections.yaml` — update to reflect single filename; examples.md multi-group scenario currently shows `work.yaml`/`private.yaml` files which no longer exist — the updated scenario must show a single `connections.yaml` with inline `group:` tags; ensure wildcard examples clearly note that the connection name key IS the pattern (e.g. `web-*`) and the matched input becomes the SSH hostname, not that host is substituted from a template

- [x] **Support `${name}` template expansion in `host` field for wildcard connections** [core] S
  - Acceptance: when a wildcard pattern match occurs and the matched connection's `host` field contains the literal `${name}`, `${name}` is replaced with the matched input before the connection is returned; if `host` does not contain `${name}`, behaviour is unchanged (input used directly as host, as today); `yconn show <input>` displays the expanded host when the pattern matches; `yconn list` shows the raw `${name}` template as-is (not expanded); unit tests in `src/config/mod.rs` cover: host with `${name}` is expanded to matched input, host without `${name}` is replaced by matched input (existing behaviour), exact-match connections with `${name}` in host are not expanded; functional test in `tests/functional.rs` covers `yconn connect` with `host: ${name}.corp.com` pattern — mock ssh receives `user@server01.corp.com`; `make test` passes
  - Depends on: Update docs and man page: reflect recent changes and extend examples
  - Modify: src/config/mod.rs, src/commands/show.rs, tests/functional.rs, docs/examples.md, docs/configuration.md, docs/man/yconn.1.md
  - Create: none
  - Reuse: src/config/mod.rs:find_with_wildcard (the single expansion site at line 202 where `resolved.host = input.to_string()` is set — change to conditional replace using `resolved.host.replace("${name}", input)`), src/config/mod.rs:Connection (host field mutated in place), src/commands/connect.rs:plan (test helper for asserting SSH args without exec), tests/functional.rs:wildcard_pattern_match_ssh_receives_input_as_host (follow this test structure for the new functional test)
  - Implementation notes:
    - `src/config/mod.rs` line ~202: `resolved.host = if resolved.host.contains("${name}") { resolved.host.replace("${name}", input) } else { input.to_string() };`
    - `src/commands/show.rs` line ~10-12: replace `cfg.find(name).ok_or_else(|| anyhow!(...))` with `cfg.find_with_wildcard(name)?`
    - New unit tests: `test_wildcard_host_with_name_template_is_expanded`, `test_wildcard_host_without_name_template_replaced_by_input`, `test_wildcard_exact_match_name_template_not_expanded`
    - New functional test: `wildcard_name_template_in_host_expands_to_fqdn` — follow `wildcard_pattern_match_ssh_receives_input_as_host` structure
    - docs/man/yconn.1.md: update wildcard YAML comment (line ~148) and description paragraph (lines ~158-161) to mention `${name}` template
  - Risks: `yconn show <input>` currently calls `cfg.find(name)` which does exact lookup only — it must be switched to `cfg.find_with_wildcard(name)` to support wildcard inputs; this change also means `yconn show` will return the expanded host for any wildcard match, which is the desired behaviour but changes existing show test assumptions; conflict detection still compares pattern names so `${name}` in host has no effect on it; `${name}` is a literal string replacement, not a regex or shell expansion — only the exact four-character sequence `${name}` (dollar, brace, n-a-m-e, brace) is replaced; ensure the replacement only applies in the wildcard match path, not for exact-name lookups; docs/examples.md wildcard scenario and docs/configuration.md wildcard section must be updated to show `host: ${name}.corp.com` as the idiomatic pattern

- [x] **Support numeric range syntax in connection names (e.g. `server[1..10]`) combined with `${name}` host template** [core] M
  - Acceptance: (1) a connection YAML key containing a `[start..end]` numeric range (e.g. `server[1..10]`) matches any input whose name equals the prefix plus an integer in `[start, end]` inclusive (e.g. `server1` through `server10`); (2) range patterns participate in `find_with_wildcard` alongside glob patterns — exact name still wins first, then range patterns are tested, then glob patterns (or interleaved in scan order); (3) when a range pattern matches and the connection's `host` field contains `${name}`, `${name}` is replaced with the matched input exactly as the `${name}` expansion task defines; (4) conflict detection treats a range pattern as a distinct pattern name — if both `server[1..10]` and `server*` match `server5`, `yconn` exits non-zero naming both patterns; (5) `yconn list` shows the raw range pattern key (e.g. `server[1..10]`) unchanged; (6) unit tests in `src/config/mod.rs` cover: range matches lower bound, range matches upper bound, range matches midpoint, input outside range does not match, range conflict with a glob pattern, exact name beats a matching range, same range pattern in two layers is shadowing not conflict, range with `${name}` expands host correctly; (7) functional test in `tests/functional.rs` covers `yconn connect server5` against a `server[1..10]` entry with `host: ${name}.corp.com` — mock ssh receives `deploy@server5.corp.com`; (8) `docs/configuration.md` wildcard section, `docs/examples.md`, and `docs/man/yconn.1.md` are updated to document range syntax with a worked example; `make test` passes
  - Depends on: Support `${name}` template expansion in `host` field for wildcard connections
  - Modify: src/config/mod.rs, tests/functional.rs, docs/configuration.md, docs/examples.md, docs/man/yconn.1.md
  - Create: none
  - Reuse: src/config/mod.rs:find_with_wildcard (extend the pattern-scan loop to detect `[N..M]` syntax and evaluate the range predicate before falling through to WildMatch), src/config/mod.rs:Connection (host field mutated in place — same `${name}` replacement already added by the preceding task), tests/functional.rs:wildcard_name_template_in_host_expands_to_fqdn (follow structure for new functional test), tests/functional.rs:wildcard_conflict_exits_nonzero_with_pattern_names_in_stderr (follow structure for range-vs-glob conflict test)
  - Risks: `[N..M]` in a YAML key is valid YAML (it is just a string key) but the `[` character at the start of a value is special in YAML — keys starting with `[` should be quoted in user-facing examples and docs; range parsing must be strict — only match the regex `\[(\d+)\.\.(\d+)\]` at the end of the key name (prefix before `[` is a literal prefix); no external crate needed — parse start/end with `str::parse::<u64>()` and compare; end < start should be treated as an empty range that never matches (emit a warning, do not panic); range patterns use `u64` bounds to avoid overflow on large numeric suffixes; conflict detection must collect both range-matched and glob-matched patterns into the same `matches` vec so the existing conflict error path fires correctly; `yconn list` raw-key display requires no change since `find_with_wildcard` is only called from connect/show, not list

- [ ] **Add `yconn ssh-config generate` command** [cli] M
  - Acceptance: `yconn ssh-config generate` reads all active connections (respecting the active group lock and layer merge identical to `yconn list`) and writes `~/.ssh/yconn-connections` with one `Host` block per non-wildcard/non-range connection; each block renders `HostName`, `User`, `Port` (only if not 22), `IdentityFile` (only if auth=key), and `# yconn: <field>: <value>` comment lines above the block for `description`, `link` (if present), and `auth`; wildcard/range-pattern connection names (containing `*`, `?`, or matching `\[N..M\]`) are skipped with a comment `# yconn: skipped '<name>' — wildcard/range patterns cannot be expressed as static SSH Host blocks`; `~/.ssh/config` is updated idempotently with `Include ~/.ssh/yconn-connections` prepended if absent, or created with just the Include line if it does not exist; a summary line is printed via `renderer` with the count of Host blocks written and the output file path; `--dry-run` flag prints the generated file content and the `~/.ssh/config` change to stdout without writing any files; unit tests in `src/commands/ssh_config.rs` cover: key-auth block format, password-auth block format (no IdentityFile), port-22 omitted, non-22 port included, wildcard name skipped with comment, range-pattern name skipped with comment, idempotent Include injection when line present, Include prepended when absent, new config created when absent; functional test in `tests/functional.rs` runs `yconn ssh-config generate` against a temp config and asserts the output file exists with correct Host block content and `~/.ssh/config` contains the Include line; `make test` passes
  - Depends on: Support numeric range syntax in connection names (e.g. `server[1..10]`) combined with `${name}` host template
  - Modify: src/cli/mod.rs, src/main.rs, src/commands/mod.rs, src/display/mod.rs, tests/functional.rs
  - Create: src/commands/ssh_config.rs
  - Reuse: src/config/mod.rs:LoadedConfig::connections (active merged connection list), src/config/mod.rs:Connection (all fields needed for Host block rendering), src/cli/mod.rs:GroupCommands (nested Subcommand enum pattern to follow for SshConfigCommands), src/display/mod.rs:Renderer::warn (for non-blocking warnings), tests/functional.rs:TestEnv (controlled temp environment with XDG_CONFIG_HOME and HOME overrides)
  - Risks: `~/.ssh/` may not exist on all systems — create it with 0o700 if absent; `~/.ssh/yconn-ssh-config` should be written with 0o600 since it may contain key paths; the Include directive must appear before any Match or Host blocks in `~/.ssh/config` to take effect — prepending is correct but must not insert a duplicate if re-run; wildcard/range detection must match the same logic used in `find_with_wildcard` — extract a shared predicate or re-use the pattern-detection helper to avoid drift; `--dry-run` must not touch the filesystem at all, including `~/.ssh/` directory creation; the `ssh-config` subcommand group currently has only one subcommand (`generate`) but is structured as a group to allow future additions (e.g. `ssh-config clean`, `ssh-config diff`) without a breaking CLI change

- [ ] **Add `yconn ssh-config show` subcommand** [cli] S
  - Acceptance: `yconn ssh-config show` prints to stdout the exact SSH config text that `generate` would write to `~/.ssh/yconn-connections` — same `Host` blocks, same `# yconn:` comment lines, same wildcard-skip comments — without touching any files; output contains no ANSI colour codes regardless of terminal or `--no-color` flag; the core rendering function (building the SSH config string from a connection list) is extracted into a shared helper in `src/commands/ssh_config.rs` so both `generate` and `show` call it without duplication; `generate`'s summary line (block count and output path) is not printed by `show`; unit tests in `src/commands/ssh_config.rs` call the shared rendering helper directly and assert correct output for key-auth, password-auth, custom port, and wildcard-skip cases; a functional test in `tests/functional.rs` runs `yconn ssh-config show` against a temp config and asserts stdout matches the expected Host block text with no trailing summary line and no ANSI escapes; `make test` passes
  - Depends on: Add `yconn ssh-config generate` command
  - Modify: src/cli/mod.rs, src/main.rs, src/commands/ssh_config.rs, tests/functional.rs
  - Create: none
  - Reuse: src/cli/mod.rs:SshConfigCommands (extend the existing nested subcommand enum with a `Show` variant), src/commands/ssh_config.rs:render_ssh_config (shared helper extracted from `generate` — takes `&[Connection]` and returns `String`), src/config/mod.rs:LoadedConfig::connections (same connection list used by `generate`), tests/functional.rs:TestEnv (controlled temp environment for functional tests)
  - Risks: the shared rendering helper must be extracted as part of this task — if `generate` was implemented with the render logic inlined, the refactor must not change `generate`'s output or break its existing tests; stdout output for `show` must bypass `Renderer` styled output entirely (use `print!` or `println!` directly, or a plain-text write path) to guarantee no ANSI codes leak through even when the renderer is constructed with colour enabled; ensure the `SshConfigCommands` enum dispatch in `src/main.rs` correctly routes `Show` to the new handler without touching the `Generate` arm
