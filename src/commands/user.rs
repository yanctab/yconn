// Handler for `yconn users show|add|edit` — manage the users: config section.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use crate::cli::LayerArg;
use crate::config::{Layer, LoadedConfig};
use crate::display::{Renderer, UserRow};

// ─── Public entry points ──────────────────────────────────────────────────────

/// `yconn users show` — list all user entries across layers with source and
/// shadowing info.
///
/// When a `user` key is present in the merged `users:` map it appears as a
/// normal row. When it is absent but `$USER` is set, a synthetic row with
/// SOURCE `env (environment variable $USER)` is appended so the effective
/// username is always visible.
pub fn show(cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let rows = build_user_rows(cfg, std::env::var("USER").ok().as_deref());
    renderer.user_list(&rows);
    Ok(())
}

/// Build the [`UserRow`] vec for `yconn users show`.
///
/// Converts `cfg.all_users` to rows, then — when no `user` key exists in
/// `cfg.users` but `env_user` is `Some` — appends a synthetic env-var row.
fn build_user_rows(cfg: &LoadedConfig, env_user: Option<&str>) -> Vec<UserRow> {
    let mut rows: Vec<UserRow> = cfg
        .all_users
        .iter()
        .map(|e| UserRow {
            key: e.key.clone(),
            value: e.value.clone(),
            source: format!("{} ({})", e.layer.label(), e.source_path.display()),
            shadowed: e.shadowed,
        })
        .collect();

    // Append a synthetic env-var row when the `user` key is absent from the
    // active (non-shadowed) users map and $USER is set.
    if cfg.users.get("user").is_none() {
        if let Some(u) = env_user {
            rows.push(UserRow {
                key: "user".to_string(),
                value: u.to_string(),
                source: "env (environment variable $USER)".to_string(),
                shadowed: false,
            });
        }
    }

    rows
}

/// `yconn users add` — interactive wizard to add a user entry to a layer.
pub fn add(layer: Option<LayerArg>) -> Result<()> {
    let target_layer = layer_arg_to_layer(layer);
    let target_dir = layer_path(target_layer)?;

    let stdin = io::stdin();
    let stdout = io::stdout();
    add_impl(
        target_layer,
        &target_dir,
        &mut stdin.lock(),
        &mut stdout.lock(),
    )
}

/// `yconn users edit` — open the source config file for a named user entry in
/// $EDITOR.
pub fn edit(cfg: &LoadedConfig, key: &str, layer: Option<LayerArg>) -> Result<()> {
    let path = resolve_edit_path(cfg, key, layer)?;
    open_editor(&path)
}

// ─── Layer resolution ─────────────────────────────────────────────────────────

fn layer_arg_to_layer(layer: Option<LayerArg>) -> Layer {
    match layer {
        Some(LayerArg::System) => Layer::System,
        Some(LayerArg::Project) => Layer::Project,
        Some(LayerArg::User) | None => Layer::User,
    }
}

fn layer_path(layer: Layer) -> Result<PathBuf> {
    match layer {
        Layer::System => Ok(PathBuf::from("/etc/yconn")),
        Layer::User => {
            let base = dirs::config_dir().context("cannot determine user config directory")?;
            Ok(base.join("yconn"))
        }
        Layer::Project => {
            let cwd = std::env::current_dir().context("cannot determine current directory")?;
            Ok(cwd.join(".yconn"))
        }
    }
}

// ─── Add wizard ───────────────────────────────────────────────────────────────

pub(crate) fn add_impl(
    layer: Layer,
    layer_dir: &Path,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<()> {
    let target = layer_dir.join("connections.yaml");

    writeln!(
        output,
        "Adding user entry to {} layer ({})",
        layer.label(),
        target.display()
    )?;
    writeln!(output, "Leave a field blank to abort.")?;
    writeln!(output)?;

    let key = prompt(output, input, "Key")?;
    if key.is_empty() {
        bail!("aborted");
    }
    let value = prompt(output, input, "Value")?;
    if value.is_empty() {
        bail!("aborted");
    }

    write_user_entry(&target, &key, &value)?;

    writeln!(output)?;
    writeln!(output, "Added user entry '{key}' to {}", target.display())?;

    Ok(())
}

// ─── Edit path resolution ─────────────────────────────────────────────────────

fn resolve_edit_path(cfg: &LoadedConfig, key: &str, layer: Option<LayerArg>) -> Result<PathBuf> {
    if let Some(layer_arg) = layer {
        let target_layer = layer_arg_to_layer(Some(layer_arg));
        // Search all_users for the entry in the specified layer.
        let entry = cfg
            .all_users
            .iter()
            .find(|e| e.key == key && e.layer == target_layer)
            .ok_or_else(|| {
                anyhow!(
                    "no user entry with key '{}' in the {} layer",
                    key,
                    target_layer.label()
                )
            })?;
        Ok(entry.source_path.clone())
    } else {
        // Default: use the active (highest-priority) entry.
        let entry = cfg
            .users
            .get(key)
            .ok_or_else(|| anyhow!("no user entry with key '{key}'"))?;
        Ok(entry.source_path.clone())
    }
}

// ─── YAML write helper ────────────────────────────────────────────────────────

/// Append (or create) a `users:` entry in the target YAML file.
fn write_user_entry(target: &Path, key: &str, value: &str) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    if target.exists() {
        let existing = std::fs::read_to_string(target)
            .with_context(|| format!("failed to read {}", target.display()))?;

        if user_entry_exists(&existing, key) {
            bail!("user entry '{key}' already exists in {}", target.display());
        }

        let updated = insert_user_entry(&existing, key, value);
        std::fs::write(target, updated)
            .with_context(|| format!("failed to write {}", target.display()))?;
    } else {
        let content = format!(
            "version: 1\n\nusers:\n  {key}: \"{}\"\n",
            escape_yaml(value)
        );
        std::fs::write(target, content)
            .with_context(|| format!("failed to write {}", target.display()))?;
    }

    set_private_permissions(target)?;

    Ok(())
}

