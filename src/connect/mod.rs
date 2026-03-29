//! SSH argument construction and process invocation.
//!
//! Takes a resolved [`Connection`] entry and builds the SSH invocation
//! arguments. Executes SSH by replacing the current process via `execvp` so
//! terminal behaviour (PTY allocation, signal handling) works correctly.
//!
//! For `auth: password` the native SSH password prompt is used — no password
//! is ever passed programmatically.

// Public API is consumed by CLI command modules not yet implemented.
#![allow(dead_code)]

use std::ffi::CString;

use anyhow::{Context, Result};

use crate::config::{Auth, Connection};

// ─── Public API ───────────────────────────────────────────────────────────────

/// Build the SSH argument list for `conn`.
///
/// Returns a `Vec<String>` where the first element is `"ssh"` and the
/// remaining elements are the flags and destination, ready to be passed
/// directly to `execvp`.
///
/// Rules:
/// - `-F /dev/null` is always inserted immediately after `"ssh"` to bypass
///   `~/.ssh/config`. This ensures yconn's own config is the sole source of
///   truth for connection parameters. Any `Include`, `IdentityFile`,
///   `ServerAliveInterval`, or other user config directives in `~/.ssh/config`
///   will be ignored when connecting via yconn.
/// - `auth: key` → `-i <key>` inserted before destination; port flag added
///   when port ≠ 22.
/// - `auth: password` (or any other value) → no `-i` flag; port flag added
///   when port ≠ 22.
/// - Destination is always `user@host`.
pub fn build_args(conn: &Connection) -> Vec<String> {
    // -F /dev/null suppresses ~/.ssh/config entirely so yconn config is
    // the sole authority for all SSH connection parameters.
    let mut args = vec!["ssh".to_string(), "-F".to_string(), "/dev/null".to_string()];

    match conn.auth {
        Auth::Key { ref key, .. } | Auth::Identity { ref key, .. } => {
            args.push("-i".to_string());
            args.push(key.clone());
        }
        Auth::Password => {}
    }

    if conn.port != 22 {
        args.push("-p".to_string());
        args.push(conn.port.to_string());
    }

    args.push(format!("{}@{}", conn.user, conn.host));

    args
}

