// Handler for `yconn list` — list all connections across all layers.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::config::{Connection, LoadedConfig};
use crate::display::{ConnectionRow, Renderer, UserRow};

pub fn run(cfg: &LoadedConfig, renderer: &Renderer, all: bool, group: Option<&str>) -> Result<()> {
    // Collect active connections for the table.
    let active_connections: Vec<&Connection> = if all {
        // When --all, we still scan only active (non-shadowed) connections
        // for the Users table, but render all connections in the table.
        cfg.connections.iter().collect()
    } else {
        let filter = cfg.effective_group_filter(all, group);
        cfg.filtered_connections(filter)
    };

    // Extract unique ${...} placeholder keys from the user field of active connections.
    let user_rows = build_user_rows(cfg, &active_connections);

    // Print Users table before connections if any placeholders were found.
    if !user_rows.is_empty() {
        renderer.user_list(&user_rows);
        println!();
    }

    // Build and print connections table.
    let rows: Vec<ConnectionRow> = if all {
        cfg.all_connections.iter().map(conn_to_row).collect()
    } else {
        active_connections.iter().map(|c| conn_to_row(c)).collect()
    };

    renderer.list(&rows);
    Ok(())
}

/// Extract unique `${...}` placeholder keys from the `user` field of the given
/// connections and build [`UserRow`] entries showing resolved values or
/// `[unresolved]` with a fix hint.
fn build_user_rows(cfg: &LoadedConfig, connections: &[&Connection]) -> Vec<UserRow> {
    let mut seen = HashSet::new();
    let mut keys_ordered: Vec<String> = Vec::new();

    for conn in connections {
        for key in extract_placeholder_keys(&conn.user) {
            if seen.insert(key.clone()) {
                keys_ordered.push(key);
            }
        }
    }

    let empty_overrides: HashMap<String, String> = HashMap::new();

    keys_ordered
        .into_iter()
        .map(|key| {
            // Try to resolve the key.
            if key == "user" {
                // ${user} resolves from $USER env var or users map.
                let (expanded, warnings) =
                    cfg.expand_user_field(&format!("${{{key}}}"), &empty_overrides);
                if warnings.is_empty() && expanded != format!("${{{key}}}") {
                    // Resolved — determine source.
                    let source = if let Some(entry) = cfg.users.get(&key) {
                        format!("{} ({})", entry.layer.label(), entry.source_path.display())
                    } else {
                        "env (environment variable $USER)".to_string()
                    };
                    UserRow {
                        key,
                        value: expanded,
                        source,
                        shadowed: false,
                    }
                } else {
                    UserRow {
                        key: key.clone(),
                        value: "[unresolved]".to_string(),
                        source: format!("-> yconn users add --user {}:VALUE", key),
                        shadowed: false,
                    }
                }
            } else if let Some(entry) = cfg.users.get(&key) {
                UserRow {
                    key,
                    value: entry.value.clone(),
                    source: format!("{} ({})", entry.layer.label(), entry.source_path.display()),
                    shadowed: false,
                }
            } else {
                UserRow {
                    key: key.clone(),
                    value: "[unresolved]".to_string(),
                    source: format!("-> yconn users add --user {}:VALUE", key),
                    shadowed: false,
                }
            }
        })
        .collect()
}

/// Extract all `${...}` placeholder key names from a string.
///
/// Returns keys in order of appearance. Duplicates within the same string
/// are included — deduplication is the caller's responsibility.
fn extract_placeholder_keys(s: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if i + 1 < len && bytes[i] == b'$' && bytes[i + 1] == b'{' {
            if let Some(close) = s[i + 2..].find('}') {
                let key = &s[i + 2..i + 2 + close];
                if !key.is_empty() {
                    keys.push(key.to_string());
                }
                i += 2 + close + 1;
                continue;
            }
        }
        i += 1;
    }
    keys
}

