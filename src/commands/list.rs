// Handler for `yconn list` — list all connections across all layers.

use anyhow::Result;

use crate::config::LoadedConfig;
use crate::display::{ConnectionRow, Renderer};

pub fn run(cfg: &LoadedConfig, renderer: &Renderer, all: bool) -> Result<()> {
    let connections = if all {
        &cfg.all_connections
    } else {
        &cfg.connections
    };

    let rows: Vec<ConnectionRow> = connections
        .iter()
        .map(|c| ConnectionRow {
            name: c.name.clone(),
            host: c.host.clone(),
            user: c.user.clone(),
            port: c.port,
            auth: c.auth.clone(),
            source: c.layer.label().to_string(),
            description: c.description.clone(),
            shadowed: c.shadowed,
        })
        .collect();

    renderer.list(&rows);
    Ok(())
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

    fn simple_conn(name: &str, host: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth: key\n    description: desc\n"
        )
    }

    fn no_color() -> Renderer {
        Renderer::new(false)
    }

    fn load(
        cwd: &std::path::Path,
        user: Option<&std::path::Path>,
        sys: &std::path::Path,
    ) -> config::LoadedConfig {
        config::load_impl(cwd, "connections", false, user, sys).unwrap()
    }

    #[test]
    fn test_list_single_connection_no_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(&yconn, "connections.yaml", &simple_conn("prod", "10.0.0.1"));

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        assert_eq!(cfg.connections.len(), 1);
        run(&cfg, &no_color(), false).unwrap();
    }

    #[test]
    fn test_list_empty_config_no_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        assert_eq!(cfg.connections.len(), 0);
        run(&cfg, &no_color(), false).unwrap();
    }

    #[test]
    fn test_list_all_includes_shadowed_entry() {
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
        assert_eq!(cfg.connections.len(), 1);
        assert_eq!(cfg.all_connections.len(), 2);

        // --all includes the shadowed entry
        run(&cfg, &no_color(), true).unwrap();
    }

    #[test]
    fn test_list_without_all_excludes_shadowed() {
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
        // Without --all, only 1 active connection exposed
        run(&cfg, &no_color(), false).unwrap();
        assert_eq!(cfg.connections.len(), 1);
        assert!(!cfg.connections[0].shadowed);
    }

    #[test]
    fn test_list_multiple_layers_no_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &simple_conn("proj-srv", "1.0.0.1"),
        );

        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("user-srv", "2.0.0.1"),
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("sys-srv", "3.0.0.1"),
        );

        let cfg = load(root.path(), Some(user.path()), sys.path());
        assert_eq!(cfg.connections.len(), 3);
        run(&cfg, &no_color(), false).unwrap();
    }
}
