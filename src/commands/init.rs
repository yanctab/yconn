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
    let group = crate::group::active_group()
        .context("cannot determine active group")?
        .name;
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    run_impl(&cwd, &group)
}

// ─── Testable impl ────────────────────────────────────────────────────────────

pub(crate) fn run_impl(cwd: &std::path::Path, group: &str) -> Result<()> {
    let yconn_dir = cwd.join(".yconn");
    let target = yconn_dir.join(format!("{group}.yaml"));

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

    println!("Created {}", canonicalize_display(&target).display());
    println!("Edit it to add connections, then run `yconn list` to verify.");

    Ok(())
}

/// Return a canonical display path, falling back to the original if
/// `canonicalize` fails (e.g. on a tempdir that has been removed).
fn canonicalize_display(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
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
        run_impl(dir.path(), "connections").unwrap();

        let target = dir.path().join(".yconn").join("connections.yaml");
        assert!(target.exists(), "config file should be created");
    }

    #[test]
    fn test_init_file_contains_template_markers() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), "connections").unwrap();

        let content =
            fs::read_to_string(dir.path().join(".yconn").join("connections.yaml")).unwrap();
        assert!(content.contains("connections:"));
        assert!(content.contains("version: 1"));
    }

    #[test]
    fn test_init_uses_active_group_name() {
        let dir = TempDir::new().unwrap();
        run_impl(dir.path(), "work").unwrap();

        let target = dir.path().join(".yconn").join("work.yaml");
        assert!(target.exists(), "file should use the active group name");
    }

    #[test]
    fn test_init_fails_if_file_already_exists() {
        let dir = TempDir::new().unwrap();
        let yconn = dir.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        fs::write(yconn.join("connections.yaml"), "connections: {}\n").unwrap();

        let err = run_impl(dir.path(), "connections").unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_init_creates_yconn_dir_when_absent() {
        let dir = TempDir::new().unwrap();
        let yconn = dir.path().join(".yconn");
        assert!(!yconn.exists());

        run_impl(dir.path(), "connections").unwrap();

        assert!(yconn.exists());
    }
}
