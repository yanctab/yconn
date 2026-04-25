// src/commands/keys.rs
// Handler for `yconn keys list|setup` — audit and generate SSH keys using
// connection `auth.generate_key` commands.
//
// `keys list` prints a table of every connection that has `generate_key`
// configured. Connections without `generate_key` are omitted entirely.
//
// `keys setup` executes the `${key}`-expanded `generate_key` command for the
// named connection (or all qualifying connections when no name is supplied),
// printing the command that was run and confirmation that the key was
// written.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, bail, Result};

use crate::config::{Connection, LoadedConfig};
use crate::display::{KeyRow, Renderer};

// ─── Public entry points ─────────────────────────────────────────────────────

/// Render the `keys list` table.
pub fn list(cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let rows = build_key_rows(cfg);
    renderer.keys_list(&rows);
    Ok(())
}

/// Run `generate_key` for one connection (by name) or every connection that
/// has a `generate_key` configured when `name` is `None`.
///
/// Semantics:
/// - `name = Some(<n>)`: if `<n>` has no `generate_key` the command aborts
///   non-zero; if `<n>` does not match any connection the command aborts
///   non-zero.
/// - `name = None`: every connection is scanned; connections without
///   `generate_key` are silently skipped. A failure in one connection does
///   not abort the loop — subsequent connections are still processed.
pub fn setup(cfg: &LoadedConfig, renderer: &Renderer, name: Option<&str>) -> Result<()> {
    run_setup(cfg, renderer, name)
}

// ─── Testable implementation ─────────────────────────────────────────────────

/// Build the rows for `keys list` — one per connection that has a
/// `generate_key` configured. Connections without `generate_key` (including
/// all password-auth connections) are omitted entirely.
pub(crate) fn build_key_rows(cfg: &LoadedConfig) -> Vec<KeyRow> {
    let mut rows = Vec::new();
    for conn in &cfg.connections {
        if conn.auth.generate_key().is_none() {
            continue;
        }
        let Some(key_path) = conn.auth.key() else {
            // generate_key without key (should not happen: Password has no
            // generate_key field) — skip defensively.
            continue;
        };
        let generate_key = conn.auth.generate_key_expanded().unwrap_or_default();
        rows.push(KeyRow {
            name: conn.name.clone(),
            key: key_path.to_string(),
            generate_key,
            layer: conn.layer.label().to_string(),
            source_path: conn.source_path.display().to_string(),
        });
    }
    rows
}

/// Core setup dispatcher — branches on whether a connection name was
/// supplied. The two forms have intentionally different error semantics:
/// single-name is strict (missing `generate_key` aborts), iterate-all is
/// lenient (missing `generate_key` is silently skipped).
pub(crate) fn run_setup(cfg: &LoadedConfig, renderer: &Renderer, name: Option<&str>) -> Result<()> {
    match name {
        Some(target) => run_setup_named(cfg, renderer, target),
        None => {
            run_setup_all(cfg, renderer);
            Ok(())
        }
    }
}

/// Strict single-connection form: missing `generate_key` or unknown name
/// aborts non-zero.
fn run_setup_named(cfg: &LoadedConfig, renderer: &Renderer, name: &str) -> Result<()> {
    let conn = cfg
        .find(name)
        .ok_or_else(|| anyhow!("unknown connection '{name}'"))?;

    if conn.auth.generate_key().is_none() {
        bail!("connection '{name}' has no generate_key configured");
    }

    process_connection(conn, renderer)
}

/// Lenient iterate-all form: silently skip connections without
/// `generate_key`; continue past individual failures so one bad entry does
/// not block the rest.
fn run_setup_all(cfg: &LoadedConfig, renderer: &Renderer) {
    for conn in &cfg.connections {
        if conn.auth.generate_key().is_none() {
            continue;
        }
        if let Err(err) = process_connection(conn, renderer) {
            renderer.error(&err.to_string());
        }
    }
}

