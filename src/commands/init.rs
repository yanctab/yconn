// src/commands/init.rs
// Handler for `yconn init` — scaffold a <group>.yaml in .yconn/ in the
// current directory.

use std::path::PathBuf;

use anyhow::{Context, Result};

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

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    run_impl(&cwd)
}

// ─── Testable impl ────────────────────────────────────────────────────────────

pub(crate) fn run_impl(cwd: &std::path::Path) -> Result<()> {
    let yconn_dir = cwd.join(".yconn");
    // Always scaffold connections.yaml — groups are inline fields, not per-file.
    let target = yconn_dir.join("connections.yaml");

    if target.exists() {
        anyhow::bail!(
            "{} already exists — edit it directly or use `yconn add` to add a connection",
            target.display()
        );
    }

    std::fs::create_dir_all(&yconn_dir)
        .with_context(|| format!("failed to create directory {}", yconn_dir.display()))?;

    std::fs::write(&target, TEMPLATE)
        .with_context(|| format!("failed to write {}", target.display()))?;

    set_private_permissions(&target)?;

    println!("Created {}", canonicalize_display(&target).display());
    println!("Edit it to add connections, then run `yconn list` to verify.");

    Ok(())
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

    #[test]
    fn test_init_creates_yconn_dir_and_file() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path()).unwrap();

        let target = dir.path().join(".yconn").join("connections.yaml");
        assert!(target.exists(), "config file should be created");
    }

    #[test]
    fn test_init_file_contains_template_markers() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path()).unwrap();

        let content =
            fs::read_to_string(dir.path().join(".yconn").join("connections.yaml")).unwrap();
        assert!(content.contains("connections:"));
        assert!(content.contains("version: 1"));
    }

    #[test]
    fn test_init_always_creates_connections_yaml() {
        // init always scaffolds connections.yaml — groups are inline fields, not per-file.
        let dir = TempDir::new().unwrap();
        run_impl(dir.path()).unwrap();

        let target = dir.path().join(".yconn").join("connections.yaml");
        assert!(target.exists(), "init must always create connections.yaml");
    }

    #[test]
    fn test_init_fails_if_file_already_exists() {
        let dir = TempDir::new().unwrap();
        let yconn = dir.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        fs::write(yconn.join("connections.yaml"), "connections: {}\n").unwrap();

        let err = run_impl(dir.path()).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_init_creates_yconn_dir_when_absent() {
        let dir = TempDir::new().unwrap();
        let yconn = dir.path().join(".yconn");
        assert!(!yconn.exists());

        run_impl(dir.path()).unwrap();

        assert!(yconn.exists());
    }

    #[test]
    #[cfg(unix)]
    fn test_init_file_has_0o600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        run_impl(dir.path()).unwrap();

        let target = dir.path().join(".yconn").join("connections.yaml");
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "init file should have 0o600 permissions");
    }
}
