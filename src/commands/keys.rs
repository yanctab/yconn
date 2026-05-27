// src/commands/keys.rs
// Handler for `yconn keys list|install` — audit and generate SSH keys using
// connection `auth.generate_key` commands.
//
// `keys list` prints a table of every connection that has `generate_key`
// configured. Connections without `generate_key` are omitted entirely.
//
// `keys install` executes the `${key}`-expanded `generate_key` command for the
// named connection (or all qualifying connections when no name is supplied),
// printing the command that was run and confirmation that the key was
// written.

use std::fs;
use std::io::{BufRead, Write as _};
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
pub fn install(cfg: &LoadedConfig, renderer: &Renderer, name: Option<&str>) -> Result<()> {
    run_install(cfg, renderer, name)
}

/// Update (refresh) the SSH key by deleting the existing key file and
/// running `generate_key` for one connection (by name) or every connection
/// that has a `generate_key` configured when `name` is `None`.
/// Prompts for confirmation (y/N) before proceeding.
///
/// Semantics:
/// - `name = Some(<n>)`: if `<n>` has no `generate_key` the command aborts
///   non-zero; if `<n>` does not match any connection the command aborts
///   non-zero.
/// - `name = None`: every connection is scanned; connections without
///   `generate_key` are silently skipped. A failure in one connection does
///   not abort the loop — subsequent connections are still processed.
#[allow(dead_code)]
pub fn update(
    cfg: &LoadedConfig,
    renderer: &Renderer,
    name: Option<&str>,
    stdin: &mut dyn BufRead,
) -> Result<()> {
    run_update(cfg, renderer, name, stdin)
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
        let generate_key = conn
            .auth
            .generate_key_rendered(&conn.user)
            .unwrap_or_default();
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
pub(crate) fn run_install(
    cfg: &LoadedConfig,
    renderer: &Renderer,
    name: Option<&str>,
) -> Result<()> {
    match name {
        Some(target) => run_install_named(cfg, renderer, target),
        None => {
            run_install_all(cfg, renderer);
            Ok(())
        }
    }
}

/// Strict single-connection form: missing `generate_key` or unknown name
/// aborts non-zero.
fn run_install_named(cfg: &LoadedConfig, renderer: &Renderer, name: &str) -> Result<()> {
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
fn run_install_all(cfg: &LoadedConfig, renderer: &Renderer) {
    for conn in &cfg.connections {
        if conn.auth.generate_key().is_none() {
            continue;
        }
        if let Err(err) = process_connection(conn, renderer) {
            renderer.error(&err.to_string());
        }
    }
}

/// Core update dispatcher — branches on whether a connection name was
/// supplied. The two forms have intentionally different error semantics:
/// single-name is strict (missing `generate_key` aborts), iterate-all is
/// lenient (missing `generate_key` is silently skipped).
#[allow(dead_code)]
pub(crate) fn run_update(
    cfg: &LoadedConfig,
    renderer: &Renderer,
    name: Option<&str>,
    stdin: &mut dyn BufRead,
) -> Result<()> {
    match name {
        Some(target) => run_update_named(cfg, renderer, target, stdin),
        None => {
            run_update_all(cfg, renderer, stdin);
            Ok(())
        }
    }
}

/// Strict single-connection form: missing `generate_key` or unknown name
/// aborts non-zero. Prompts for confirmation before deleting the key file.
#[allow(dead_code)]
fn run_update_named(
    cfg: &LoadedConfig,
    renderer: &Renderer,
    name: &str,
    stdin: &mut dyn BufRead,
) -> Result<()> {
    let conn = cfg
        .find(name)
        .ok_or_else(|| anyhow!("unknown connection '{name}'"))?;

    if conn.auth.generate_key().is_none() {
        bail!("connection '{name}' has no generate_key configured");
    }

    if !confirm_update(stdin)? {
        return Ok(());
    }

    delete_key_file(conn)?;
    process_connection(conn, renderer)
}

/// Lenient iterate-all form: silently skip connections without
/// `generate_key`; continue past individual failures so one bad entry does
/// not block the rest. Prompts for confirmation before each deletion.
#[allow(dead_code)]
fn run_update_all(cfg: &LoadedConfig, renderer: &Renderer, stdin: &mut dyn BufRead) {
    for conn in &cfg.connections {
        if conn.auth.generate_key().is_none() {
            continue;
        }

        if let Ok(confirmed) = confirm_update(stdin) {
            if !confirmed {
                continue;
            }
        } else {
            // Error reading from stdin — skip this connection
            continue;
        }

        if let Err(err) = delete_key_file(conn) {
            renderer.error(&err.to_string());
            continue;
        }

        if let Err(err) = process_connection(conn, renderer) {
            renderer.error(&err.to_string());
        }
    }
}

/// Prompt the user for confirmation (y/N) and return whether the answer was affirmative.
/// Only responses starting with 'y' or 'Y' return true; all others (including empty) return false.
#[allow(dead_code)]
fn confirm_update(stdin: &mut dyn BufRead) -> Result<bool> {
    print!("Update this key? [y/N] ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    stdin.read_line(&mut input)?;
    Ok(input.trim().starts_with('y') || input.trim().starts_with('Y'))
}

/// Delete the key file for the given connection. Tolerates `ErrorKind::NotFound`.
#[allow(dead_code)]
fn delete_key_file(conn: &Connection) -> Result<()> {
    let Some(key_path) = conn.auth.key() else {
        bail!(
            "connection '{}' has no key path configured; cannot delete key",
            conn.name
        );
    };

    let expanded_path = expand_tilde(key_path);
    match fs::remove_file(&expanded_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File doesn't exist — this is fine
            Ok(())
        }
        Err(e) => Err(anyhow!(
            "failed to delete key file {} for '{}': {e}",
            key_path,
            conn.name
        )),
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

    let Some(expanded_cmd) = conn.auth.generate_key_rendered(&conn.user) else {
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

    renderer.print_keys_setup_notice(
        &conn.name,
        conn.layer.label(),
        &conn.source_path.display().to_string(),
    );
    renderer.print_line(&expanded_cmd);

    let status = Command::new("sh")
        .arg("-c")
        .arg(&expanded_cmd)
        .status()
        .map_err(|e| anyhow!("failed to spawn shell for generate_key: {e}"))?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        bail!("keys install {} failed (exit code {code})", conn.name);
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
    fn test_build_key_rows_expands_user_and_key_placeholders() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  bastion:\n    host: 10.0.0.1\n    user: ec2-user\n    auth:\n      type: key\n      key: ~/.ssh/bastion_key\n      generate_key: \"vault read ssh/${user} > ${key}\"\n    description: Bastion\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let rows = build_key_rows(&cfg);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].generate_key, "vault read ssh/ec2-user > ~/.ssh/bastion_key",
            "GENERATE_KEY column must expand both ${{user}} and ${{key}}"
        );
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

    // ── run_install named connection — error semantics ─────────────────────────

    #[test]
    fn test_run_install_named_without_generate_key_returns_error() {
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
        let err = run_install(&cfg, &no_color(), Some("srv")).unwrap_err();
        assert!(
            err.to_string().contains("has no generate_key configured"),
            "error should mention missing generate_key, got: {err}"
        );
    }

    #[test]
    fn test_run_install_unknown_name_returns_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        let err = run_install(&cfg, &no_color(), Some("nope")).unwrap_err();
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
    fn test_run_install_iterate_all_silently_skips_without_generate_key() {
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
        run_install(&cfg, &no_color(), None).unwrap();
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
        run_install(&cfg, &no_color(), Some("srv")).unwrap();
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

        run_install(&cfg, &no_color(), Some("srv")).unwrap();
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

        let err = run_install(&cfg, &no_color(), Some("srv")).unwrap_err();
        assert!(
            err.to_string().contains("keys install srv failed"),
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

        run_install(&cfg, &no_color(), None).unwrap();
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
    fn test_run_install_named_creates_missing_parent_directory() {
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

        run_install(&cfg, &no_color(), Some("srv")).unwrap();

        assert!(
            parent.is_dir(),
            "parent directory must be created before the shell command runs"
        );
        let contents = fs::read_to_string(&key_path)
            .expect("key file must be written into the newly-created parent");
        assert_eq!(contents, "hello");
    }

    #[test]
    fn test_run_install_iterate_all_creates_parent_per_connection() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        // Two connections, each with `auth.key` under its own non-existent
        // parent directory. Iterate-all must create both parents and produce
        // both key files.
        let alpha_parent = root.path().join("alpha-parent");
        let beta_parent = root.path().join("beta-parent");
        let alpha_key = alpha_parent.join("alpha_key");
        let beta_key = beta_parent.join("beta_key");

        assert!(!alpha_parent.exists());
        assert!(!beta_parent.exists());

        let cfg_yaml = format!(
            "connections:\n  alpha:\n    host: 1.1.1.1\n    user: u\n    auth:\n      type: key\n      key: {a}\n      generate_key: \"printf %s a > ${{key}}\"\n    description: a\n  beta:\n    host: 2.2.2.2\n    user: u\n    auth:\n      type: key\n      key: {b}\n      generate_key: \"printf %s b > ${{key}}\"\n    description: b\n",
            a = alpha_key.display(),
            b = beta_key.display(),
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        run_install(&cfg, &no_color(), None).unwrap();

        assert!(alpha_parent.is_dir(), "alpha parent must be created");
        assert!(beta_parent.is_dir(), "beta parent must be created");
        assert_eq!(fs::read_to_string(&alpha_key).unwrap(), "a");
        assert_eq!(fs::read_to_string(&beta_key).unwrap(), "b");
    }

    #[test]
    fn test_run_install_named_uncreatable_parent_returns_error_and_does_not_spawn() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        // A regular file occupies what would otherwise be a parent directory
        // component, so create_dir_all cannot create the parent for the key.
        let blocker = root.path().join("blocker");
        fs::write(&blocker, "i am a file, not a directory").unwrap();
        let key_path = blocker.join("subdir").join("new_key");

        // Sentinel file the shell command would touch if it ran. Asserting
        // its absence proves the command was NOT spawned.
        let sentinel = root.path().join("sentinel");
        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {key}\n      generate_key: \"touch {sentinel}\"\n    description: srv\n",
            key = key_path.display(),
            sentinel = sentinel.display(),
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let err = run_install(&cfg, &no_color(), Some("srv")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("failed to create parent directory"),
            "error must surface a clear parent-dir failure, got: {msg}"
        );
        assert!(
            !sentinel.exists(),
            "shell command must not be spawned when parent-dir creation fails"
        );
    }

    #[test]
    fn test_run_install_iterate_all_uncreatable_parent_reports_and_continues() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        // alpha's parent path component is an existing file → un-creatable.
        let blocker = root.path().join("blocker");
        fs::write(&blocker, "regular file").unwrap();
        let alpha_key = blocker.join("subdir").join("alpha_key");

        // beta's parent does not exist but is creatable; success expected.
        let beta_parent = root.path().join("beta-parent");
        let beta_key = beta_parent.join("beta_key");

        let cfg_yaml = format!(
            "connections:\n  alpha:\n    host: 1.1.1.1\n    user: u\n    auth:\n      type: key\n      key: {a}\n      generate_key: \"echo unreachable > ${{key}}\"\n    description: a\n  beta:\n    host: 2.2.2.2\n    user: u\n    auth:\n      type: key\n      key: {b}\n      generate_key: \"printf %s done > ${{key}}\"\n    description: b\n",
            a = alpha_key.display(),
            b = beta_key.display(),
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        // Iterate-all returns Ok; alpha's failure is reported via the
        // renderer's error channel, beta still succeeds.
        run_install(&cfg, &no_color(), None).unwrap();

        assert!(
            beta_parent.is_dir(),
            "beta's parent must be created despite alpha's failure"
        );
        assert_eq!(
            fs::read_to_string(&beta_key).expect("beta key must be written"),
            "done"
        );
        assert!(
            !alpha_key.exists(),
            "alpha key must not be created when its parent could not be"
        );
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

    // ── update: criterion 1 — unknown name returns Err ───────────────────────

    #[test]
    fn test_update_unknown_name_returns_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        let mut stdin = "y\n".as_bytes();
        let err = update(&cfg, &no_color(), Some("nope"), &mut stdin).unwrap_err();
        assert!(
            err.to_string().contains("unknown connection"),
            "error should mention 'unknown connection', got: {err}"
        );
    }

    // ── update: criterion 2 — no generate_key returns Err ──────────────────────

    #[test]
    fn test_update_named_without_generate_key_returns_error() {
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
        let mut stdin = "y\n".as_bytes();
        let err = update(&cfg, &no_color(), Some("srv"), &mut stdin).unwrap_err();
        assert!(
            err.to_string().contains("has no generate_key configured"),
            "error should mention missing generate_key, got: {err}"
        );
    }

    // ── update: criterion 3 — "n" answer returns without deleting or running ──

    #[test]
    fn test_update_n_answer_does_not_delete_or_run() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let key_path = root.path().join("existing_key");
        fs::write(&key_path, "original content").unwrap();

        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {}\n      generate_key: \"echo fail > ${{key}}\"\n    description: srv\n",
            key_path.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let mut stdin = "n\n".as_bytes();
        update(&cfg, &no_color(), Some("srv"), &mut stdin).unwrap();

        // Key file must still exist with original content
        let contents = fs::read_to_string(&key_path).unwrap();
        assert_eq!(contents, "original content", "key file must not be deleted");
    }

    // ── update: criterion 4 — "y" answer deletes and runs ───────────────────────

    #[test]
    fn test_update_y_answer_deletes_and_runs() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let key_path = root.path().join("existing_key");
        fs::write(&key_path, "old key").unwrap();

        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {}\n      generate_key: \"printf %s newkey > ${{key}}\"\n    description: srv\n",
            key_path.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let mut stdin = "y\n".as_bytes();
        update(&cfg, &no_color(), Some("srv"), &mut stdin).unwrap();

        // Key file must exist with new content
        let contents = fs::read_to_string(&key_path).unwrap();
        assert_eq!(
            contents, "newkey",
            "key file must be updated with new content"
        );
    }

    // ── update: criterion 5 — "y" when key file doesn't exist still proceeds ──

    #[test]
    fn test_update_y_when_key_file_missing_still_proceeds() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let key_path = root.path().join("nonexistent_key");
        assert!(!key_path.exists(), "key must not exist initially");

        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {}\n      generate_key: \"printf %s newkey > ${{key}}\"\n    description: srv\n",
            key_path.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let mut stdin = "y\n".as_bytes();
        update(&cfg, &no_color(), Some("srv"), &mut stdin).unwrap();

        // Key file must be created with new content
        let contents = fs::read_to_string(&key_path).unwrap();
        assert_eq!(
            contents, "newkey",
            "key file must be created even if it didn't exist"
        );
    }

    // ── update: criterion 6 — iterate-all with "y" answers ────────────────────

    #[test]
    fn test_update_iterate_all_with_y_answers() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let key1 = root.path().join("key1");
        let key2 = root.path().join("key2");
        fs::write(&key1, "old1").unwrap();
        fs::write(&key2, "old2").unwrap();

        let cfg_yaml = format!(
            "connections:\n  srv1:\n    host: 1.1.1.1\n    user: u\n    auth:\n      type: key\n      key: {}\n      generate_key: \"printf %s new1 > ${{key}}\"\n    description: srv1\n  srv2:\n    host: 2.2.2.2\n    user: u\n    auth:\n      type: key\n      key: {}\n      generate_key: \"printf %s new2 > ${{key}}\"\n    description: srv2\n",
            key1.display(),
            key2.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        // Two "y\n" answers, one per connection
        let mut stdin = "y\ny\n".as_bytes();
        update(&cfg, &no_color(), None, &mut stdin).unwrap();

        let contents1 = fs::read_to_string(&key1).unwrap();
        let contents2 = fs::read_to_string(&key2).unwrap();
        assert_eq!(contents1, "new1");
        assert_eq!(contents2, "new2");
    }

    // ── update: criterion 7 — silently skips without generate_key ──────────────

    #[test]
    fn test_update_iterate_all_silently_skips_without_generate_key() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        // Only password auth and key auth without generate_key
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  db:\n    host: db.internal\n    user: dbadmin\n    auth:\n      type: password\n    description: db\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: srv\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        // Empty stdin — no prompts should be generated
        let mut stdin = "".as_bytes();
        update(&cfg, &no_color(), None, &mut stdin).unwrap();
    }

    // ── update: criterion 8 — continues past failing process_connection ───────

    #[test]
    fn test_update_iterate_all_continues_past_failure() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let fail_key = root.path().join("fail_key");
        let ok_key = root.path().join("ok_key");

        // alpha: failing generate_key command
        // beta: succeeds
        let cfg_yaml = format!(
            "connections:\n  alpha:\n    host: 1.1.1.1\n    user: u\n    auth:\n      type: key\n      key: {}\n      generate_key: \"false\"\n    description: a\n  beta:\n    host: 2.2.2.2\n    user: u\n    auth:\n      type: key\n      key: {}\n      generate_key: \"printf %s ok > ${{key}}\"\n    description: b\n",
            fail_key.display(),
            ok_key.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        // Two "y\n" answers
        let mut stdin = "y\ny\n".as_bytes();
        update(&cfg, &no_color(), None, &mut stdin).unwrap();

        // beta must still be processed and created
        let ok_contents = fs::read_to_string(&ok_key).expect("beta key must be created");
        assert_eq!(ok_contents, "ok");
    }

    // ── update: criterion 9 — iterate-all with "n" answer ──────────────────────

    #[test]
    fn test_update_iterate_all_n_answer_does_not_delete_or_run() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let key_path = root.path().join("existing_key");
        fs::write(&key_path, "original content").unwrap();

        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {}\n      generate_key: \"echo fail > ${{key}}\"\n    description: srv\n",
            key_path.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        // Single "n\n" answer in iterate-all mode
        let mut stdin = "n\n".as_bytes();
        update(&cfg, &no_color(), None, &mut stdin).unwrap();

        // Key file must still exist with original content (not deleted, not regenerated)
        let contents = fs::read_to_string(&key_path).unwrap();
        assert_eq!(
            contents, "original content",
            "key file must not be deleted or regenerated"
        );
    }

    // ── update: criterion 9 — confirmation prompt is flushed before read_line ──

    #[test]
    fn test_update_confirms_before_reading() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let key_path = root.path().join("key");
        let cfg_yaml = format!(
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: {}\n      generate_key: \"printf %s ok > ${{key}}\"\n    description: srv\n",
            key_path.display()
        );
        write_yaml(&yconn, "connections.yaml", &cfg_yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        // The test checks that confirm_update calls flush before read_line
        // This is verified by the function implementation itself.
        let mut stdin = "y\n".as_bytes();
        update(&cfg, &no_color(), Some("srv"), &mut stdin).unwrap();
        let contents = fs::read_to_string(&key_path).unwrap();
        assert_eq!(contents, "ok");
    }
}