fn conn_to_row(c: &Connection) -> ConnectionRow {
    ConnectionRow {
        name: c.name.clone(),
        host: c.host.clone(),
        user: c.user.clone(),
        port: c.port,
        auth: c.auth.type_label().to_string(),
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
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n"
        )
    }

    fn conn_with_group(name: &str, host: &str, group: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n    group: {group}\n"
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

    // ─── extract_placeholder_keys tests ──────────────────────────────────────

    #[test]
    fn test_extract_single_placeholder() {
        let keys = extract_placeholder_keys("${deploy_user}");
        assert_eq!(keys, vec!["deploy_user"]);
    }

    #[test]
    fn test_extract_multiple_placeholders() {
        let keys = extract_placeholder_keys("${prefix}_${suffix}");
        assert_eq!(keys, vec!["prefix", "suffix"]);
    }

    #[test]
    fn test_extract_no_placeholders() {
        let keys = extract_placeholder_keys("plain_user");
        assert!(keys.is_empty());
    }

    #[test]
    fn test_extract_user_placeholder() {
        let keys = extract_placeholder_keys("${user}");
        assert_eq!(keys, vec!["user"]);
    }

    // ─── Users table in yconn list tests ─────────────────────────────────────

    fn conn_yaml_with_user(name: &str, host: &str, user: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: {user}\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n"
        )
    }

    fn conn_yaml_with_users_section(conns: &str, users: &str) -> String {
        format!("{conns}\nusers:\n{users}")
    }

    #[test]
    fn test_placeholder_resolved_shows_value_and_source() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let yaml = conn_yaml_with_users_section(
            &conn_yaml_with_user("srv", "10.0.0.1", "${deploy_user}"),
            "  deploy_user: admin",
        );
        write_yaml(&yconn, "connections.yaml", &yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let active: Vec<&Connection> = cfg.connections.iter().collect();
        let rows = build_user_rows(&cfg, &active);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "deploy_user");
        assert_eq!(rows[0].value, "admin");
        assert!(rows[0].source.contains("project"));
        assert!(!rows[0].shadowed);
    }

    #[test]
    fn test_placeholder_unresolved_shows_hint() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        write_yaml(
            &yconn,
            "connections.yaml",
            &conn_yaml_with_user("srv", "10.0.0.1", "${missing_key}"),
        );

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let active: Vec<&Connection> = cfg.connections.iter().collect();
        let rows = build_user_rows(&cfg, &active);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "missing_key");
        assert_eq!(rows[0].value, "[unresolved]");
        assert!(rows[0]
            .source
            .contains("yconn users add --user missing_key:VALUE"));
    }

    #[test]
    fn test_no_placeholders_produces_no_user_rows() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        write_yaml(&yconn, "connections.yaml", &simple_conn("srv", "10.0.0.1"));

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let active: Vec<&Connection> = cfg.connections.iter().collect();
        let rows = build_user_rows(&cfg, &active);

        assert!(rows.is_empty());
    }

    #[test]
    fn test_mixed_resolved_and_unresolved_placeholders() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        // Two connections: one with a resolved placeholder, one with an unresolved one.
        let yaml = conn_yaml_with_users_section(
            &format!(
                "connections:\n  srv1:\n    host: 10.0.0.1\n    user: ${{known_user}}\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n  srv2:\n    host: 10.0.0.2\n    user: ${{unknown_user}}\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n"
            ),
            "  known_user: admin",
        );
        write_yaml(&yconn, "connections.yaml", &yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let active: Vec<&Connection> = cfg.connections.iter().collect();
        let rows = build_user_rows(&cfg, &active);

        assert_eq!(rows.len(), 2);

        // Find each row by key.
        let known = rows.iter().find(|r| r.key == "known_user").unwrap();
        let unknown = rows.iter().find(|r| r.key == "unknown_user").unwrap();

        assert_eq!(known.value, "admin");
        assert!(known.source.contains("project"));

        assert_eq!(unknown.value, "[unresolved]");
        assert!(unknown
            .source
            .contains("yconn users add --user unknown_user:VALUE"));
    }

    #[test]
    fn test_duplicate_placeholder_deduplication() {
        // The same ${deploy_user} in multiple connections should produce one row.
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let yaml = conn_yaml_with_users_section(
            &format!(
                "connections:\n  srv1:\n    host: 10.0.0.1\n    user: ${{deploy_user}}\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n  srv2:\n    host: 10.0.0.2\n    user: ${{deploy_user}}\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n"
            ),
            "  deploy_user: admin",
        );
        write_yaml(&yconn, "connections.yaml", &yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let active: Vec<&Connection> = cfg.connections.iter().collect();
        let rows = build_user_rows(&cfg, &active);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "deploy_user");
        assert_eq!(rows[0].value, "admin");
    }

    #[test]
    fn test_connections_table_preserves_raw_placeholder_syntax() {
        // The USER column in the connections table should show the raw ${...} syntax.
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();

        let yaml = conn_yaml_with_users_section(
            &conn_yaml_with_user("srv", "10.0.0.1", "${deploy_user}"),
            "  deploy_user: admin",
        );
        write_yaml(&yconn, "connections.yaml", &yaml);

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        // The connection's user field should still contain the raw placeholder.
        assert_eq!(cfg.connections[0].user, "${deploy_user}");

        // The ConnectionRow should also preserve the raw syntax.
        let row = conn_to_row(&cfg.connections[0]);
        assert_eq!(row.user, "${deploy_user}");
    }
}