/// Return `true` if a `users:` key named `key` already appears in `content`.
fn user_entry_exists(content: &str, key: &str) -> bool {
    let pattern = format!("  {key}:");
    // Only match within the users: section (simple heuristic: look for the pattern anywhere).
    content.lines().any(|l| {
        l == pattern
            || l.starts_with(&format!("{pattern} "))
            || l.starts_with(&format!("{pattern}\""))
    })
}

/// Insert a `key: "value"` line under the `users:` section of `content`.
///
/// If a `users:` key exists, append after the last line of the users block.
/// Otherwise, append a new `users:` section at the end.
fn insert_user_entry(content: &str, key: &str, value: &str) -> String {
    let new_line = format!("  {key}: \"{}\"", escape_yaml(value));

    if content
        .lines()
        .any(|l| l == "users:" || l.starts_with("users:"))
    {
        let trimmed = content.trim_end();
        format!("{trimmed}\n{new_line}\n")
    } else {
        let trimmed = content.trim_end();
        format!("{trimmed}\n\nusers:\n{new_line}\n")
    }
}

/// Escape `"` characters in a YAML double-quoted scalar.
fn escape_yaml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("failed to set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

// ─── Prompt helpers ───────────────────────────────────────────────────────────

fn prompt(output: &mut dyn Write, input: &mut dyn BufRead, label: &str) -> Result<String> {
    write!(output, "  {label}: ")?;
    output.flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(line.trim().to_string())
}

// ─── Editor invocation ────────────────────────────────────────────────────────

fn open_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());

    let status = std::process::Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("failed to launch editor '{editor}'"))?;

    if !status.success() {
        bail!(
            "editor '{editor}' exited with status {}",
            status.code().unwrap_or(-1)
        );
    }

    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    use crate::config;

    fn write_yaml(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    fn load(
        cwd: &std::path::Path,
        user: Option<&std::path::Path>,
        sys: &std::path::Path,
    ) -> config::LoadedConfig {
        config::load_impl(cwd, Some("connections"), false, user, sys).unwrap()
    }

    fn run_add(layer: Layer, layer_dir: &Path, answers: &[&str]) -> Result<String> {
        let input_str = answers.join("\n") + "\n";
        let mut input = input_str.as_bytes();
        let mut output = Vec::new();
        add_impl(layer, layer_dir, &mut input, &mut output)?;
        Ok(String::from_utf8(output).unwrap())
    }

    // ── insert_user_entry ─────────────────────────────────────────────────────

    #[test]
    fn test_insert_user_entry_appends_under_existing_users_key() {
        let content = "version: 1\n\nusers:\n  existing: \"val\"\n";
        let result = insert_user_entry(content, "newkey", "newval");
        assert!(result.contains("existing:"));
        assert!(result.contains("newkey:"));
        assert!(result.contains("newval"));
    }

    #[test]
    fn test_insert_user_entry_adds_users_section_when_missing() {
        let content = "version: 1\n";
        let result = insert_user_entry(content, "testuser", "t1ext");
        assert!(result.contains("users:"));
        assert!(result.contains("testuser:"));
    }

    // ── user_entry_exists ─────────────────────────────────────────────────────

    #[test]
    fn test_user_entry_exists_finds_key() {
        let content = "users:\n  mykey: \"val\"\n";
        assert!(user_entry_exists(content, "mykey"));
    }

    #[test]
    fn test_user_entry_exists_false_when_absent() {
        let content = "users:\n  other: \"val\"\n";
        assert!(!user_entry_exists(content, "mykey"));
    }

    // ── add wizard round-trip ─────────────────────────────────────────────────

    #[test]
    fn test_add_creates_new_file_with_user_entry() {
        let dir = TempDir::new().unwrap();
        let answers = ["testuser", "testusername"];
        run_add(Layer::User, dir.path(), &answers).unwrap();

        let target = dir.path().join("connections.yaml");
        assert!(target.exists());
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("users:"));
        assert!(content.contains("testuser:"));
        assert!(content.contains("testusername"));
    }

    #[test]
    fn test_add_appends_to_existing_file() {
        let dir = TempDir::new().unwrap();
        write_yaml(
            dir.path(),
            "connections.yaml",
            "version: 1\n\nusers:\n  existing: \"oldval\"\n",
        );

        let answers = ["newkey", "newval"];
        run_add(Layer::User, dir.path(), &answers).unwrap();

        let content = fs::read_to_string(dir.path().join("connections.yaml")).unwrap();
        assert!(content.contains("existing:"));
        assert!(content.contains("newkey:"));
    }

    #[test]
    fn test_add_empty_key_aborts() {
        let dir = TempDir::new().unwrap();
        let answers = [""];
        let err = run_add(Layer::User, dir.path(), &answers).unwrap_err();
        assert!(err.to_string().contains("aborted"));
    }

    #[test]
    fn test_add_duplicate_key_returns_error() {
        let dir = TempDir::new().unwrap();
        write_yaml(
            dir.path(),
            "connections.yaml",
            "version: 1\n\nusers:\n  mykey: \"val\"\n",
        );

        let answers = ["mykey", "otherval"];
        let err = run_add(Layer::User, dir.path(), &answers).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    // ── resolve_edit_path ─────────────────────────────────────────────────────

    #[test]
    fn test_resolve_edit_path_no_layer_uses_active() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "version: 1\n\nusers:\n  mykey: \"val\"\nconnections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        let path = resolve_edit_path(&cfg, "mykey", None).unwrap();
        assert!(path.starts_with(user.path()));
    }

    #[test]
    fn test_resolve_edit_path_unknown_key_returns_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());

        let err = resolve_edit_path(&cfg, "no-such-key", None).unwrap_err();
        assert!(err.to_string().contains("no-such-key"));
    }

    // ── file permissions ──────────────────────────────────────────────────────

    #[test]
    #[cfg(unix)]
    fn test_add_new_file_has_0o600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let answers = ["k", "v"];
        run_add(Layer::User, dir.path(), &answers).unwrap();
        let target = dir.path().join("connections.yaml");
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // ── build_user_rows ───────────────────────────────────────────────────────

    /// When the `users:` map contains a `user` key, it appears as a normal row
    /// (no synthetic env-var row is added).
    #[test]
    fn test_build_user_rows_user_key_in_map_no_synthetic_row() {
        let cwd = TempDir::new().unwrap();
        let user_dir = TempDir::new().unwrap();
        write_yaml(
            user_dir.path(),
            "connections.yaml",
            "version: 1\n\nusers:\n  user: \"alice\"\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user_dir.path()), empty.path());

        let rows = build_user_rows(&cfg, Some("bob"));
        // The `user` row comes from the map.
        let user_row = rows.iter().find(|r| r.key == "user").expect("user row");
        assert_eq!(user_row.value, "alice");
        // Source must NOT be the env label.
        assert!(
            !user_row.source.contains("environment variable"),
            "source should not be env label: {}",
            user_row.source
        );
        // No duplicate user rows.
        assert_eq!(
            rows.iter().filter(|r| r.key == "user").count(),
            1,
            "should have exactly one user row"
        );
    }

    /// When the `users:` map has no `user` key but `$USER` is set, a synthetic
    /// env-var row is appended with SOURCE `env (environment variable $USER)`.
    #[test]
    fn test_build_user_rows_no_user_key_synthetic_env_row() {
        let cwd = TempDir::new().unwrap();
        let user_dir = TempDir::new().unwrap();
        write_yaml(
            user_dir.path(),
            "connections.yaml",
            "version: 1\n\nusers:\n  testuser: \"t1val\"\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user_dir.path()), empty.path());

        let rows = build_user_rows(&cfg, Some("bob"));
        let user_row = rows.iter().find(|r| r.key == "user").expect("synthetic user row");
        assert_eq!(user_row.value, "bob");
        assert!(
            user_row.source.contains("environment variable $USER"),
            "expected env label in source: {}",
            user_row.source
        );
        assert!(!user_row.shadowed);
    }

    /// When neither the `users:` map nor `$USER` is available, no synthetic row
    /// is added and no `user` row appears at all.
    #[test]
    fn test_build_user_rows_no_user_key_no_env_no_row() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());

        let rows = build_user_rows(&cfg, None);
        assert!(
            rows.iter().all(|r| r.key != "user"),
            "should have no user row when both absent"
        );
    }
}
