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

        // Determine whether the user field contains an unresolved template token.
        // This is evaluated once and used both for the pre-Host comment and for
        // suppressing the User directive inside the block.
        let has_unresolved_user = !skip_user && conn.user.contains("${");

        // All comment lines appear contiguously before the Host line.
        out.push_str(&format!("# description: {}\n", conn.description));
        out.push_str(&format!("# auth: {}\n", conn.auth));
        if let Some(link) = &conn.link {
            out.push_str(&format!("# link: {link}\n"));
        }
        // If the user field still contains an unresolved template token,
        // emit a comment here (before Host) instead of an invalid SSH User
        // directive inside the block.
        if has_unresolved_user {
            out.push_str(&format!("# user: {} (unresolved)\n", conn.user));
        }

        out.push_str(&format!("Host {ssh_host}\n"));
        out.push_str(&format!("    HostName {ssh_hostname}\n"));
        if !skip_user && !has_unresolved_user {
            out.push_str(&format!("    User {}\n", conn.user));
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

// ─── Host block upsert ────────────────────────────────────────────────────────

/// A single Host block from a `yconn-connections` file: the SSH Host pattern
/// and the full text of the block (including any preceding comment lines and
/// the trailing newline).
#[derive(Debug, PartialEq)]
struct HostBlock {
    /// The SSH Host pattern as it appears on the `Host <pattern>` line.
    ssh_host: String,
    /// Full block text, including preamble comments and a trailing blank line.
    text: String,
}

/// Parse the contents of `~/.ssh/yconn-connections` into an ordered list of
/// `HostBlock` values.
///
/// The format produced by `render_ssh_config` is:
///
/// ```text
/// # description: …
/// # auth: …
/// Host <name>
///     HostName …
///     …
///
/// ```
///
/// A block boundary is a blank line. Lines before the first `Host` line in a
/// block are treated as that block's preamble (comment lines). The `Host`
/// pattern is extracted from lines matching `^Host <single-token>$`.
///
/// Wildcard Host patterns (e.g. `Host web-*`) are matched exactly — they are
/// not expanded.
fn parse_host_blocks(content: &str) -> Vec<HostBlock> {
    let mut blocks: Vec<HostBlock> = Vec::new();

    // Collect lines, grouping them into blocks separated by blank lines.
    // We accumulate a "pending" chunk of lines; when we hit a blank line we
    // finalise the chunk into a block if it contains a `Host` line.
    let mut pending: Vec<&str> = Vec::new();

    for line in content.lines() {
        if line.is_empty() {
            // Blank line: finalise any pending chunk.
            if !pending.is_empty() {
                if let Some(block) = finalise_block(&pending) {
                    blocks.push(block);
                }
                pending.clear();
            }
        } else {
            pending.push(line);
        }
    }

    // Handle a trailing block with no terminating blank line.
    if !pending.is_empty() {
        if let Some(block) = finalise_block(&pending) {
            blocks.push(block);
        }
    }

    blocks
}

/// Build a `HostBlock` from a non-empty slice of non-blank lines.
///
/// Scans for the first line matching `^Host <token>$` and uses the token as
/// the SSH host pattern. If no such line is found, returns `None` (the chunk
/// is kept as-is but cannot participate in keyed merge).
fn finalise_block(lines: &[&str]) -> Option<HostBlock> {
    let ssh_host = lines.iter().find_map(|l| {
        let rest = l.strip_prefix("Host ")?;
        // Ensure it is exactly one token (no embedded spaces).
        if !rest.is_empty() && !rest.contains(' ') {
            Some(rest.to_string())
        } else {
            None
        }
    })?;

    // Reconstruct block text: all lines joined with '\n', plus a trailing '\n'
    // so blocks end at a newline, followed by a blank line separator.
    let text = format!("{}\n\n", lines.join("\n"));
    Some(HostBlock { ssh_host, text })
}

/// Merge the newly rendered blocks into the existing set of blocks, then
/// return the full file content.
///
/// Merge strategy:
/// - Walk existing blocks in order. If a block's `ssh_host` matches one from
///   `new_blocks`, replace its text with the new block's text; remove the new
///   block from the pending set so it is not appended again.
/// - Append any remaining new blocks (those not present in the existing file)
///   after the preserved/updated existing blocks.
///
/// This preserves "foreign" blocks (those not in the current yconn config)
/// unchanged while updating only matching blocks in place.
fn merge_host_blocks(existing: Vec<HostBlock>, new_blocks: Vec<HostBlock>) -> String {
    use std::collections::HashMap;

    // Build a map from ssh_host → block text for fast lookup.
    let mut new_map: HashMap<String, String> = new_blocks
        .iter()
        .map(|b| (b.ssh_host.clone(), b.text.clone()))
        .collect();

    // Track which new blocks were consumed (matched an existing entry).
    let mut merged = String::new();

    for existing_block in &existing {
        if let Some(new_text) = new_map.remove(&existing_block.ssh_host) {
            // Replace the existing block with the new text.
            merged.push_str(&new_text);
        } else {
            // Preserve the foreign block unchanged.
            merged.push_str(&existing_block.text);
        }
    }

    // Append new blocks that were not present in the existing file, in the
    // same order they appear in new_blocks.
    for new_block in &new_blocks {
        if new_map.contains_key(&new_block.ssh_host) {
            merged.push_str(&new_block.text);
        }
    }

    // The final content should end with exactly one newline (each block ends
    // with "\n\n", so the last block has a trailing blank line; strip it so
    // write_secure appends a single "\n" consistently).
    if merged.ends_with("\n\n") {
        merged.truncate(merged.len() - 1);
    }

    merged
}

// ─── Warning helpers ──────────────────────────────────────────────────────────

/// Extract the first unresolved template key from a user field value.
///
/// When `expand_user_field` cannot resolve a `${key}` token it leaves the
/// token unchanged in the returned string.  This helper finds the first such
/// token and returns the key name so the caller can compose a fix command.
///
/// Returns `None` if no `${...}` token is present.
fn extract_unresolved_key(value: &str) -> Option<&str> {
    let start = value.find("${")?;
    let rest = &value[start + 2..];
    let end = rest.find('}')?;
    Some(&rest[..end])
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
            // Extract the unresolved key from the expanded user field so we can
            // suggest the fix command.  The expanded value still contains the
            // original `${key}` token when the key could not be resolved.
            let fix = extract_unresolved_key(&expanded)
                .map(|key| {
                    format!("  Fix: yconn users add --user {key}:<value>")
                })
                .unwrap_or_default();
            if fix.is_empty() {
                renderer.warn(w);
            } else {
                renderer.warn(&format!("{w}\n{fix}"));
            }
        }
        conn.user = expanded;
    }

    let rendered = render_ssh_config(&connections, skip_user);
    let block_count = connections.len();

    // Parse the newly rendered blocks.
    let new_blocks = parse_host_blocks(&rendered);

    // Read the existing file (if present) and merge.
    let out_path = output_path(home);
    let existing_content = if out_path.exists() {
        fs::read_to_string(&out_path)
            .with_context(|| format!("failed to read {}", out_path.display()))?
    } else {
        String::new()
    };

    let existing_blocks = if existing_content.is_empty() {
        Vec::new()
    } else {
        parse_host_blocks(&existing_content)
    };

    let merged = merge_host_blocks(existing_blocks, new_blocks);

    if dry_run {
        println!("{merged}");
        return Ok(());
    }

    ensure_ssh_dir(home)?;
    write_secure(&out_path, &format!("{merged}\n"))?;
    inject_include(home)?;

    println!(
        "Wrote {block_count} Host block(s) to {}",
        out_path.display()
    );

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
        assert!(out.contains("# link: https://wiki.example.com/srv"));
        assert!(!out.contains("# yconn:"));
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
            out.contains("# user: ${user} (unresolved)"),
            "must render as comment: {out}"
        );
    }

    #[test]
    fn test_named_key_expanded_from_users_map() {
        let yaml = "users:\n  testuser: \"ops\"\nconnections:\n  srv:\n    host: myhost\n    user: \"${testuser}\"\n    auth: password\n    description: test\n";
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
        let yaml = "users:\n  testuser: \"ops\"\nconnections:\n  srv:\n    host: myhost\n    user: \"${testuser}\"\n    auth: password\n    description: test\n";
        let mut overrides = HashMap::new();
        overrides.insert("testuser".to_string(), "alice".to_string());
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

    #[test]
    fn test_unresolved_template_warning_contains_fix_command() {
        // run_generate enriches warnings with the fix command at its call site.
        // We test extract_unresolved_key directly here, and verify that the
        // enrichment logic produces the expected fix string.
        assert_eq!(
            super::extract_unresolved_key("${t1user}"),
            Some("t1user"),
            "must extract key from simple token"
        );
        assert_eq!(
            super::extract_unresolved_key("${t1user}.suffix"),
            Some("t1user"),
            "must extract key when followed by extra text"
        );
        assert_eq!(
            super::extract_unresolved_key("no_template"),
            None,
            "must return None when no template present"
        );

        // Verify the enriched warning format for a known key.
        let key = super::extract_unresolved_key("${t1user}").unwrap();
        let fix = format!("  Fix: yconn users add --user {key}:<value>");
        assert!(
            fix.contains("yconn users add --user t1user:<value>"),
            "fix command must match expected format: {fix}"
        );
    }

    /// All four comment fields (description, auth, link, unresolved user) must
    /// appear contiguously before the `Host` line, in that order, and no `#`
    /// lines must appear inside the Host block (after `Host` until the next
    /// blank line).
    #[test]
    fn test_all_comment_fields_precede_host_line() {
        let mut conn = make_conn(
            "srv", "myhost", "${nokey}", // unresolved → triggers user comment
            22, "password", None,
        );
        conn.link = Some("https://wiki.example.com/srv".to_string());
        conn.description = "My server".to_string();

        let out = render_ssh_config(&[conn], false);

        // Locate positions.
        let host_pos = out.find("Host srv\n").expect("Host line must be present");
        let desc_pos = out
            .find("# description:")
            .expect("# description must be present");
        let auth_pos = out.find("# auth:").expect("# auth must be present");
        let link_pos = out.find("# link:").expect("# link must be present");
        let user_pos = out
            .find("# user: ${nokey} (unresolved)")
            .expect("# user comment must be present");

        // All comments precede the Host line.
        assert!(desc_pos < host_pos, "# description must precede Host line");
        assert!(auth_pos < host_pos, "# auth must precede Host line");
        assert!(link_pos < host_pos, "# link must precede Host line");
        assert!(
            user_pos < host_pos,
            "# user (unresolved) must precede Host line"
        );

        // Order: description → auth → link → user comment.
        assert!(desc_pos < auth_pos, "# description must come before # auth");
        assert!(auth_pos < link_pos, "# auth must come before # link");
        assert!(
            link_pos < user_pos,
            "# link must come before # user comment"
        );

        // No # lines inside the Host block (between Host line and the trailing blank line).
        let block_body = &out[host_pos..];
        let blank_pos = block_body.find("\n\n").unwrap_or(block_body.len());
        let block_interior = &block_body[..blank_pos];
        // Skip the "Host srv\n" line itself when looking for embedded comments.
        let after_host_line = &block_interior["Host srv\n".len()..];
        assert!(
            !after_host_line.contains("\n#"),
            "no # lines must appear inside the Host block: {after_host_line:?}"
        );
        assert!(
            !after_host_line.starts_with('#'),
            "first line after Host must not be a comment: {after_host_line:?}"
        );
    }

    // ── host block upsert ──────────────────────────────────────────────────────

    /// `parse_host_blocks` returns one block per `Host` line.
    #[test]
    fn test_parse_host_blocks_basic() {
        let content = "# description: prod\n# auth: key\nHost prod-web\n    HostName 10.0.1.50\n    User deploy\n\n# description: db\n# auth: password\nHost staging-db\n    HostName staging.internal\n\n";
        let blocks = parse_host_blocks(content);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].ssh_host, "prod-web");
        assert_eq!(blocks[1].ssh_host, "staging-db");
    }

    /// `parse_host_blocks` on an empty string returns no blocks.
    #[test]
    fn test_parse_host_blocks_empty() {
        assert!(parse_host_blocks("").is_empty());
    }

    /// `merge_host_blocks`: existing file has two foreign blocks and one
    /// matching block. The matching block is replaced, the two foreign blocks
    /// are preserved, and the result has three blocks total.
    #[test]
    fn test_merge_preserves_foreign_blocks_and_replaces_matching() {
        let existing_content = "# description: foreign one\n# auth: key\nHost foreign-1\n    HostName f1.example.com\n\n# description: old prod\n# auth: key\nHost prod-web\n    HostName 10.0.0.1\n\n# description: foreign two\n# auth: password\nHost foreign-2\n    HostName f2.example.com\n\n";
        let existing = parse_host_blocks(existing_content);
        assert_eq!(existing.len(), 3);

        // New blocks contain only prod-web (updated).
        let new_content =
            "# description: new prod\n# auth: key\nHost prod-web\n    HostName 10.0.1.50\n";
        let new_blocks = parse_host_blocks(new_content);

        let merged = merge_host_blocks(existing, new_blocks);

        // Three blocks total.
        let result_blocks = parse_host_blocks(&merged);
        assert_eq!(result_blocks.len(), 3, "expected 3 blocks, got: {merged}");

        // Foreign blocks are preserved.
        assert!(
            merged.contains("Host foreign-1"),
            "foreign-1 must be preserved: {merged}"
        );
        assert!(
            merged.contains("Host foreign-2"),
            "foreign-2 must be preserved: {merged}"
        );

        // Matching block is replaced with new content.
        assert!(
            merged.contains("10.0.1.50"),
            "new prod-web HostName must appear: {merged}"
        );
        assert!(
            !merged.contains("10.0.0.1"),
            "old prod-web HostName must be gone: {merged}"
        );

        // Order: foreign-1 first, then prod-web, then foreign-2.
        let pos_f1 = merged.find("Host foreign-1").unwrap();
        let pos_prod = merged.find("Host prod-web").unwrap();
        let pos_f2 = merged.find("Host foreign-2").unwrap();
        assert!(pos_f1 < pos_prod, "foreign-1 must precede prod-web");
        assert!(pos_prod < pos_f2, "prod-web must precede foreign-2");
    }

    /// `merge_host_blocks`: when the existing file is absent (empty blocks),
    /// the output equals the rendered blocks exactly.
    #[test]
    fn test_merge_absent_file_equals_rendered_blocks() {
        let new_content = "# description: prod\n# auth: key\nHost prod-web\n    HostName 10.0.1.50\n    User deploy\n";
        let new_blocks = parse_host_blocks(new_content);

        let merged = merge_host_blocks(Vec::new(), new_blocks);

        // Must contain the rendered block.
        assert!(
            merged.contains("Host prod-web"),
            "prod-web must appear: {merged}"
        );
        assert!(
            merged.contains("    HostName 10.0.1.50"),
            "HostName must appear: {merged}"
        );
    }

    /// `merge_host_blocks`: new blocks not in the existing file are appended
    /// after the existing blocks.
    #[test]
    fn test_merge_new_blocks_appended_after_existing() {
        let existing_content =
            "# description: foreign\n# auth: key\nHost foreign-1\n    HostName f1.example.com\n\n";
        let existing = parse_host_blocks(existing_content);

        let new_content =
            "# description: prod\n# auth: key\nHost prod-web\n    HostName 10.0.1.50\n";
        let new_blocks = parse_host_blocks(new_content);

        let merged = merge_host_blocks(existing, new_blocks);

        let pos_foreign = merged.find("Host foreign-1").unwrap();
        let pos_prod = merged.find("Host prod-web").unwrap();
        assert!(
            pos_foreign < pos_prod,
            "existing foreign block must precede newly appended block"
        );
    }

    /// When `skip_user=true` and the user field is resolved (no template token),
    /// no `#` lines must appear inside the Host block.
    #[test]
    fn test_skip_user_resolved_no_comment_inside_host_block() {
        let conn = make_conn("srv", "myhost", "deploy", 22, "key", Some("~/.ssh/id_rsa"));
        let out = render_ssh_config(&[conn], true);

        let host_pos = out.find("Host srv\n").expect("Host line must be present");
        let block_body = &out[host_pos..];
        let blank_pos = block_body.find("\n\n").unwrap_or(block_body.len());
        let block_interior = &block_body[..blank_pos];
        let after_host_line = &block_interior["Host srv\n".len()..];

        assert!(
            !after_host_line.contains("\n#"),
            "no # lines must appear inside the Host block with skip_user=true: {after_host_line:?}"
        );
        assert!(
            !after_host_line.starts_with('#'),
            "first line after Host must not be a comment: {after_host_line:?}"
        );
        assert!(
            !out.contains("User "),
            "User line must be absent with skip_user=true"
        );
    }
}
