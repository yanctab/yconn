//! Permission checks and credential field detection.
//!
//! All functions return [`Warning`] values — they never panic or error on bad
//! input and never write output directly. Callers route warnings through the
//! display module. All checks are non-blocking.

// Public API is consumed by the config module and CLI, not yet implemented.
#![allow(dead_code)]

use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

// ─── Public types ─────────────────────────────────────────────────────────────

/// A non-blocking security warning to be shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub message: String,
}

impl Warning {
    fn new(msg: impl Into<String>) -> Self {
        Warning {
            message: msg.into(),
        }
    }
}

// ─── Constants ────────────────────────────────────────────────────────────────

/// World-readable permission bit (`o+r`).
const WORLD_READABLE: u32 = 0o004;

/// Mask for group+world read/write/execute bits — anything set here is too
/// permissive for an SSH private key.
const KEY_TOO_PERMISSIVE: u32 = 0o077;

/// Field names that indicate a stored credential in a config file.
const CREDENTIAL_KEYS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "credential",
];

// ─── Public API ───────────────────────────────────────────────────────────────

/// Check whether a config file has world-readable permissions.
///
/// Returns a warning if the file mode has the `o+r` bit set.
/// Returns `None` if the file does not exist or its metadata cannot be read.
pub fn check_file_permissions(path: &Path) -> Option<Warning> {
    let mode = file_mode(path)?;
    if mode & WORLD_READABLE != 0 {
        Some(Warning::new(format!(
            "config file {} has world-readable permissions ({:#o}); consider `chmod 600 {}`",
            path.display(),
            mode & 0o777,
            path.display()
        )))
    } else {
        None
    }
}

/// Scan YAML `content` for credential-like field names.
///
/// This is called for **project-layer** config files (`.yconn/`) only — those
/// are git-tracked and must never contain credentials. A warning is emitted for
/// every suspicious key found anywhere in the document.
///
/// Returns an empty `Vec` if the YAML is invalid (fail-safe, not fail-hard).
pub fn check_credential_fields(path: &Path, content: &str) -> Vec<Warning> {
    let value: serde_yaml::Value = match serde_yaml::from_str(content) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut warnings = Vec::new();
    collect_credential_warnings(&value, path, &mut warnings);
    warnings
}

/// Produce a warning that a `docker` block was found in a user-layer config.
///
/// The docker block is only honoured from project (`.yconn/`) and system
/// (`/etc/yconn/`) layers. If it appears in `~/.config/yconn/` it is ignored
/// and this warning is issued.
pub fn check_docker_in_user_layer(path: &Path) -> Warning {
    Warning::new(format!(
        "docker block in user-layer config {} is ignored; \
         docker configuration is only trusted from project (.yconn/) \
         or system (/etc/yconn/) layers",
        path.display()
    ))
}

/// Check that an SSH key file exists and has safe permissions.
///
/// Returns zero or more warnings; does not abort on any individual failure.
pub fn check_key_file(path: &Path) -> Vec<Warning> {
    let mut warnings = Vec::new();

    if !path.exists() {
        warnings.push(Warning::new(format!(
            "key file {} does not exist",
            path.display()
        )));
        return warnings;
    }

    if let Some(mode) = file_mode(path) {
        if mode & KEY_TOO_PERMISSIVE != 0 {
            warnings.push(Warning::new(format!(
                "key file {} has insecure permissions ({:#o}); \
                 SSH may refuse to use it; consider `chmod 600 {}`",
                path.display(),
                mode & 0o777,
                path.display()
            )));
        }
    }

    warnings
}

// ─── Private helpers ─────────────────────────────────────────────────────────

/// Return the Unix file mode for `path`, or `None` on any error.
#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    std::fs::metadata(path).ok().map(|m| m.permissions().mode())
}

/// Non-Unix stub — permission checks are always clean on unsupported platforms.
#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Option<u32> {
    None
}

