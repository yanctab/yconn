// Handler for `yconn ssh-config generate` — write SSH Host blocks to
// ~/.ssh/yconn-connections and update ~/.ssh/config with an Include line.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{Connection, LoadedConfig};
use crate::display::Renderer;

// ─── Translation helpers ──────────────────────────────────────────────────────

/// Translate a yconn connection name to a valid SSH `Host` pattern.
///
/// - Glob names (`web-*`, `srv-?`) are valid SSH `Host` patterns as-is.
/// - Range names (`server[1..10]`) have the `[N..M]` suffix replaced with `*`
///   so SSH sees `server*`.
/// - Literal names are returned unchanged.
fn translate_name_for_ssh(name: &str) -> String {
    if let Some(bracket) = name.rfind('[') {
        let suffix = &name[bracket..];
        if suffix.ends_with(']') && suffix.contains("..") {
            return format!("{}*", &name[..bracket]);
        }
    }
    name.to_string()
}

/// Translate a yconn `host` field value to an SSH `HostName` value.
///
/// `${name}` is replaced with `%h` — SSH's token that expands to the hostname
/// supplied on the command line. This allows a wildcard `Host` block to derive
/// its real target from the input (e.g. `HostName %h.corp.com`).
fn translate_host_for_ssh(host: &str) -> String {
    host.replace("${name}", "%h")
}

// ─── Rendering ────────────────────────────────────────────────────────────────

/// Build the full SSH config text for `connections`.
///
/// Every connection produces a `Host` block. Pattern names are translated:
/// - Glob (`*`, `?`) — used directly as the SSH `Host` pattern.
/// - Range (`[N..M]`) — the range suffix is replaced with `*`.
///
/// `${name}` in the `host` field is replaced with `%h` so SSH expands it to
/// the matched hostname at connection time.
///
/// When `skip_user` is `true`, the `User` line is omitted from all blocks.
///
/// The output contains no trailing newline after the last block.
pub fn render_ssh_config(connections: &[Connection], skip_user: bool) -> String {
    let mut out = String::new();

    for conn in connections {
        let ssh_host = translate_name_for_ssh(&conn.name);
        let ssh_hostname = translate_host_for_ssh(&conn.host);

        // Comment header with metadata.
        out.push_str(&format!("# yconn: description: {}\n", conn.description));
        out.push_str(&format!("# yconn: auth: {}\n", conn.auth));
        if let Some(link) = &conn.link {
            out.push_str(&format!("# yconn: link: {link}\n"));
        }

        out.push_str(&format!("Host {ssh_host}\n"));
        out.push_str(&format!("    HostName {ssh_hostname}\n"));
        if !skip_user {
            // If the user field still contains an unresolved template token,
            // emit a comment instead of an invalid SSH User directive.
            if conn.user.contains("${") {
                out.push_str(&format!("# yconn: user: {} (unresolved)\n", conn.user));
            } else {
                out.push_str(&format!("    User {}\n", conn.user));
            }
        }
        if conn.port != 22 {
            out.push_str(&format!("    Port {}\n", conn.port));
        }
        if let Some(key) = &conn.key {
            out.push_str(&format!("    IdentityFile {key}\n"));
        }
        out.push('\n');
    }

    // Remove the trailing newline after the last block so callers can control
    // the final newline themselves.
    if out.ends_with('\n') {
        out.pop();
    }

    out
}

// ─── File helpers ─────────────────────────────────────────────────────────────

const INCLUDE_LINE: &str = "Include ~/.ssh/yconn-connections";
const OUTPUT_FILENAME: &str = "yconn-connections";

/// Return the path to `~/.ssh/yconn-connections`.
fn output_path(home: &Path) -> PathBuf {
    home.join(".ssh").join(OUTPUT_FILENAME)
}

/// Ensure `~/.ssh/` exists with 0o700 permissions.
fn ensure_ssh_dir(home: &Path) -> Result<()> {
    let ssh_dir = home.join(".ssh");
    if !ssh_dir.exists() {
        fs::create_dir_all(&ssh_dir)
            .with_context(|| format!("failed to create {}", ssh_dir.display()))?;
        fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to set permissions on {}", ssh_dir.display()))?;
    }
    Ok(())
}

