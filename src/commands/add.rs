// src/commands/add.rs
// Handler for `yconn add` — interactive wizard to add a connection to a
// chosen layer.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Layer;

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn run(layer: Option<&str>) -> Result<()> {
    let target_layer = resolve_layer(layer)?;
    let target_path = layer_path(target_layer)?;

    let stdin = io::stdin();
    let stdout = io::stdout();
    run_impl(
        target_layer,
        &target_path,
        &mut stdin.lock(),
        &mut stdout.lock(),
    )
}

// ─── Layer resolution ─────────────────────────────────────────────────────────

fn resolve_layer(layer: Option<&str>) -> Result<Layer> {
    match layer {
        Some("system") => Ok(Layer::System),
        Some("user") | None => Ok(Layer::User),
        Some("project") => Ok(Layer::Project),
        Some(other) => bail!("unknown layer '{other}'; use system, user, or project"),
    }
}

fn layer_path(layer: Layer) -> Result<PathBuf> {
    match layer {
        Layer::System => Ok(PathBuf::from("/etc/yconn")),
        Layer::User => {
            let base = dirs::config_dir().context("cannot determine user config directory")?;
            Ok(base.join("yconn"))
        }
        Layer::Project => {
            let cwd = std::env::current_dir().context("cannot determine current directory")?;
            Ok(cwd.join(".yconn"))
        }
    }
}

// ─── Testable impl ────────────────────────────────────────────────────────────

