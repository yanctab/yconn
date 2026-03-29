// src/commands/edit.rs
// Handler for `yconn edit <name>` — open the connection's source config file
// in $EDITOR.

use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

use crate::cli::LayerArg;
use crate::config::{Layer, LoadedConfig};

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn run(cfg: &LoadedConfig, name: &str, layer: Option<LayerArg>) -> Result<()> {
    let path = resolve_path(cfg, name, layer)?;
    open_editor(&path)
}

// ─── Path resolution ──────────────────────────────────────────────────────────

/// Find the config file path to open.
///
/// If `--layer` is given, look for the connection in that layer only.
/// Otherwise use the source path from the active (highest-priority) connection.
fn resolve_path(
    cfg: &LoadedConfig,
    name: &str,
    layer: Option<LayerArg>,
) -> Result<std::path::PathBuf> {
    if let Some(layer_arg) = layer {
        let target_layer = layer_arg_to_layer(layer_arg);
        // Search all_connections for the named entry in the specified layer.
        let conn = cfg
            .all_connections
            .iter()
            .find(|c| c.name == name && c.layer == target_layer)
            .ok_or_else(|| {
                anyhow!(
                    "no connection named '{name}' in the {} layer",
                    target_layer.label()
                )
            })?;
        Ok(conn.source_path.clone())
    } else {
        let conn = cfg
            .find(name)
            .ok_or_else(|| anyhow!("no connection named '{name}'"))?;
        Ok(conn.source_path.clone())
    }
}

fn layer_arg_to_layer(arg: LayerArg) -> Layer {
    match arg {
        LayerArg::Project => Layer::Project,
        LayerArg::User => Layer::User,
        LayerArg::System => Layer::System,
    }
}

// ─── Editor invocation ────────────────────────────────────────────────────────

fn open_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());

    let status = Command::new(&editor)
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

    fn write_yaml(dir: &std::path::Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    fn simple_conn(name: &str, host: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: u\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: d\n"
        )
    }

    fn load(
        cwd: &std::path::Path,
        user: Option<&std::path::Path>,
        sys: &std::path::Path,
    ) -> config::LoadedConfig {
        config::load_impl(cwd, Some("connections"), false, user, sys).unwrap()
    }

    // ── resolve_path ──────────────────────────────────────────────────────────

    #[test]
    fn test_resolve_path_no_layer_uses_active_source() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(&yconn, "connections.yaml", &simple_conn("srv", "10.0.0.1"));

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let path = resolve_path(&cfg, "srv", None).unwrap();
        assert_eq!(path, yconn.join("connections.yaml"));
    }

    #[test]
    fn test_resolve_path_with_layer_flag_finds_shadowed_entry() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &simple_conn("srv", "project-host"),
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("srv", "system-host"),
        );

        let cfg = load(root.path(), None, sys.path());

        // Without --layer we get the project path.
        let p = resolve_path(&cfg, "srv", None).unwrap();
        assert!(p.starts_with(&yconn));

        // With --layer system we get the system path.
        let p2 = resolve_path(&cfg, "srv", Some(LayerArg::System)).unwrap();
        assert!(p2.starts_with(sys.path()));
    }

    #[test]
    fn test_resolve_path_unknown_name_returns_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());

        let err = resolve_path(&cfg, "no-such", None).unwrap_err();
        assert!(err.to_string().contains("no-such"));
    }

    #[test]
    fn test_resolve_path_layer_flag_name_missing_in_layer() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("srv", "1.2.3.4"),
        );

        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        // 'srv' exists in user layer but not project layer.
        let err = resolve_path(&cfg, "srv", Some(LayerArg::Project)).unwrap_err();
        assert!(err.to_string().contains("srv"));
        assert!(err.to_string().contains("project"));
    }

    // ── layer_arg_to_layer ────────────────────────────────────────────────────

    #[test]
    fn test_layer_arg_to_layer_all_variants() {
        assert!(matches!(
            layer_arg_to_layer(LayerArg::System),
            Layer::System
        ));
        assert!(matches!(layer_arg_to_layer(LayerArg::User), Layer::User));
        assert!(matches!(
            layer_arg_to_layer(LayerArg::Project),
            Layer::Project
        ));
    }
}
