// Handler for `yconn ssh-config generate` — write SSH Host blocks to
// ~/.ssh/yconn-connections and update ~/.ssh/config with an Include line.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::Connection;
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
/// The output contains no trailing newline after the last block.
pub fn render_ssh_config(connections: &[Connection]) -> String {
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
        out.push_str(&format!("    User {}\n", conn.user));
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
    connections: &[Connection],
    _renderer: &Renderer,
    dry_run: bool,
    home: &Path,
) -> Result<()> {
    let content = render_ssh_config(connections);
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
        let out = render_ssh_config(&[conn]);
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
        let out = render_ssh_config(&[conn]);
        assert!(out.contains("Host staging-db\n"));
        assert!(
            !out.contains("IdentityFile"),
            "no IdentityFile for password auth"
        );
    }

    #[test]
    fn test_port_22_omitted() {
        let conn = make_conn("srv", "1.2.3.4", "ops", 22, "password", None);
        let out = render_ssh_config(&[conn]);
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
        let out = render_ssh_config(&[conn]);
        assert!(out.contains("    Port 2222\n"), "custom port must appear");
    }

    #[test]
    fn test_glob_name_rendered_as_ssh_host_pattern() {
        let conn = make_conn("web-*", "${name}.corp.com", "deploy", 22, "password", None);
        let out = render_ssh_config(&[conn]);
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
        let out = render_ssh_config(&[conn]);
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
        let out = render_ssh_config(&[conn]);
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
        let out = render_ssh_config(&[conn]);
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
        let out = render_ssh_config(&[conn]);
        assert!(out.contains("# yconn: link: https://wiki.example.com/srv"));
    }
}