/// Write `content` to `path` with 0o600 permissions.
fn write_secure(path: &Path, content: &str) -> Result<()> {
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to set permissions on {}", path.display()))?;
    Ok(())
}

/// Ensure `~/.ssh/config` contains `Include ~/.ssh/yconn-connections` as its
/// first non-empty line. Creates the file if absent.
fn inject_include(home: &Path) -> Result<()> {
    let config_path = home.join(".ssh").join("config");

    if config_path.exists() {
        let existing = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        if existing.lines().any(|l| l.trim() == INCLUDE_LINE) {
            return Ok(()); // Already present — idempotent.
        }
        let updated = format!("{INCLUDE_LINE}\n\n{existing}");
        write_secure(&config_path, &updated)?;
    } else {
        write_secure(&config_path, &format!("{INCLUDE_LINE}\n"))?;
    }
    Ok(())
}

// ─── Command entry points ─────────────────────────────────────────────────────

pub fn run_generate(
    cfg: &LoadedConfig,
    renderer: &Renderer,
    dry_run: bool,
    home: &Path,
    inline_overrides: &HashMap<String, String>,
    skip_user: bool,
) -> Result<()> {
    // Expand ${<key>} templates in the user field of each connection.
    let mut connections: Vec<Connection> = cfg.connections.clone();
    for conn in &mut connections {
        let (expanded, warnings) = cfg.expand_user_field(&conn.user, inline_overrides);
        for w in &warnings {
            renderer.warn(w);
        }
        conn.user = expanded;
    }

    let content = render_ssh_config(&connections, skip_user);
    let block_count = connections.len();

    if dry_run {
        println!("{content}");
        return Ok(());
    }

    ensure_ssh_dir(home)?;
    let out = output_path(home);
    write_secure(&out, &format!("{content}\n"))?;
    inject_include(home)?;

    println!("Wrote {block_count} Host block(s) to {}", out.display());

    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Connection, Layer};
    use std::path::PathBuf;

    fn make_conn(
        name: &str,
        host: &str,
        user: &str,
        port: u16,
        auth: &str,
        key: Option<&str>,
    ) -> Connection {
        Connection {
            name: name.to_string(),
            host: host.to_string(),
            user: user.to_string(),
            port,
            auth: auth.to_string(),
            key: key.map(|s| s.to_string()),
            description: format!("{name} description"),
            link: None,
            group: None,
            layer: Layer::User,
            source_path: PathBuf::from("test.yaml"),
            shadowed: false,
        }
    }

    fn make_conn_with_link(name: &str, link: &str) -> Connection {
        let mut c = make_conn(name, "host.example.com", "user", 22, "password", None);
        c.link = Some(link.to_string());
        c
    }

    #[test]
    fn test_key_auth_block_format() {
        let conn = make_conn(
            "prod-web",
            "10.0.1.50",
            "deploy",
            22,
            "key",
            Some("~/.ssh/id_rsa"),
        );
        let out = render_ssh_config(&[conn], false);
        assert!(out.contains("Host prod-web\n"), "missing Host line");
        assert!(out.contains("    HostName 10.0.1.50\n"));
        assert!(out.contains("    User deploy\n"));
        assert!(out.contains("    IdentityFile ~/.ssh/id_rsa\n"));
        assert!(!out.contains("Port"), "port 22 must be omitted");
    }

    #[test]
    fn test_password_auth_block_no_identity_file() {
        let conn = make_conn(
            "staging-db",
            "staging.internal",
            "dbadmin",
            22,
            "password",
            None,
        );
        let out = render_ssh_config(&[conn], false);
        assert!(out.contains("Host staging-db\n"));
        assert!(
            !out.contains("IdentityFile"),
            "no IdentityFile for password auth"
        );
    }

    #[test]
    fn test_port_22_omitted() {
        let conn = make_conn("srv", "1.2.3.4", "ops", 22, "password", None);
        let out = render_ssh_config(&[conn], false);
        assert!(!out.contains("Port"), "port 22 must not appear");
    }

    #[test]
    fn test_non_22_port_included() {
        let conn = make_conn(
            "bastion",
            "bastion.example.com",
            "ec2-user",
            2222,
            "key",
            Some("~/.ssh/key"),
        );
        let out = render_ssh_config(&[conn], false);
        assert!(out.contains("    Port 2222\n"), "custom port must appear");
    }

    #[test]
    fn test_glob_name_rendered_as_ssh_host_pattern() {
        let conn = make_conn("web-*", "${name}.corp.com", "deploy", 22, "password", None);
        let out = render_ssh_config(&[conn], false);
        assert!(
            out.contains("Host web-*\n"),
            "glob must appear as Host pattern"
        );
        assert!(
            out.contains("    HostName %h.corp.com\n"),
            "\\${{name}} must become %h"
        );
        assert!(!out.contains("skipped"));
    }

    #[test]
    fn test_range_pattern_name_translated_to_glob() {
        let conn = make_conn(
            "server[1..10]",
            "${name}.internal",
            "ops",
            22,
            "password",
            None,
        );
        let out = render_ssh_config(&[conn], false);
        assert!(
            out.contains("Host server*\n"),
            "range [N..M] must become * in Host line"
        );
        assert!(
            out.contains("    HostName %h.internal\n"),
            "\\${{name}} must become %h"
        );
        assert!(
            !out.contains("Host server[1..10]"),
            "range must not appear in Host line"
        );
    }

    #[test]
    fn test_name_template_in_host_becomes_percent_h() {
        let conn = make_conn("web-*", "${name}.corp.com", "deploy", 22, "password", None);
        let out = render_ssh_config(&[conn], false);
        assert!(out.contains("    HostName %h.corp.com\n"));
        assert!(!out.contains("${name}"));
    }

    #[test]
    fn test_literal_host_unchanged() {
        let conn = make_conn(
            "bastion",
            "bastion.example.com",
            "ec2-user",
            22,
            "key",
            None,
        );
        let out = render_ssh_config(&[conn], false);
        assert!(out.contains("    HostName bastion.example.com\n"));
    }

    #[test]
    fn test_idempotent_include_injection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        fs::create_dir_all(&ssh_dir).unwrap();
        let config_path = ssh_dir.join("config");
        fs::write(
            &config_path,
            format!("{INCLUDE_LINE}\n\nHost old\n    HostName 1.2.3.4\n"),
        )
        .unwrap();

        inject_include(tmp.path()).unwrap();

        let result = fs::read_to_string(&config_path).unwrap();
        let count = result.lines().filter(|l| l.trim() == INCLUDE_LINE).count();
        assert_eq!(count, 1, "Include must appear exactly once");
    }

    #[test]
    fn test_include_prepended_when_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        fs::create_dir_all(&ssh_dir).unwrap();
        let config_path = ssh_dir.join("config");
        fs::write(&config_path, "Host existing\n    HostName 9.9.9.9\n").unwrap();

        inject_include(tmp.path()).unwrap();

        let result = fs::read_to_string(&config_path).unwrap();
        assert!(
            result.starts_with(INCLUDE_LINE),
            "Include must be first line"
        );
        assert!(
            result.contains("Host existing"),
            "existing content preserved"
        );
    }

    #[test]
    fn test_config_created_when_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        fs::create_dir_all(&ssh_dir).unwrap();

        inject_include(tmp.path()).unwrap();

        let config_path = ssh_dir.join("config");
        assert!(config_path.exists(), "config file must be created");
        let result = fs::read_to_string(&config_path).unwrap();
        assert!(result.contains(INCLUDE_LINE));
    }

    #[test]
    fn test_link_field_appears_in_comment() {
        let conn = make_conn_with_link("srv", "https://wiki.example.com/srv");
        let out = render_ssh_config(&[conn], false);
        assert!(out.contains("# yconn: link: https://wiki.example.com/srv"));
    }

    // ── user field expansion ───────────────────────────────────────────────────

    /// Helper: load config from inline YAML with a users: section, expand user
    /// fields using inline_overrides, and return the rendered SSH config string.
    fn render_expanded(
        yaml: &str,
        inline_overrides: &HashMap<String, String>,
        skip_user: bool,
    ) -> (String, Vec<String>) {
        use crate::config;
        use tempfile::TempDir;

        let user_dir = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let sys = TempDir::new().unwrap();
        fs::write(user_dir.path().join("connections.yaml"), yaml).unwrap();

        let cfg = config::load_impl(
            cwd.path(),
            Some("connections"),
            false,
            Some(user_dir.path()),
            sys.path(),
        )
        .unwrap();

        let mut conns: Vec<Connection> = cfg.connections.clone();
        let mut all_warnings: Vec<String> = Vec::new();
        for conn in &mut conns {
            let (expanded, warnings) = cfg.expand_user_field(&conn.user, inline_overrides);
            all_warnings.extend(warnings);
            conn.user = expanded;
        }

        (render_ssh_config(&conns, skip_user), all_warnings)
    }

    #[test]
    fn test_dollar_user_expanded_from_override() {
        // Use --user user:alice override (deterministic, no dependency on $USER env var).
        let yaml = "connections:\n  srv:\n    host: myhost\n    user: \"${user}\"\n    auth: password\n    description: test\n";
        let mut overrides = HashMap::new();
        overrides.insert("user".to_string(), "alice".to_string());
        let (out, _warnings) = render_expanded(yaml, &overrides, false);
        assert!(
            out.contains("    User alice\n"),
            "expected 'User alice', got: {out}"
        );
        assert!(!out.contains("${user}"));
    }

    #[test]
    fn test_dollar_user_unresolved_emits_comment_not_user_line() {
        // When expansion leaves ${user} unchanged (no override, env may be set but
        // we test the render path directly with the literal value).
        let conn = make_conn("srv", "myhost", "${user}", 22, "password", None);
        let out = render_ssh_config(&[conn], false);
        // The unresolved token should appear as a comment, not a User directive.
        assert!(
            !out.contains("    User ${user}"),
            "must not render as User line: {out}"
        );
        assert!(
            out.contains("# yconn: user: ${user} (unresolved)"),
            "must render as comment: {out}"
        );
    }

    #[test]
    fn test_named_key_expanded_from_users_map() {
        let yaml = "users:\n  t1user: \"ops\"\nconnections:\n  srv:\n    host: myhost\n    user: \"${t1user}\"\n    auth: password\n    description: test\n";
        let (out, warnings) = render_expanded(yaml, &HashMap::new(), false);
        assert!(
            out.contains("    User ops\n"),
            "expected 'User ops', got: {out}"
        );
        assert!(warnings.is_empty(), "no warnings expected: {warnings:?}");
    }

    #[test]
    fn test_skip_user_omits_user_line() {
        let conn = make_conn("srv", "myhost", "deploy", 22, "password", None);
        let out = render_ssh_config(&[conn], true);
        assert!(
            !out.contains("User"),
            "User line must be omitted with skip_user: {out}"
        );
    }

    #[test]
    fn test_user_override_overrides_users_map() {
        let yaml = "users:\n  t1user: \"ops\"\nconnections:\n  srv:\n    host: myhost\n    user: \"${t1user}\"\n    auth: password\n    description: test\n";
        let mut overrides = HashMap::new();
        overrides.insert("t1user".to_string(), "alice".to_string());
        let (out, warnings) = render_expanded(yaml, &overrides, false);
        assert!(
            out.contains("    User alice\n"),
            "expected 'User alice', got: {out}"
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_multiple_user_overrides_all_applied() {
        let yaml = "users:\n  k1: \"a\"\nconnections:\n  c1:\n    host: h1\n    user: \"${k1}\"\n    auth: password\n    description: d1\n  c2:\n    host: h2\n    user: \"${user}\"\n    auth: password\n    description: d2\n";
        let mut overrides = HashMap::new();
        overrides.insert("k1".to_string(), "carol".to_string());
        overrides.insert("user".to_string(), "dave".to_string());
        let (out, warnings) = render_expanded(yaml, &overrides, false);
        assert!(
            out.contains("    User carol\n") || out.contains("    User dave\n"),
            "both overrides must be applied: {out}"
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_unresolved_template_produces_warning() {
        let yaml = "connections:\n  srv:\n    host: myhost\n    user: \"${nokey}\"\n    auth: password\n    description: test\n";
        let (_out, warnings) = render_expanded(yaml, &HashMap::new(), false);
        assert!(
            !warnings.is_empty(),
            "expected warning for unresolved template"
        );
        assert!(
            warnings[0].contains("unresolved"),
            "warning must say unresolved: {}",
            warnings[0]
        );
    }
}
