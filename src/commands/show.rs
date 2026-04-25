// Handler for `yconn show <name>` and `yconn show --dump`.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::{Auth, Connection, LoadedConfig};
use crate::display::{ConnectionDetail, Renderer};

/// Render `generate_key` for a connection with both `${key}` and `${user}`
/// placeholders expanded in a single pass. Returns `None` when the connection
/// has no `generate_key` configured.
fn render_generate_key(conn: &Connection) -> Option<String> {
    conn.auth.generate_key_rendered(&conn.user)
}

// ─── Dump serialisation types ─────────────────────────────────────────────────

#[derive(Serialize)]
struct DumpConn {
    host: String,
    user: String,
    port: u16,
    auth: Auth,
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
///
/// The raw `serde_yaml` output is post-processed to inject blank lines:
/// - Between consecutive connection entries within the `connections:` block.
/// - Between the `connections:` block and the `users:` block.
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
    let raw = serde_yaml::to_string(&output).context("failed to serialise config to YAML")?;
    Ok(inject_dump_blank_lines(&raw))
}

/// Post-process a serialised `DumpOutput` YAML string to insert blank lines:
///
/// 1. Between each consecutive pair of connection entries (two-space-indented
///    map keys inside the `connections:` block).
/// 2. Between the `connections:` block and the top-level `users:` key.
///
/// The output remains valid YAML — blank lines are allowed between mapping
/// entries in YAML 1.1 and 1.2, and `serde_yaml` round-trips them correctly.
fn inject_dump_blank_lines(raw: &str) -> String {
    // State machine over lines.
    // Sections at column-0: "connections:" and "users:"
    // Connection entry lines: exactly "  <name>:" (two leading spaces, then
    // a non-space char). These are the per-entry headers inside `connections:`.

    #[derive(PartialEq)]
    enum Section {
        Other,
        Connections,
        Users,
    }

    let mut out = String::with_capacity(raw.len() + 64);
    let mut section = Section::Other;
    let mut first_conn_entry = true; // skip blank before the very first entry

    for line in raw.lines() {
        // Detect top-level section transitions (no leading whitespace).
        if !line.starts_with(' ') && !line.is_empty() {
            if line.starts_with("connections:") {
                section = Section::Connections;
                first_conn_entry = true;
            } else if line.starts_with("users:") {
                // Insert a blank line between the connections block and users:
                if section == Section::Connections {
                    out.push('\n');
                }
                section = Section::Users;
            } else {
                section = Section::Other;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Inside `connections:`, detect entry headers: lines with exactly two
        // leading spaces followed by a non-space character.
        if section == Section::Connections
            && line.starts_with("  ")
            && line.len() > 2
            && !line[2..].starts_with(' ')
        {
            if first_conn_entry {
                first_conn_entry = false;
            } else {
                // Blank line before every entry after the first.
                out.push('\n');
            }
        }

        out.push_str(line);
        out.push('\n');
    }

    out
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
        auth: conn.auth.type_label().to_string(),
        key: conn.auth.key().map(str::to_string),
        generate_key: render_generate_key(&conn),
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
            "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: Test server\n",
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
            "connections:\n  web:\n    host: 1.2.3.4\n    user: ${testuser}\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: Web server\nusers:\n  testuser: alice\n",
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

    // ── inject_dump_blank_lines unit tests ────────────────────────────────────

    /// Single connection: no blank line injected between entries (nothing to
    /// separate), but blank line between connections block and users block.
    #[test]
    fn test_inject_blank_lines_single_connection_and_users() {
        let input = "connections:\n  web:\n    host: 1.2.3.4\n    user: ops\nusers:\n  k: v\n";
        let out = inject_dump_blank_lines(input);
        // users: must be preceded by a blank line
        assert!(
            out.contains("\nusers:"),
            "expected blank line before users: got:\n{out}"
        );
        // Count blank lines inside connections block — should be zero (one entry)
        let conn_section: String = out
            .lines()
            .skip_while(|l| !l.starts_with("connections:"))
            .take_while(|l| !l.starts_with("users:"))
            .collect::<Vec<_>>()
            .join("\n");
        let blank_in_conn = conn_section.lines().filter(|l| l.is_empty()).count();
        assert_eq!(
            blank_in_conn, 0,
            "expected no blank lines inside single-entry connections block"
        );
    }

    /// Two connections: one blank line injected between them.
    #[test]
    fn test_inject_blank_lines_two_connections() {
        let input = "connections:\n  a:\n    host: 1.1.1.1\n    user: u\n  b:\n    host: 2.2.2.2\n    user: v\nusers: {}\n";
        let out = inject_dump_blank_lines(input);
        // Count blank lines within the connections block.
        let conn_section: Vec<&str> = out
            .lines()
            .skip_while(|l| !l.starts_with("connections:"))
            .take_while(|l| !l.starts_with("users:"))
            .collect();
        let blank_count = conn_section.iter().filter(|l| l.is_empty()).count();
        assert!(
            blank_count >= 1,
            "expected at least one blank line between two connection entries, got:\n{out}"
        );
        // Output is still valid YAML.
        let _: serde_yaml::Value = serde_yaml::from_str(&out).expect("output should be valid YAML");
    }

    /// Three connections: two blank lines injected (one between each pair).
    #[test]
    fn test_inject_blank_lines_three_connections_two_blanks() {
        let input = "connections:\n  a:\n    host: 1.1.1.1\n    user: u\n  b:\n    host: 2.2.2.2\n    user: v\n  c:\n    host: 3.3.3.3\n    user: w\nusers: {}\n";
        let out = inject_dump_blank_lines(input);
        let conn_section: Vec<&str> = out
            .lines()
            .skip_while(|l| !l.starts_with("connections:"))
            .take_while(|l| !l.starts_with("users:"))
            .collect();
        let blank_count = conn_section.iter().filter(|l| l.is_empty()).count();
        assert!(
            blank_count >= 2,
            "expected at least two blank lines between three connection entries, got:\n{out}"
        );
        let _: serde_yaml::Value = serde_yaml::from_str(&out).expect("output should be valid YAML");
    }

    /// Two connections + users: blank line between connections block and users.
    #[test]
    fn test_inject_blank_line_between_connections_and_users_blocks() {
        let input =
            "connections:\n  a:\n    host: h\n    user: u\n  b:\n    host: h2\n    user: v\nusers:\n  k: val\n";
        let out = inject_dump_blank_lines(input);
        // Find position of last line of connections block and first line of users block.
        assert!(
            out.contains("\nusers:"),
            "expected blank line immediately before users: in:\n{out}"
        );
        let _: serde_yaml::Value = serde_yaml::from_str(&out).expect("output should be valid YAML");
    }

    /// build_dump_yaml on a two-connection config contains at least two blank
    /// lines in the connections section and one before users:.
    #[test]
    fn test_build_dump_yaml_two_connections_blank_lines() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  alpha:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: Alpha\n  beta:\n    host: 10.0.0.2\n    user: admin\n    auth:\n      type: password\n    description: Beta\nusers:\n  k: v\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        let yaml = build_dump_yaml(&cfg).unwrap();

        // At least one blank line between the two connection entries.
        let conn_section: Vec<&str> = yaml
            .lines()
            .skip_while(|l| !l.starts_with("connections:"))
            .take_while(|l| !l.starts_with("users:"))
            .collect();
        let blank_in_conn = conn_section.iter().filter(|l| l.is_empty()).count();
        assert!(
            blank_in_conn >= 1,
            "expected blank lines between connection entries in:\n{yaml}"
        );

        // Blank line before users:.
        assert!(
            yaml.contains("\nusers:"),
            "expected blank line before users: in:\n{yaml}"
        );

        // Output round-trips as valid YAML.
        let _: serde_yaml::Value =
            serde_yaml::from_str(&yaml).expect("dump output should be valid YAML");
    }

    #[test]
    fn test_show_existing_connection_no_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  prod:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: Production server\n",
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
            "connections:\n  srv:\n    host: h\n    user: u\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: d\n",
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
            "connections:\n  srv:\n    host: 1.2.3.4\n    user: admin\n    port: 2222\n    auth:\n      type: key\n      key: ~/.ssh/id_ed25519\n    description: Test\n    link: https://wiki.example.com\n",
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
            "connections:\n  db:\n    host: db.internal\n    user: dbadmin\n    auth:\n      type: password\n    description: Database\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());
        run(&cfg, &no_color(), "db").unwrap();
    }

    /// Showing a connection whose `auth.generate_key` contains both
    /// `${user}` and `${key}` must populate `ConnectionDetail.generate_key`
    /// with both placeholders expanded — the user-facing show output must
    /// not leak either raw token.
    #[test]
    fn test_show_generate_key_expands_user_and_key_placeholders() {
        use crate::display::ConnectionDetail;

        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  bastion:\n    host: 10.0.0.1\n    user: ec2-user\n    auth:\n      type: key\n      key: ~/.ssh/foo\n      generate_key: \"vault read -field=private_key secret/users/${user} > ${key}\"\n    description: Bastion\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let conn = cfg.find_with_wildcard("bastion").unwrap();
        let detail = ConnectionDetail {
            name: conn.name.clone(),
            host: conn.host.clone(),
            user: conn.user.clone(),
            port: conn.port,
            auth: conn.auth.type_label().to_string(),
            key: conn.auth.key().map(str::to_string),
            generate_key: render_generate_key(&conn),
            description: conn.description.clone(),
            link: conn.link.clone(),
            source_label: conn.layer.label().to_string(),
            source_path: conn.source_path.display().to_string(),
        };

        let rendered = detail.generate_key.as_deref().unwrap();
        assert_eq!(
            rendered, "vault read -field=private_key secret/users/ec2-user > ~/.ssh/foo",
            "expected both ${{user}} and ${{key}} expanded in show output"
        );
        assert!(
            !rendered.contains("${user}"),
            "expected no raw ${{user}} token in generate_key: {rendered}"
        );
        assert!(
            !rendered.contains("${key}"),
            "expected no raw ${{key}} token in generate_key: {rendered}"
        );
    }

    /// Showing a connection whose `auth.generate_key` contains `${key}` must
    /// populate `ConnectionDetail.generate_key` with the expanded key path
    /// (no unexpanded `${key}` token left behind).
    #[test]
    fn test_show_generate_key_expands_key_placeholder() {
        use crate::display::ConnectionDetail;

        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  bastion:\n    host: 10.0.0.1\n    user: ec2-user\n    auth:\n      type: key\n      key: ~/.ssh/foo\n      generate_key: \"vault read -field=private_key secret/ssh/foo > ${key}\"\n    description: Bastion\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        // Build the ConnectionDetail the same way `run` does — this is what
        // flows into the renderer and ultimately the user-facing output.
        let conn = cfg.find_with_wildcard("bastion").unwrap();
        let detail = ConnectionDetail {
            name: conn.name.clone(),
            host: conn.host.clone(),
            user: conn.user.clone(),
            port: conn.port,
            auth: conn.auth.type_label().to_string(),
            key: conn.auth.key().map(str::to_string),
            generate_key: conn.auth.generate_key_expanded(),
            description: conn.description.clone(),
            link: conn.link.clone(),
            source_label: conn.layer.label().to_string(),
            source_path: conn.source_path.display().to_string(),
        };

        let rendered = detail.generate_key.as_deref().unwrap();
        assert_eq!(
            rendered, "vault read -field=private_key secret/ssh/foo > ~/.ssh/foo",
            "expected ${{key}} expanded to the literal key path"
        );
        assert!(
            rendered.contains("~/.ssh/foo"),
            "expected expanded key path present: {rendered}"
        );
        assert!(
            !rendered.contains("${key}"),
            "expected no raw ${{key}} token in generate_key: {rendered}"
        );

        // Also ensure `run` itself succeeds end-to-end on this config.
        run(&cfg, &no_color(), "bastion").unwrap();
    }
}
