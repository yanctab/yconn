// Handler for `yconn list` — list all connections across all layers.

use anyhow::Result;

use crate::config::{Connection, LoadedConfig};
use crate::display::{ConnectionRow, Renderer};

pub fn run(cfg: &LoadedConfig, renderer: &Renderer, all: bool, group: Option<&str>) -> Result<()> {
    let rows: Vec<ConnectionRow> = if all {
        cfg.all_connections.iter().map(conn_to_row).collect()
    } else {
        let filter = cfg.effective_group_filter(all, group);
        cfg.filtered_connections(filter)
            .iter()
            .map(|c| conn_to_row(c))
            .collect()
    };

    renderer.list(&rows);
    Ok(())
}

fn conn_to_row(c: &Connection) -> ConnectionRow {
    ConnectionRow {
        name: c.name.clone(),
        host: c.host.clone(),
        user: c.user.clone(),
        port: c.port,
        auth: c.auth.clone(),
        source: c.layer.label().to_string(),
        description: c.description.clone(),
        shadowed: c.shadowed,
    }
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

    fn conn_with_group(name: &str, host: &str, group: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth: key\n    description: desc\n    group: {group}\n"
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
        config::load_impl(cwd, Some("connections"), false, user, sys).unwrap()
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
        run(&cfg, &no_color(), false, None).unwrap();
    }

    #[test]
    fn test_list_empty_config_no_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        assert_eq!(cfg.connections.len(), 0);
        run(&cfg, &no_color(), false, None).unwrap();
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
        run(&cfg, &no_color(), true, None).unwrap();
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
        run(&cfg, &no_color(), false, None).unwrap();
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
        run(&cfg, &no_color(), false, None).unwrap();
    }

    // ─── --group filter tests ─────────────────────────────────────────────────

    #[test]
    fn test_list_group_filter_returns_only_matching_connections() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        // Two connections: one tagged "work", one tagged "private".
        let yaml = format!(
            "{}\n{}",
            conn_with_group("work-srv", "10.0.0.1", "work"),
            conn_with_group("private-srv", "10.0.0.2", "private").replace("connections:\n", "")
        );
        write_yaml(&yconn, "connections.yaml", &yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        assert_eq!(cfg.connections.len(), 2);

        // Only the "work" connection should appear when filtering by "work".
        let filter = cfg.effective_group_filter(false, Some("work"));
        let filtered = cfg.filtered_connections(filter);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "work-srv");

        // run() should not error.
        run(&cfg, &no_color(), false, Some("work")).unwrap();
    }

    #[test]
    fn test_list_group_with_all_shows_all_connections() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        let yaml = format!(
            "{}\n{}",
            conn_with_group("work-srv", "10.0.0.1", "work"),
            conn_with_group("private-srv", "10.0.0.2", "private").replace("connections:\n", "")
        );
        write_yaml(&yconn, "connections.yaml", &yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        // --all overrides any group filter — effective_group_filter returns None.
        let filter = cfg.effective_group_filter(true, Some("work"));
        assert!(filter.is_none());

        // run() with all=true shows both connections.
        run(&cfg, &no_color(), true, Some("work")).unwrap();
    }

    #[test]
    fn test_list_group_with_no_matches_returns_empty_list() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &conn_with_group("work-srv", "10.0.0.1", "work"),
        );

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        // Filtering by an unknown group should return empty — no error.
        let filter = cfg.effective_group_filter(false, Some("unknown"));
        let filtered = cfg.filtered_connections(filter);
        assert_eq!(filtered.len(), 0);

        run(&cfg, &no_color(), false, Some("unknown")).unwrap();
    }
}