/// Execute the expanded `generate_key` command for a single connection.
///
/// Returns `Err` when:
/// - the connection's `auth.key` is missing (defensive — every variant with
///   `generate_key` also has `key`)
/// - the shell command exits non-zero
///
/// When the key file already exists on disk, the command is skipped and a
/// `Skipping` message is printed.
fn process_connection(conn: &Connection, renderer: &Renderer) -> Result<()> {
    let Some(key_path) = conn.auth.key() else {
        bail!(
            "connection '{}' has no key path configured; cannot run generate_key",
            conn.name
        );
    };

    let expanded_path = expand_tilde(key_path);
    if expanded_path.exists() {
        renderer.print_line(&format!(
            "Skipping {}: {} already exists",
            conn.name, key_path
        ));
        return Ok(());
    }

    let Some(expanded_cmd) = conn.auth.generate_key_expanded() else {
        // Should never happen: caller guarantees generate_key is set.
        bail!("connection '{}' has no generate_key configured", conn.name);
    };

    if let Some(parent) = expanded_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                anyhow!(
                    "failed to create parent directory {} for {}: {e}",
                    parent.display(),
                    conn.name
                )
            })?;
        }
    }

    renderer.print_line(&conn.name);
    renderer.print_line(&expanded_cmd);

    let status = Command::new("sh")
        .arg("-c")
        .arg(&expanded_cmd)
        .status()
        .map_err(|e| anyhow!("failed to spawn shell for generate_key: {e}"))?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        bail!("keys setup {} failed (exit code {code})", conn.name);
    }

    renderer.print_line(&format!("Key written to: {}", key_path));
    Ok(())
}

