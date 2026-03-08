// src/commands/init.rs
// Handler for `yconn init` — scaffold a connections.yaml in the current
// directory, at the location specified by `--location`.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::cli::InitLocation;

// ─── Template ────────────────────────────────────────────────────────────────

const TEMPLATE: &str = "\
version: 1

# Uncomment and fill in the docker block if you want yconn to re-invoke itself
# inside a container that has SSH keys pre-baked.
#
# docker:
#   image: ghcr.io/myorg/yconn-keys:latest
#   pull: missing   # always | missing | never
#   args: []        # extra docker run arguments

connections:
  # example:
  #   host: 10.0.1.50
  #   user: deploy
  #   port: 22        # optional, defaults to 22
  #   auth: key       # key | password
  #   key: ~/.ssh/id_rsa
  #   description: \"Example server\"
  #   link: https://wiki.internal/servers/example   # optional
";

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn run(location: InitLocation) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    run_impl(&cwd, location)
}

// ─── Testable impl ────────────────────────────────────────────────────────────

pub(crate) fn run_impl(cwd: &std::path::Path, location: InitLocation) -> Result<()> {
    let target = resolve_target(cwd, location);

    if target.exists() {
        anyhow::bail!(
            "{} already exists — edit it directly or use `yconn connections add` to add a connection",
            target.display()
        );
    }

    // Create parent directory if needed (only .yconn/ for the default location).
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    std::fs::write(&target, TEMPLATE)
        .with_context(|| format!("failed to write {}", target.display()))?;

    set_private_permissions(&target)?;

    println!("Created {}", canonicalize_display(&target).display());
    println!("Edit it to add connections, then run `yconn list` to verify.");

    Ok(())
}

/// Resolve the target file path from `cwd` and the chosen location.
///
/// - `InitLocation::Yconn`   → `<cwd>/.yconn/connections.yaml`
/// - `InitLocation::Dotfile` → `<cwd>/.connections.yaml`
/// - `InitLocation::Plain`   → `<cwd>/connections.yaml`
pub(crate) fn resolve_target(cwd: &std::path::Path, location: InitLocation) -> PathBuf {
    match location {
        InitLocation::Yconn => cwd.join(".yconn").join("connections.yaml"),
        InitLocation::Dotfile => cwd.join(".connections.yaml"),
        InitLocation::Plain => cwd.join("connections.yaml"),
    }
}

/// Return a canonical display path, falling back to the original if
/// `canonicalize` fails (e.g. on a tempdir that has been removed).
fn canonicalize_display(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Set 0o600 permissions on `path` so it is not world-readable.
///
/// No-op on non-Unix platforms.
#[cfg(unix)]
fn set_private_permissions(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("failed to set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &std::path::Path) -> anyhow::Result<()> {
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── default location (yconn) ──────────────────────────────────────────────

    #[test]
    fn test_init_yconn_creates_yconn_dir_and_file() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), InitLocation::Yconn).unwrap();

        let target = dir.path().join(".yconn").join("connections.yaml");
        assert!(target.exists(), "config file should be created");
    }

    #[test]
    fn test_init_yconn_file_contains_template_markers() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), InitLocation::Yconn).unwrap();

        let content =
            fs::read_to_string(dir.path().join(".yconn").join("connections.yaml")).unwrap();
        assert!(content.contains("connections:"));
        assert!(content.contains("version: 1"));
    }

    #[test]
    fn test_init_yconn_creates_yconn_dir_when_absent() {
        let dir = TempDir::new().unwrap();
        let yconn = dir.path().join(".yconn");
        assert!(!yconn.exists());

        run_impl(dir.path(), InitLocation::Yconn).unwrap();

        assert!(yconn.exists());
    }

    #[test]
    fn test_init_yconn_fails_if_file_already_exists() {
        let dir = TempDir::new().unwrap();
        let yconn = dir.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        fs::write(yconn.join("connections.yaml"), "connections: {}\n").unwrap();

        let err = run_impl(dir.path(), InitLocation::Yconn).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    // ── dotfile location ──────────────────────────────────────────────────────

    #[test]
    fn test_init_dotfile_creates_file_in_cwd() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), InitLocation::Dotfile).unwrap();

        let target = dir.path().join(".connections.yaml");
        assert!(target.exists(), ".connections.yaml should be created");
    }

    #[test]
    fn test_init_dotfile_file_contains_template_markers() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), InitLocation::Dotfile).unwrap();

        let content = fs::read_to_string(dir.path().join(".connections.yaml")).unwrap();
        assert!(content.contains("connections:"));
        assert!(content.contains("version: 1"));
    }

    #[test]
    fn test_init_dotfile_fails_if_file_already_exists() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".connections.yaml"), "connections: {}\n").unwrap();

        let err = run_impl(dir.path(), InitLocation::Dotfile).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    // ── plain location ────────────────────────────────────────────────────────

    #[test]
    fn test_init_plain_creates_file_in_cwd() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), InitLocation::Plain).unwrap();

        let target = dir.path().join("connections.yaml");
        assert!(target.exists(), "connections.yaml should be created");
    }

    #[test]
    fn test_init_plain_file_contains_template_markers() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), InitLocation::Plain).unwrap();

        let content = fs::read_to_string(dir.path().join("connections.yaml")).unwrap();
        assert!(content.contains("connections:"));
        assert!(content.contains("version: 1"));
    }

    #[test]
    fn test_init_plain_fails_if_file_already_exists() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("connections.yaml"), "connections: {}\n").unwrap();

        let err = run_impl(dir.path(), InitLocation::Plain).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    // ── resolve_target helper ─────────────────────────────────────────────────

    #[test]
    fn test_resolve_target_yconn() {
        let dir = TempDir::new().unwrap();
        let target = resolve_target(dir.path(), InitLocation::Yconn);
        assert_eq!(target, dir.path().join(".yconn").join("connections.yaml"));
    }

    #[test]
    fn test_resolve_target_dotfile() {
        let dir = TempDir::new().unwrap();
        let target = resolve_target(dir.path(), InitLocation::Dotfile);
        assert_eq!(target, dir.path().join(".connections.yaml"));
    }

    #[test]
    fn test_resolve_target_plain() {
        let dir = TempDir::new().unwrap();
        let target = resolve_target(dir.path(), InitLocation::Plain);
        assert_eq!(target, dir.path().join("connections.yaml"));
    }

    // ── permissions ───────────────────────────────────────────────────────────

    #[test]
    #[cfg(unix)]
    fn test_init_yconn_file_has_0o600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), InitLocation::Yconn).unwrap();

        let target = dir.path().join(".yconn").join("connections.yaml");
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "init file should have 0o600 permissions");
    }

    #[test]
    #[cfg(unix)]
    fn test_init_dotfile_has_0o600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), InitLocation::Dotfile).unwrap();

        let target = dir.path().join(".connections.yaml");
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "dotfile should have 0o600 permissions");
    }

    #[test]
    #[cfg(unix)]
    fn test_init_plain_has_0o600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), InitLocation::Plain).unwrap();

        let target = dir.path().join("connections.yaml");
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "plain file should have 0o600 permissions");
    }
}