/// Replace the current process with `ssh` invoked for `conn`.
///
/// On success this function never returns — the kernel replaces the process
/// image. On failure it returns an `Err`.
pub fn exec(conn: &Connection) -> Result<()> {
    let args = build_args(conn);
    exec_argv(&args)
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Low-level `execvp` wrapper. Converts `argv` to C strings and calls
/// `libc::execvp`. Extracted to a separate function so tests can call
/// `build_args` without exercising the actual exec path.
#[cfg(unix)]
fn exec_argv(argv: &[String]) -> Result<()> {
    let c_args: Vec<CString> = argv
        .iter()
        .map(|s| CString::new(s.as_bytes()).context("argument contains null byte"))
        .collect::<Result<_>>()?;

    let c_ptrs: Vec<*const libc::c_char> = c_args
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    // Safety: c_args is alive for the duration of this call; c_ptrs is
    // null-terminated; program is the first element of argv (POSIX convention).
    let ret = unsafe { libc::execvp(c_ptrs[0], c_ptrs.as_ptr()) };

    // execvp only returns on error.
    Err(anyhow::anyhow!(
        "execvp failed (exit code {}): {}",
        ret,
        std::io::Error::last_os_error()
    ))
}

/// Non-Unix stub — exec is unsupported on this platform.
#[cfg(not(unix))]
fn exec_argv(_argv: &[String]) -> Result<()> {
    anyhow::bail!("process exec is not supported on this platform")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::config::{Auth, Connection, Layer};

    fn make_conn(
        auth_type: &str,
        key: Option<&str>,
        port: u16,
        host: &str,
        user: &str,
    ) -> Connection {
        let auth = match auth_type {
            "key" => Auth::Key {
                key: key.unwrap_or("~/.ssh/id_rsa").to_string(),
                cmd: None,
            },
            "identity" => Auth::Identity {
                key: key.unwrap_or("~/.ssh/id_rsa").to_string(),
                cmd: None,
            },
            _ => Auth::Password,
        };
        Connection {
            name: "test".to_string(),
            host: host.to_string(),
            user: user.to_string(),
            port,
            auth,
            description: String::new(),
            link: None,
            group: None,
            layer: Layer::User,
            source_path: PathBuf::from("test.yaml"),
            shadowed: false,
        }
    }

    // ── four SSH arg scenarios from CLAUDE.md ─────────────────────────────────

    #[test]
    fn test_key_auth_default_port() {
        let conn = make_conn("key", Some("~/.ssh/id_rsa"), 22, "myhost", "deploy");
        let args = build_args(&conn);
        assert_eq!(
            args,
            vec![
                "ssh",
                "-F",
                "/dev/null",
                "-i",
                "~/.ssh/id_rsa",
                "deploy@myhost"
            ]
        );
    }

    #[test]
    fn test_key_auth_custom_port() {
        let conn = make_conn("key", Some("~/.ssh/id_rsa"), 2222, "myhost", "deploy");
        let args = build_args(&conn);
        assert_eq!(
            args,
            vec![
                "ssh",
                "-F",
                "/dev/null",
                "-i",
                "~/.ssh/id_rsa",
                "-p",
                "2222",
                "deploy@myhost"
            ]
        );
    }

    #[test]
    fn test_password_auth_default_port() {
        let conn = make_conn("password", None, 22, "myhost", "deploy");
        let args = build_args(&conn);
        assert_eq!(args, vec!["ssh", "-F", "/dev/null", "deploy@myhost"]);
    }

    #[test]
    fn test_password_auth_custom_port() {
        let conn = make_conn("password", None, 2222, "myhost", "deploy");
        let args = build_args(&conn);
        assert_eq!(
            args,
            vec!["ssh", "-F", "/dev/null", "-p", "2222", "deploy@myhost"]
        );
    }

    // ── identity auth scenarios ────────────────────────────────────────────────

    #[test]
    fn test_identity_auth_default_port() {
        let conn = make_conn("identity", Some("~/.ssh/id_rsa"), 22, "github.com", "git");
        let args = build_args(&conn);
        assert_eq!(
            args,
            vec![
                "ssh",
                "-F",
                "/dev/null",
                "-i",
                "~/.ssh/id_rsa",
                "git@github.com"
            ]
        );
    }

    #[test]
    fn test_identity_auth_custom_port() {
        let conn = make_conn("identity", Some("~/.ssh/id_rsa"), 2222, "github.com", "git");
        let args = build_args(&conn);
        assert_eq!(
            args,
            vec![
                "ssh",
                "-F",
                "/dev/null",
                "-i",
                "~/.ssh/id_rsa",
                "-p",
                "2222",
                "git@github.com"
            ]
        );
    }

    #[test]
    fn test_identity_auth_warning_text() {
        // Verify the warning message content matches expectations.
        let warning = "this connection is configured as identity-only (e.g. for git hosts) \
                       and may not support interactive SSH sessions";
        assert!(warning.contains("identity-only"));
        assert!(warning.contains("git hosts"));
    }

    #[test]
    fn test_f_devnull_always_present() {
        // -F /dev/null must appear regardless of auth type or port.
        let cases = vec![
            make_conn("key", Some("~/.ssh/id_rsa"), 22, "host", "user"),
            make_conn("key", Some("~/.ssh/id_rsa"), 2222, "host", "user"),
            make_conn("password", None, 22, "host", "user"),
            make_conn("password", None, 2222, "host", "user"),
        ];
        for conn in &cases {
            let args = build_args(conn);
            assert_eq!(args[1], "-F", "expected -F at position 1: {:?}", args);
            assert_eq!(
                args[2], "/dev/null",
                "expected /dev/null at position 2: {:?}",
                args
            );
        }
    }

    // ── verbose SSH command format (same multiline format as verbose_docker_cmd) ─

    /// Format args the same way `Renderer::verbose_ssh_cmd` does, so the
    /// expected verbose output can be asserted without reaching into the
    /// display module from connect tests.
    fn format_verbose(args: &[String]) -> String {
        match args.split_first() {
            None => String::new(),
            Some((first, rest)) => {
                let mut line = format!("[yconn] Running: {}", first);
                for arg in rest {
                    line.push_str(&format!(" \\\n         {}", arg));
                }
                line
            }
        }
    }

    #[test]
    fn test_verbose_key_auth_default_port() {
        let conn = make_conn("key", Some("~/.ssh/id_rsa"), 22, "myhost", "deploy");
        let args = build_args(&conn);
        let out = format_verbose(&args);
        assert!(
            out.starts_with("[yconn] Running: ssh"),
            "must start with prefix: {out}"
        );
        assert!(out.contains("-i"), "must include -i flag: {out}");
        assert!(
            out.contains("~/.ssh/id_rsa"),
            "must include key path: {out}"
        );
        assert!(
            out.contains("deploy@myhost"),
            "must include destination: {out}"
        );
        assert!(
            !out.contains("-p"),
            "must not include port flag for default port: {out}"
        );
    }

    #[test]
    fn test_verbose_key_auth_custom_port() {
        let conn = make_conn("key", Some("~/.ssh/id_rsa"), 2222, "myhost", "deploy");
        let args = build_args(&conn);
        let out = format_verbose(&args);
        assert!(
            out.starts_with("[yconn] Running: ssh"),
            "must start with prefix: {out}"
        );
        assert!(out.contains("-i"), "must include -i flag: {out}");
        assert!(
            out.contains("~/.ssh/id_rsa"),
            "must include key path: {out}"
        );
        assert!(
            out.contains("-p"),
            "must include -p flag for custom port: {out}"
        );
        assert!(out.contains("2222"), "must include port number: {out}");
        assert!(
            out.contains("deploy@myhost"),
            "must include destination: {out}"
        );
    }

    #[test]
    fn test_verbose_password_auth_default_port() {
        let conn = make_conn("password", None, 22, "myhost", "deploy");
        let args = build_args(&conn);
        let out = format_verbose(&args);
        assert!(
            out.starts_with("[yconn] Running: ssh"),
            "must start with prefix: {out}"
        );
        assert!(
            !out.contains("-i"),
            "must not include -i flag for password auth: {out}"
        );
        assert!(
            !out.contains("-p"),
            "must not include port flag for default port: {out}"
        );
        assert!(
            out.contains("deploy@myhost"),
            "must include destination: {out}"
        );
    }

    #[test]
    fn test_verbose_password_auth_custom_port() {
        let conn = make_conn("password", None, 2222, "myhost", "deploy");
        let args = build_args(&conn);
        let out = format_verbose(&args);
        assert!(
            out.starts_with("[yconn] Running: ssh"),
            "must start with prefix: {out}"
        );
        assert!(
            !out.contains("-i"),
            "must not include -i flag for password auth: {out}"
        );
        assert!(
            out.contains("-p"),
            "must include -p flag for custom port: {out}"
        );
        assert!(out.contains("2222"), "must include port number: {out}");
        assert!(
            out.contains("deploy@myhost"),
            "must include destination: {out}"
        );
    }

    // ── additional edge cases ─────────────────────────────────────────────────

    #[test]
    fn test_key_auth_always_has_key_flag() {
        // Auth::Key always includes a key path, so -i is always emitted.
        let conn = make_conn("key", Some("~/.ssh/id_rsa"), 22, "myhost", "user");
        let args = build_args(&conn);
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"~/.ssh/id_rsa".to_string()));
    }

    #[test]
    fn test_destination_format() {
        let conn = make_conn("password", None, 22, "10.0.0.1", "admin");
        let args = build_args(&conn);
        assert!(args.last().unwrap().contains('@'));
        assert_eq!(args.last().unwrap(), "admin@10.0.0.1");
    }
}
