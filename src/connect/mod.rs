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

use crate::config::Connection;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Build the SSH argument list for `conn`.
///
/// Returns a `Vec<String>` where the first element is `"ssh"` and the
/// remaining elements are the flags and destination, ready to be passed
/// directly to `execvp`.
///
/// Rules:
/// - `auth: key` → `-i <key>` inserted before destination; port flag added
///   when port ≠ 22.
/// - `auth: password` (or any other value) → no `-i` flag; port flag added
///   when port ≠ 22.
/// - Destination is always `user@host`.
pub fn build_args(conn: &Connection) -> Vec<String> {
    let mut args = vec!["ssh".to_string()];

    if conn.auth == "key" {
        if let Some(ref key) = conn.key {
            args.push("-i".to_string());
            args.push(key.clone());
        }
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

    use crate::config::{Connection, Layer};

    fn make_conn(auth: &str, key: Option<&str>, port: u16, host: &str, user: &str) -> Connection {
        Connection {
            name: "test".to_string(),
            host: host.to_string(),
            user: user.to_string(),
            port,
            auth: auth.to_string(),
            key: key.map(str::to_string),
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
        assert_eq!(args, vec!["ssh", "-i", "~/.ssh/id_rsa", "deploy@myhost"]);
    }

    #[test]
    fn test_key_auth_custom_port() {
        let conn = make_conn("key", Some("~/.ssh/id_rsa"), 2222, "myhost", "deploy");
        let args = build_args(&conn);
        assert_eq!(
            args,
            vec!["ssh", "-i", "~/.ssh/id_rsa", "-p", "2222", "deploy@myhost"]
        );
    }

    #[test]
    fn test_password_auth_default_port() {
        let conn = make_conn("password", None, 22, "myhost", "deploy");
        let args = build_args(&conn);
        assert_eq!(args, vec!["ssh", "deploy@myhost"]);
    }

    #[test]
    fn test_password_auth_custom_port() {
        let conn = make_conn("password", None, 2222, "myhost", "deploy");
        let args = build_args(&conn);
        assert_eq!(args, vec!["ssh", "-p", "2222", "deploy@myhost"]);
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
    fn test_key_auth_without_key_field() {
        // auth=key but no key path — no -i flag emitted
        let conn = make_conn("key", None, 22, "myhost", "user");
        let args = build_args(&conn);
        assert_eq!(args, vec!["ssh", "user@myhost"]);
    }

    #[test]
    fn test_destination_format() {
        let conn = make_conn("password", None, 22, "10.0.0.1", "admin");
        let args = build_args(&conn);
        assert!(args.last().unwrap().contains('@'));
        assert_eq!(args.last().unwrap(), "admin@10.0.0.1");
    }
}