/// Expand a leading `~` to the current user's home directory.
///
/// Only a literal leading `~/` (or the bare string `"~"`) is expanded.
/// `~username` forms are not supported. If `dirs::home_dir()` returns
/// `None`, the path is returned unchanged.
fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    use crate::config;
    use crate::display::Renderer;

    fn write_yaml(dir: &std::path::Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    fn no_color() -> Renderer {
        Renderer::new(false)
    }

    fn load(
        cwd: &std::path::Path,
        user: Option<&std::path::Path>,
        sys: &std::path::Path,
    ) -> config::LoadedConfig {
        config::load_impl(cwd, Some("connections"), false, user, sys).unwrap()
    }

    // ── build_key_rows ────────────────────────────────────────────────────────

    #[test]
    fn test_build_key_rows_filters_out_password_auth() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  db:\n    host: db.internal\n    user: dbadmin\n    auth:\n      type: password\n    description: db\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let rows = build_key_rows(&cfg);
        assert!(rows.is_empty(), "password auth must be filtered out");
    }

    #[test]
    fn test_build_key_rows_filters_out_key_auth_without_generate_key() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: srv\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let rows = build_key_rows(&cfg);
        assert!(
            rows.is_empty(),
            "key auth without generate_key must be filtered out"
        );
    }

    #[test]
    fn test_build_key_rows_emits_row_for_connections_with_generate_key() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  bastion:\n    host: 10.0.0.1\n    user: ec2-user\n    auth:\n      type: key\n      key: ~/.ssh/bastion_key\n      generate_key: \"vault read ssh/bastion > ${key}\"\n    description: Bastion\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let rows = build_key_rows(&cfg);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "bastion");
        assert_eq!(rows[0].key, "~/.ssh/bastion_key");
        assert_eq!(
            rows[0].generate_key, "vault read ssh/bastion > ~/.ssh/bastion_key",
            "GENERATE_KEY column must contain the ${{key}}-expanded command"
        );
        assert_eq!(rows[0].layer, "project");
    }

    #[test]
    fn test_build_key_rows_empty_when_no_qualifying_connections() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        let rows = build_key_rows(&cfg);
        assert!(rows.is_empty());
    }

    #[test]
    fn test_build_key_rows_identity_auth_with_generate_key_included() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  github:\n    host: github.com\n    user: git\n    auth:\n      type: identity\n      key: ~/.ssh/github_key\n      generate_key: \"op read secret > ${key}\"\n    description: github\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let rows = build_key_rows(&cfg);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "github");
        assert_eq!(rows[0].generate_key, "op read secret > ~/.ssh/github_key");
    }

    // ── run_setup named connection — error semantics ─────────────────────────

    #[test]
    fn test_run_setup_named_without_generate_key_returns_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: srv\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let err = run_setup(&cfg, &no_color(), Some("srv")).unwrap_err();
        assert!(
            err.to_string().contains("has no generate_key configured"),
            "error should mention missing generate_key, got: {err}"
        );
    }

    #[test]
    fn test_run_setup_unknown_name_returns_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        let err = run_setup(&cfg, &no_color(), Some("nope")).unwrap_err();
        assert!(
            err.to_string().contains("unknown connection"),
            "error should mention 'unknown connection', got: {err}"
        );
        assert!(
            err.to_string().contains("nope"),
            "error should include the missing name: {err}"
        );
    }

    #[test]
    fn test_run_setup_iterate_all_silently_skips_without_generate_key() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        // Only password auth and key auth without generate_key — nothing to do.
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  db:\n    host: db.internal\n    user: dbadmin\n    auth:\n      type: password\n    description: db\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: srv\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        // Iterate-all returns Ok() with no output and no error — nothing
        // qualifies, so there is nothing to do.
        run_setup(&cfg, &no_color(), None).unwrap();
    }

    // ── process_connection: key file already exists → skip ───────────────────

    #[test]
    fn test_process_connection_skips_when_key_file_exists() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        // Create the target key file first.
        let existing_key = root.path().join("existing_key");
        fs::write(&existing_key, "pretend key").unwrap();

        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {}\n      generate_key: \"echo fail > ${{key}}\"\n    description: srv\n",
            existing_key.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        // The command must NOT be executed (which would write "fail" over the
        // existing content); instead the function returns Ok and the original
        // contents remain.
        run_setup(&cfg, &no_color(), Some("srv")).unwrap();
        let contents = fs::read_to_string(&existing_key).unwrap();
        assert_eq!(
            contents, "pretend key",
            "existing key file must not be overwritten"
        );
    }

    // ── process_connection: successful command writes the file ───────────────

    #[test]
    fn test_process_connection_success_writes_key_file() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let key_path = root.path().join("new_key");
        // Do NOT pre-create — expansion into $key must create the file via sh.

        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {}\n      generate_key: \"printf %s hello > ${{key}}\"\n    description: srv\n",
            key_path.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        run_setup(&cfg, &no_color(), Some("srv")).unwrap();
        let contents = fs::read_to_string(&key_path).unwrap();
        assert_eq!(contents, "hello");
    }

    // ── process_connection: failing command returns error (named mode) ───────

    #[test]
    fn test_process_connection_named_failing_command_returns_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let key_path = root.path().join("key_that_will_not_exist");

        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {}\n      generate_key: \"false\"\n    description: srv\n",
            key_path.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let err = run_setup(&cfg, &no_color(), Some("srv")).unwrap_err();
        assert!(
            err.to_string().contains("keys setup srv failed"),
            "error message should mention failure with exit code, got: {err}"
        );
    }

    // ── process_connection: iterate-all continues past failure ───────────────

    #[test]
    fn test_iterate_all_continues_past_failure() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let ok_key = root.path().join("ok_key");
        let fail_key = root.path().join("fail_key");

        // Two connections: alpha fails (exit 1), beta succeeds (writes
        // "done"). Iterate-all must continue past alpha's failure and still
        // produce beta's key file.
        let cfg_yaml = format!(
            "connections:\n  alpha:\n    host: 1.1.1.1\n    user: u\n    auth:\n      type: key\n      key: {fail}\n      generate_key: \"false\"\n    description: a\n  beta:\n    host: 2.2.2.2\n    user: u\n    auth:\n      type: key\n      key: {ok}\n      generate_key: \"printf %s done > ${{key}}\"\n    description: b\n",
            fail = fail_key.display(),
            ok = ok_key.display(),
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        run_setup(&cfg, &no_color(), None).unwrap();
        let contents = fs::read_to_string(&ok_key).expect("beta key must be produced");
        assert_eq!(contents, "done");
        assert!(
            !fail_key.exists(),
            "alpha key must not be created when its command failed"
        );
    }

    // ── list renders without panicking ───────────────────────────────────────

    #[test]
    fn test_list_runs_without_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n      generate_key: \"echo > ${key}\"\n    description: srv\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        list(&cfg, &no_color()).unwrap();
    }

    // ── parent-directory creation ────────────────────────────────────────────

    #[test]
    fn test_run_setup_named_creates_missing_parent_directory() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        // Target key path nested under a parent that does not exist yet.
        let parent = root.path().join("nested").join("dir");
        let key_path = parent.join("new_key");
        assert!(
            !parent.exists(),
            "test precondition: parent dir must not exist"
        );

        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {}\n      generate_key: \"printf %s hello > ${{key}}\"\n    description: srv\n",
            key_path.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        run_setup(&cfg, &no_color(), Some("srv")).unwrap();

        assert!(
            parent.is_dir(),
            "parent directory must be created before the shell command runs"
        );
        let contents = fs::read_to_string(&key_path)
            .expect("key file must be written into the newly-created parent");
        assert_eq!(contents, "hello");
    }

    // ── expand_tilde ──────────────────────────────────────────────────────────

    #[test]
    fn test_expand_tilde_prefix_joins_home() {
        let result = expand_tilde("~/foo");
        let home = dirs::home_dir().expect("home dir must be set in test environment");
        assert_eq!(result, home.join("foo"));
    }

    #[test]
    fn test_expand_tilde_absolute_unchanged() {
        let result = expand_tilde("/etc/passwd");
        assert_eq!(result, PathBuf::from("/etc/passwd"));
    }
}