pub(crate) fn run_impl(
    layer: Layer,
    layer_dir: &Path,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<()> {
    let group = crate::group::active_group()
        .context("cannot determine active group")?
        .name;

    let target = layer_dir.join(format!("{group}.yaml"));

    writeln!(
        output,
        "Adding connection to {} layer ({})",
        layer.label(),
        target.display()
    )?;
    writeln!(output, "Leave a field blank to abort.")?;
    writeln!(output)?;

    // Collect required fields.
    let name = prompt(output, input, "Connection name")?;
    if name.is_empty() {
        bail!("aborted");
    }
    let host = prompt(output, input, "Host")?;
    if host.is_empty() {
        bail!("aborted");
    }
    let user = prompt(output, input, "User")?;
    if user.is_empty() {
        bail!("aborted");
    }
    let port_raw = prompt(output, input, "Port [22]")?;
    let port: u16 = if port_raw.is_empty() {
        22
    } else {
        port_raw
            .parse()
            .with_context(|| format!("invalid port '{port_raw}'"))?
    };
    let auth = prompt_choice(output, input, "Auth", &["key", "password"])?;
    if auth.is_empty() {
        bail!("aborted");
    }
    let key = if auth == "key" {
        let k = prompt(output, input, "Key path (e.g. ~/.ssh/id_rsa)")?;
        if k.is_empty() {
            bail!("aborted");
        }
        Some(k)
    } else {
        None
    };
    let description = prompt(output, input, "Description")?;
    if description.is_empty() {
        bail!("aborted");
    }
    let link = prompt(output, input, "Link (optional)")?;

    // Build the YAML entry.
    let entry = build_entry(
        &host,
        &user,
        port,
        &auth,
        key.as_deref(),
        &description,
        if link.is_empty() {
            None
        } else {
            Some(link.as_str())
        },
    );

    write_entry(&target, &name, &entry)?;

    writeln!(output)?;
    writeln!(output, "Added '{name}' to {}", target.display())?;

    Ok(())
}

// ─── Prompt helpers ───────────────────────────────────────────────────────────

fn prompt(output: &mut dyn Write, input: &mut dyn BufRead, label: &str) -> Result<String> {
    write!(output, "  {label}: ")?;
    output.flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn prompt_choice(
    output: &mut dyn Write,
    input: &mut dyn BufRead,
    label: &str,
    choices: &[&str],
) -> Result<String> {
    let options = choices.join("/");
    loop {
        let answer = prompt(output, input, &format!("{label} [{options}]"))?;
        if answer.is_empty() {
            return Ok(answer);
        }
        if choices.contains(&answer.as_str()) {
            return Ok(answer);
        }
        writeln!(output, "  Please enter one of: {options}")?;
    }
}

// ─── YAML construction ────────────────────────────────────────────────────────

fn build_entry(
    host: &str,
    user: &str,
    port: u16,
    auth: &str,
    key: Option<&str>,
    description: &str,
    link: Option<&str>,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("  host: {}\n", host));
    s.push_str(&format!("  user: {}\n", user));
    if port != 22 {
        s.push_str(&format!("  port: {}\n", port));
    }
    s.push_str(&format!("  auth: {}\n", auth));
    if let Some(k) = key {
        s.push_str(&format!("  key: {}\n", k));
    }
    s.push_str(&format!(
        "  description: \"{}\"\n",
        description.replace('"', "\\\"")
    ));
    if let Some(l) = link {
        s.push_str(&format!("  link: {}\n", l));
    }
    s
}

/// Append (or create) the entry in the target YAML file under `connections:`.
fn write_entry(target: &Path, name: &str, entry: &str) -> Result<()> {
    // Ensure the directory exists.
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    if target.exists() {
        // Read existing content and insert the new entry under `connections:`.
        let existing = std::fs::read_to_string(target)
            .with_context(|| format!("failed to read {}", target.display()))?;

        // Check for name collision.
        if entry_exists(&existing, name) {
            bail!("connection '{name}' already exists in {}", target.display());
        }

        let updated = insert_connection(&existing, name, entry);
        std::fs::write(target, updated)
            .with_context(|| format!("failed to write {}", target.display()))?;
    } else {
        // Create a minimal file with just this connection.
        let content = format!("version: 1\n\nconnections:\n  {name}:\n{entry}");
        std::fs::write(target, content)
            .with_context(|| format!("failed to write {}", target.display()))?;
    }

    Ok(())
}

/// Return `true` if a connections key named `name` already appears in `content`.
fn entry_exists(content: &str, name: &str) -> bool {
    // Simple line-based scan: look for "  <name>:" at the start of a line.
    let pattern = format!("  {name}:");
    content
        .lines()
        .any(|l| l == pattern || l.starts_with(&format!("{pattern} ")))
}

/// Insert `name: <entry>` under the `connections:` key of `content`.
///
/// If a `connections:` line is found we append after the last line of
/// the connections block (i.e. at end of file). Otherwise we append a
/// new `connections:` section at the end.
fn insert_connection(content: &str, name: &str, entry: &str) -> String {
    let new_block = format!("  {name}:\n{entry}");

    // If there is already a `connections:` key, append to the end of the file.
    if content
        .lines()
        .any(|l| l == "connections:" || l.starts_with("connections:"))
    {
        let trimmed = content.trim_end();
        format!("{trimmed}\n{new_block}\n")
    } else {
        // Append a new connections section.
        let trimmed = content.trim_end();
        format!("{trimmed}\n\nconnections:\n{new_block}\n")
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_yaml(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    /// Simulate user input through run_impl.
    fn run_with_input(layer: Layer, layer_dir: &Path, answers: &[&str]) -> Result<String> {
        let input_str = answers.join("\n") + "\n";
        let mut input = input_str.as_bytes();
        let mut output = Vec::new();
        run_impl(layer, layer_dir, &mut input, &mut output)?;
        Ok(String::from_utf8(output).unwrap())
    }

    // ── layer resolution ──────────────────────────────────────────────────────

    #[test]
    fn test_resolve_layer_none_defaults_to_user() {
        assert!(matches!(resolve_layer(None).unwrap(), Layer::User));
    }

    #[test]
    fn test_resolve_layer_user() {
        assert!(matches!(resolve_layer(Some("user")).unwrap(), Layer::User));
    }

    #[test]
    fn test_resolve_layer_project() {
        assert!(matches!(
            resolve_layer(Some("project")).unwrap(),
            Layer::Project
        ));
    }

    #[test]
    fn test_resolve_layer_system() {
        assert!(matches!(
            resolve_layer(Some("system")).unwrap(),
            Layer::System
        ));
    }

    #[test]
    fn test_resolve_layer_unknown_returns_error() {
        let err = resolve_layer(Some("bogus")).unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    // ── add creates new file ──────────────────────────────────────────────────

    #[test]
    fn test_add_creates_new_file_with_connection() {
        let dir = TempDir::new().unwrap();
        // key auth: name, host, user, port, auth, key, description, link
        let answers = [
            "myconn",
            "10.0.0.1",
            "deploy",
            "",
            "key",
            "~/.ssh/id_rsa",
            "My server",
            "",
        ];
        run_with_input(Layer::User, dir.path(), &answers).unwrap();

        let target = dir.path().join("connections.yaml");
        assert!(target.exists());
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("myconn:"));
        assert!(content.contains("host: 10.0.0.1"));
        assert!(content.contains("user: deploy"));
        assert!(content.contains("auth: key"));
        assert!(content.contains("key: ~/.ssh/id_rsa"));
        assert!(content.contains("description:"));
    }

    #[test]
    fn test_add_password_auth_no_key_field() {
        let dir = TempDir::new().unwrap();
        // password auth: name, host, user, port, auth, description, link
        let answers = [
            "dbconn",
            "db.internal",
            "dbadmin",
            "",
            "password",
            "Database",
            "",
        ];
        run_with_input(Layer::User, dir.path(), &answers).unwrap();

        let content = fs::read_to_string(dir.path().join("connections.yaml")).unwrap();
        assert!(content.contains("auth: password"));
        assert!(!content.contains("key:"));
    }

    #[test]
    fn test_add_custom_port_included() {
        let dir = TempDir::new().unwrap();
        let answers = [
            "sshbox",
            "host.example.com",
            "admin",
            "2222",
            "key",
            "~/.ssh/k",
            "Box",
            "",
        ];
        run_with_input(Layer::User, dir.path(), &answers).unwrap();

        let content = fs::read_to_string(dir.path().join("connections.yaml")).unwrap();
        assert!(content.contains("port: 2222"));
    }

    #[test]
    fn test_add_default_port_22_omitted() {
        let dir = TempDir::new().unwrap();
        let answers = [
            "srv", "1.2.3.4", "root", "", "key", "~/.ssh/k", "Server", "",
        ];
        run_with_input(Layer::User, dir.path(), &answers).unwrap();

        let content = fs::read_to_string(dir.path().join("connections.yaml")).unwrap();
        assert!(!content.contains("port: 22"));
    }

    #[test]
    fn test_add_appends_to_existing_file() {
        let dir = TempDir::new().unwrap();
        write_yaml(
            dir.path(),
            "connections.yaml",
            "version: 1\n\nconnections:\n  existing:\n    host: h\n    user: u\n    auth: key\n    description: \"d\"\n",
        );

        let answers = [
            "newconn",
            "2.2.2.2",
            "user2",
            "",
            "password",
            "New server",
            "",
        ];
        run_with_input(Layer::User, dir.path(), &answers).unwrap();

        let content = fs::read_to_string(dir.path().join("connections.yaml")).unwrap();
        assert!(content.contains("existing:"));
        assert!(content.contains("newconn:"));
    }

    #[test]
    fn test_add_duplicate_name_returns_error() {
        let dir = TempDir::new().unwrap();
        write_yaml(
            dir.path(),
            "connections.yaml",
            "version: 1\n\nconnections:\n  myconn:\n    host: h\n    user: u\n    auth: key\n    description: \"d\"\n",
        );

        let answers = ["myconn", "other.host", "user", "", "password", "Dup", ""];
        let err = run_with_input(Layer::User, dir.path(), &answers).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_add_empty_name_aborts() {
        let dir = TempDir::new().unwrap();
        // First answer is name = "" → abort
        let answers = [""];
        let err = run_with_input(Layer::User, dir.path(), &answers).unwrap_err();
        assert!(err.to_string().contains("aborted"));
    }

    // ── layer targeting ───────────────────────────────────────────────────────

    #[test]
    fn test_add_to_project_layer_creates_in_yconn_dir() {
        let dir = TempDir::new().unwrap();
        let yconn = dir.path().join(".yconn");
        // The layer_dir IS the yconn dir for project layer.
        let answers = [
            "proj-conn",
            "10.1.1.1",
            "ops",
            "",
            "password",
            "Proj server",
            "",
        ];
        run_with_input(Layer::Project, &yconn, &answers).unwrap();

        let target = yconn.join("connections.yaml");
        assert!(target.exists());
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("proj-conn:"));
    }

    // ── insert_connection helper ──────────────────────────────────────────────

    #[test]
    fn test_insert_connection_appends_under_existing_connections_key() {
        let content = "version: 1\n\nconnections:\n  a:\n    host: h\n";
        let entry = "  host: newhost\n  user: u\n  auth: key\n  description: \"d\"\n";
        let result = insert_connection(content, "b", entry);
        assert!(result.contains("a:"));
        assert!(result.contains("b:"));
        assert!(result.contains("newhost"));
    }

    #[test]
    fn test_insert_connection_adds_connections_section_when_missing() {
        let content = "version: 1\n";
        let entry = "  host: h\n  user: u\n  auth: key\n  description: \"d\"\n";
        let result = insert_connection(content, "srv", entry);
        assert!(result.contains("connections:"));
        assert!(result.contains("srv:"));
    }

    // ── entry_exists helper ───────────────────────────────────────────────────

    #[test]
    fn test_entry_exists_detects_existing_name() {
        let content = "connections:\n  myconn:\n    host: h\n";
        assert!(entry_exists(content, "myconn"));
    }

    #[test]
    fn test_entry_exists_returns_false_when_absent() {
        let content = "connections:\n  other:\n    host: h\n";
        assert!(!entry_exists(content, "myconn"));
    }
}