/// Recursively walk a YAML value and collect warnings for any mapping key that
/// looks like a credential field name.
fn collect_credential_warnings(
    value: &serde_yaml::Value,
    path: &Path,
    warnings: &mut Vec<Warning>,
) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (k, v) in map {
                if let serde_yaml::Value::String(key) = k {
                    let lower = key.to_lowercase();
                    if CREDENTIAL_KEYS.iter().any(|&c| lower == c) {
                        warnings.push(Warning::new(format!(
                            "credential field `{key}` found in git-trackable config {}; \
                             credentials must not be stored in project-layer config files",
                            path.display()
                        )));
                    }
                }
                collect_credential_warnings(v, path, warnings);
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for item in seq {
                collect_credential_warnings(item, path, warnings);
            }
        }
        _ => {}
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn make_file(dir: &TempDir, name: &str, content: &str, mode: u32) -> std::path::PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    // ── check_file_permissions ────────────────────────────────────────────────

    #[test]
    fn test_file_permissions_world_readable_warns() {
        let dir = TempDir::new().unwrap();
        let path = make_file(&dir, "connections.yaml", "", 0o644);
        assert!(check_file_permissions(&path).is_some());
    }

    #[test]
    fn test_file_permissions_private_no_warning() {
        let dir = TempDir::new().unwrap();
        let path = make_file(&dir, "connections.yaml", "", 0o600);
        assert!(check_file_permissions(&path).is_none());
    }

    #[test]
    fn test_file_permissions_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("does-not-exist.yaml");
        assert!(check_file_permissions(&path).is_none());
    }

    #[test]
    fn test_file_permissions_warning_mentions_chmod() {
        let dir = TempDir::new().unwrap();
        let path = make_file(&dir, "connections.yaml", "", 0o644);
        let w = check_file_permissions(&path).unwrap();
        assert!(w.message.contains("chmod"));
    }

    // ── check_credential_fields ───────────────────────────────────────────────

    #[test]
    fn test_credential_fields_clean_yaml() {
        let yaml = "connections:\n  prod:\n    host: 10.0.0.1\n    user: deploy\n    auth: key\n";
        let path = std::path::Path::new("/repo/.yconn/connections.yaml");
        let warnings = check_credential_fields(path, yaml);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_credential_fields_password_field_warns() {
        let yaml = "connections:\n  bad:\n    password: hunter2\n";
        let path = std::path::Path::new("/repo/.yconn/connections.yaml");
        let warnings = check_credential_fields(path, yaml);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("password"));
    }

    #[test]
    fn test_credential_fields_passphrase_warns() {
        let yaml = "connections:\n  bad:\n    passphrase: abc\n";
        let path = std::path::Path::new("/repo/.yconn/connections.yaml");
        let warnings = check_credential_fields(path, yaml);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn test_credential_fields_token_warns() {
        let yaml = "token: ghp_abc123\n";
        let path = std::path::Path::new("/repo/.yconn/connections.yaml");
        let warnings = check_credential_fields(path, yaml);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn test_credential_fields_multiple_warns() {
        let yaml = "connections:\n  bad:\n    password: x\n    secret: y\n";
        let path = std::path::Path::new("/repo/.yconn/connections.yaml");
        let warnings = check_credential_fields(path, yaml);
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn test_credential_fields_invalid_yaml_no_panic() {
        let yaml = ": : : invalid {{ yaml";
        let path = std::path::Path::new("/repo/.yconn/connections.yaml");
        // Must not panic; returns empty vec.
        let warnings = check_credential_fields(path, yaml);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_credential_fields_empty_yaml() {
        let path = std::path::Path::new("/repo/.yconn/connections.yaml");
        let warnings = check_credential_fields(path, "");
        assert!(warnings.is_empty());
    }

    // ── check_docker_in_user_layer ────────────────────────────────────────────

    #[test]
    fn test_docker_in_user_layer_returns_warning() {
        let path = std::path::Path::new("/home/user/.config/yconn/connections.yaml");
        let w = check_docker_in_user_layer(path);
        assert!(w.message.contains("docker"));
        assert!(w.message.contains("ignored"));
    }

    // ── check_key_file ────────────────────────────────────────────────────────

    #[test]
    fn test_key_file_missing_warns() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing_key");
        let warnings = check_key_file(&path);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("does not exist"));
    }

    #[test]
    fn test_key_file_secure_permissions_no_warning() {
        let dir = TempDir::new().unwrap();
        let path = make_file(&dir, "id_rsa", "KEY DATA", 0o600);
        let warnings = check_key_file(&path);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_key_file_too_permissive_warns() {
        let dir = TempDir::new().unwrap();
        let path = make_file(&dir, "id_rsa", "KEY DATA", 0o644);
        let warnings = check_key_file(&path);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("insecure permissions"));
    }

    #[test]
    fn test_key_file_group_readable_warns() {
        let dir = TempDir::new().unwrap();
        let path = make_file(&dir, "id_rsa", "KEY DATA", 0o640);
        let warnings = check_key_file(&path);
        assert_eq!(warnings.len(), 1);
    }
}
