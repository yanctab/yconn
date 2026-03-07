// Handler for `yconn show <name>` and `yconn show --dump`.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::LoadedConfig;
use crate::display::{ConnectionDetail, Renderer};

// ─── Dump serialisation types ─────────────────────────────────────────────────

#[derive(Serialize)]
struct DumpConn {
    host: String,
    user: String,
    port: u16,
    auth: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
}

#[derive(Serialize)]
struct DumpOutput {
    connections: HashMap<String, DumpConn>,
    users: HashMap<String, String>,
}

/// Build the YAML string for `yconn show --dump`.
fn build_dump_yaml(cfg: &LoadedConfig) -> Result<String> {
    let mut connections = HashMap::new();
    for conn in &cfg.connections {
        connections.insert(
            conn.name.clone(),
            DumpConn {
                host: conn.host.clone(),
                user: conn.user.clone(),
                port: conn.port,
                auth: conn.auth.clone(),
                key: conn.key.clone(),
                description: conn.description.clone(),
                link: conn.link.clone(),
                group: conn.group.clone(),
            },
        );
    }
    let users: HashMap<String, String> = cfg
        .users
        .iter()
        .map(|(k, v)| (k.clone(), v.value.clone()))
        .collect();
    let output = DumpOutput { connections, users };
    serde_yaml::to_string(&output).context("failed to serialise config to YAML")
}

pub fn run_dump(cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let yaml = build_dump_yaml(cfg)?;
    renderer.dump(&yaml);
    Ok(())
}

pub fn run(cfg: &LoadedConfig, renderer: &Renderer, name: &str) -> Result<()> {
    let conn = cfg.find_with_wildcard(name)?;

    let detail = ConnectionDetail {
        name: conn.name.clone(),
        host: conn.host.clone(),
        user: conn.user.clone(),
        port: conn.port,
        auth: conn.auth.clone(),
        key: conn.key.clone(),
        description: conn.description.clone(),
        link: conn.link.clone(),
        source_label: conn.layer.label().to_string(),
        source_path: conn.source_path.display().to_string(),
    };

    renderer.show(&detail);
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

    // ── dump unit tests ───────────────────────────────────────────────────────

    #[test]
    fn test_dump_with_connections_only() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth: key\n    key: ~/.ssh/id_rsa\n    description: Test server\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let yaml = build_dump_yaml(&cfg).unwrap();
        assert!(yaml.contains("connections:"));
        assert!(yaml.contains("srv:"));
        assert!(yaml.contains("10.0.0.1"));
    }

    #[test]
    fn test_dump_with_users_only() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "users:\n  alice: al\n  bob: bobby\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let yaml = build_dump_yaml(&cfg).unwrap();
        assert!(yaml.contains("users:"));
        assert!(yaml.contains("alice"));
        assert!(yaml.contains("bob"));
    }

    #[test]
    fn test_dump_with_connections_and_users() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  web:\n    host: 1.2.3.4\n    user: ${testuser}\n    auth: key\n    key: ~/.ssh/id_rsa\n    description: Web server\nusers:\n  testuser: alice\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let yaml = build_dump_yaml(&cfg).unwrap();
        assert!(yaml.contains("connections:"));
        assert!(yaml.contains("users:"));
        assert!(yaml.contains("web:"));
        assert!(yaml.contains("testuser"));
        // Raw unexpanded value in connections
        assert!(yaml.contains("${testuser}"));
    }

    #[test]
    fn test_dump_with_empty_config() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        let yaml = build_dump_yaml(&cfg).unwrap();
        // Valid YAML even with no data
        assert!(yaml.contains("connections:"));
        assert!(yaml.contains("users:"));
    }

    #[test]
    fn test_show_existing_connection_no_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  prod:\n    host: 10.0.0.1\n    user: deploy\n    auth: key\n    key: ~/.ssh/id_rsa\n    description: Production server\n",
        );

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        run(&cfg, &no_color(), "prod").unwrap();
    }

    #[test]
    fn test_show_missing_name_returns_error() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        let err = run(&cfg, &no_color(), "does-not-exist").unwrap_err();
        assert!(err.to_string().contains("does-not-exist"));
    }

    #[test]
    fn test_show_error_message_contains_name() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());

        let err = run(&cfg, &no_color(), "my-conn").unwrap_err();
        assert!(err.to_string().contains("my-conn"));
    }

    #[test]
    fn test_show_with_all_optional_fields() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  srv:\n    host: 1.2.3.4\n    user: admin\n    port: 2222\n    auth: key\n    key: ~/.ssh/id_ed25519\n    description: Test\n    link: https://wiki.example.com\n",
        );

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        run(&cfg, &no_color(), "srv").unwrap();
    }

    #[test]
    fn test_show_password_auth_no_key() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  db:\n    host: db.internal\n    user: dbadmin\n    auth: password\n    description: Database\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());
        run(&cfg, &no_color(), "db").unwrap();
    }
}
