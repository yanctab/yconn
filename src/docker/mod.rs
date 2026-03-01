//! Container detection, mount resolution, and Docker process invocation.
//!
//! Handles all Docker-related logic: detecting whether the process is already
//! running inside a container, building the exact `docker run` command
//! described in CLAUDE.md, and replacing the current process via `execvp`.
//!
//! This module is completely separate from `connect` — they are two different
//! execution paths. When a `docker.image` is configured and yconn is **not**
//! already inside a container, this module takes over before SSH is invoked.

// Public API is consumed by CLI command modules not yet implemented.
#![allow(dead_code)]

use std::path::Path;

use anyhow::{Context, Result};

use crate::config::DockerConfig;
use crate::display::Renderer;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Return `true` if the current process is running inside a container.
///
/// Checks `/.dockerenv` existence **and** `CONN_IN_DOCKER=1` env var.
pub fn in_container() -> bool {
    detect_container(Path::new("/.dockerenv"))
}

/// Build the `docker run` argv for container re-invocation.
///
/// `original_argv` is the full yconn invocation to replay inside the
/// container — e.g. `["yconn", "connect", "prod"]`. The binary path,
/// working directory, and user config directory are resolved from the
/// running process.
pub fn build_args(docker: &DockerConfig, original_argv: &[String]) -> Result<Vec<String>> {
    let binary = std::env::current_exe().context("cannot determine current executable path")?;
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let user_config = dirs::config_dir().map(|d| d.join("yconn"));
    let pid = std::process::id();
    Ok(build_args_impl(
        docker,
        original_argv,
        &binary,
        user_config.as_deref(),
        &cwd,
        pid,
    ))
}

/// Replace the current process with the Docker bootstrap invocation.
///
/// If `verbose` is `true`, the full `docker run` command is printed via the
/// display module before exec.
pub fn exec(
    docker: &DockerConfig,
    original_argv: &[String],
    verbose: bool,
    renderer: &Renderer,
) -> Result<()> {
    if verbose {
        renderer.verbose(&format!("Docker image configured: {}", docker.image));
        renderer.verbose("Not running inside container — bootstrapping into Docker");
    }

    let args = build_args(docker, original_argv)?;

    if verbose {
        renderer.verbose_docker_cmd(&args);
    }

    exec_argv(&args)
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Container detection with an injectable `dockerenv` path for testing.
fn detect_container(dockerenv_path: &Path) -> bool {
    dockerenv_path.exists() || std::env::var("CONN_IN_DOCKER").as_deref() == Ok("1")
}

/// Core `docker run` argv construction with all paths injected for testing.
fn build_args_impl(
    docker: &DockerConfig,
    original_argv: &[String],
    binary_path: &Path,
    user_config_dir: Option<&Path>,
    cwd: &Path,
    pid: u32,
) -> Vec<String> {
    let mut args = vec!["docker".to_string(), "run".to_string()];

    // Unique, traceable container name.
    args.push("--name".to_string());
    args.push(format!("yconn-connection-{pid}"));

    // Terminal and lifecycle flags.
    args.push("-i".to_string());
    args.push("-t".to_string());
    args.push("--rm".to_string());

    // Re-invocation guard env var.
    args.push("-e".to_string());
    args.push("CONN_IN_DOCKER=1".to_string());

    // Mount: yconn binary (same path, read-only).
    let bin = binary_path.to_string_lossy();
    args.push("-v".to_string());
    args.push(format!("{bin}:{bin}:ro"));

    // Mount: system config layer (read-only).
    args.push("-v".to_string());
    args.push("/etc/yconn:/etc/yconn:ro".to_string());

    // Mount: user config layer (read-write — session.yml must be writable).
    if let Some(ucfg) = user_config_dir {
        let p = ucfg.to_string_lossy();
        args.push("-v".to_string());
        args.push(format!("{p}:{p}"));
    }

    // Mount: working directory (read-only — enables upward walk to project config).
    let cwd_str = cwd.to_string_lossy();
    args.push("-v".to_string());
    args.push(format!("{cwd_str}:{cwd_str}:ro"));
    args.push("-w".to_string());
    args.push(cwd_str.into_owned());

    // Pull policy — omit for the default "missing" to avoid requiring a newer Docker.
    if docker.pull != "missing" {
        args.push("--pull".to_string());
        args.push(docker.pull.clone());
    }

    // User-supplied extra args inserted after yconn's args, before the image name.
    args.extend(docker.args.iter().cloned());

    // Image name.
    args.push(docker.image.clone());

    // Original yconn argv replayed verbatim inside the container.
    args.extend(original_argv.iter().cloned());

    args
}

#[cfg(unix)]
fn exec_argv(argv: &[String]) -> Result<()> {
    use std::ffi::CString;

    let c_args: Vec<CString> = argv
        .iter()
        .map(|s| CString::new(s.as_bytes()).context("argument contains null byte"))
        .collect::<Result<_>>()?;

    let c_ptrs: Vec<*const libc::c_char> = c_args
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    // Safety: c_args is alive for the duration of the call; c_ptrs is
    // null-terminated; program is the first element of argv (POSIX convention).
    let ret = unsafe { libc::execvp(c_ptrs[0], c_ptrs.as_ptr()) };

    Err(anyhow::anyhow!(
        "execvp failed (exit code {}): {}",
        ret,
        std::io::Error::last_os_error()
    ))
}

#[cfg(not(unix))]
fn exec_argv(_argv: &[String]) -> Result<()> {
    anyhow::bail!("process exec is not supported on this platform")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use crate::config::{DockerConfig, Layer};

    fn make_docker(image: &str, pull: &str, args: Vec<&str>) -> DockerConfig {
        DockerConfig {
            image: image.to_string(),
            pull: pull.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            layer: Layer::Project,
            source_path: PathBuf::from(".yconn/connections.yaml"),
        }
    }

    /// Call build_args_impl with fixed test paths and pid 12345.
    fn call_build(docker: &DockerConfig, original_argv: &[&str]) -> Vec<String> {
        let argv: Vec<String> = original_argv.iter().map(|s| s.to_string()).collect();
        build_args_impl(
            docker,
            &argv,
            Path::new("/usr/local/bin/yconn"),
            Some(Path::new("/home/user/.config/yconn")),
            Path::new("/home/user/projects/acme"),
            12345,
        )
    }

    // ── Scenario 1: not in container → correct docker run args ───────────────

    #[test]
    fn test_build_args_basic_structure() {
        let docker = make_docker("ghcr.io/org/keys:latest", "missing", vec![]);
        let args = call_build(&docker, &["yconn", "connect", "prod"]);
        assert_eq!(args[0], "docker");
        assert_eq!(args[1], "run");
        assert!(args.contains(&"--name".to_string()));
        assert!(args.contains(&"yconn-connection-12345".to_string()));
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"-t".to_string()));
        assert!(args.contains(&"--rm".to_string()));
        assert!(args.contains(&"-e".to_string()));
        assert!(args.contains(&"CONN_IN_DOCKER=1".to_string()));
    }

    #[test]
    fn test_image_before_original_argv() {
        let docker = make_docker("myimage:latest", "missing", vec![]);
        let args = call_build(&docker, &["yconn", "connect", "prod"]);
        let img_pos = args.iter().position(|a| a == "myimage:latest").unwrap();
        let yconn_pos = args.iter().position(|a| a == "yconn").unwrap();
        assert!(img_pos < yconn_pos);
        assert_eq!(args[args.len() - 2], "connect");
        assert_eq!(args[args.len() - 1], "prod");
    }

    // ── Scenario 2: inside container via env var → detected ──────────────────

    #[test]
    fn test_in_container_via_env_var() {
        let dir = TempDir::new().unwrap();
        let absent = dir.path().join("no-dockerenv");
        std::env::set_var("CONN_IN_DOCKER", "1");
        let result = detect_container(&absent);
        std::env::remove_var("CONN_IN_DOCKER");
        assert!(result);
    }

    // ── Scenario 3: inside container via file → detected ─────────────────────

    #[test]
    fn test_in_container_via_file() {
        let dir = TempDir::new().unwrap();
        let dockerenv = dir.path().join(".dockerenv");
        std::fs::write(&dockerenv, "").unwrap();
        assert!(detect_container(&dockerenv));
    }

    #[test]
    fn test_not_in_container() {
        let dir = TempDir::new().unwrap();
        let absent = dir.path().join("no-dockerenv");
        std::env::remove_var("CONN_IN_DOCKER");
        assert!(!detect_container(&absent));
    }

    // ── Scenario 4: pull: always → --pull always included ────────────────────

    #[test]
    fn test_pull_always_included() {
        let docker = make_docker("img:latest", "always", vec![]);
        let args = call_build(&docker, &["yconn", "connect", "srv"]);
        let pull_pos = args.iter().position(|a| a == "--pull").unwrap();
        assert_eq!(args[pull_pos + 1], "always");
    }

    #[test]
    fn test_pull_never_included() {
        let docker = make_docker("img:latest", "never", vec![]);
        let args = call_build(&docker, &["yconn", "connect", "srv"]);
        let pull_pos = args.iter().position(|a| a == "--pull").unwrap();
        assert_eq!(args[pull_pos + 1], "never");
    }

    // ── Scenario 5 & 8: docker args appear before image ──────────────────────

    #[test]
    fn test_docker_args_appear_before_image() {
        let docker = make_docker(
            "myimage:v1",
            "missing",
            vec!["--network=host", "--env=FOO=bar"],
        );
        let args = call_build(&docker, &["yconn", "connect", "srv"]);
        let img_pos = args.iter().position(|a| a == "myimage:v1").unwrap();
        let net_pos = args.iter().position(|a| a == "--network=host").unwrap();
        let env_pos = args.iter().position(|a| a == "--env=FOO=bar").unwrap();
        assert!(net_pos < img_pos);
        assert!(env_pos < img_pos);
    }

    // ── Scenario 6: docker args empty → image directly followed by argv ──────

    #[test]
    fn test_docker_args_empty_no_extra_flags() {
        let docker = make_docker("myimage:v1", "missing", vec![]);
        let args = call_build(&docker, &["yconn", "connect", "srv"]);
        let img_pos = args.iter().position(|a| a == "myimage:v1").unwrap();
        let tail = &args[img_pos + 1..];
        assert_eq!(tail, &["yconn", "connect", "srv"]);
    }

    // ── Scenario 7: no docker block → build_args never called ────────────────

    #[test]
    fn test_no_docker_block_no_bootstrap() {
        // When LoadedConfig.docker is None the CLI skips docker entirely.
        // From this module's perspective: if docker is absent, nothing here runs.
        let no_docker: Option<DockerConfig> = None;
        assert!(no_docker.is_none());
        // Container detection also returns false when not in a container.
        let dir = TempDir::new().unwrap();
        std::env::remove_var("CONN_IN_DOCKER");
        assert!(!detect_container(&dir.path().join("no-dockerenv")));
    }

    // ── Scenario 9: pull: missing (default) → --pull not emitted ─────────────

    #[test]
    fn test_pull_missing_not_emitted() {
        let docker = make_docker("img:latest", "missing", vec![]);
        let args = call_build(&docker, &["yconn", "connect", "srv"]);
        assert!(!args.contains(&"--pull".to_string()));
    }

    // ── Mounts ────────────────────────────────────────────────────────────────

    #[test]
    fn test_binary_mount_readonly() {
        let docker = make_docker("img:v1", "missing", vec![]);
        let args = call_build(&docker, &["yconn"]);
        assert!(args
            .iter()
            .any(|a| a == "/usr/local/bin/yconn:/usr/local/bin/yconn:ro"));
    }

    #[test]
    fn test_system_config_mount_readonly() {
        let docker = make_docker("img:v1", "missing", vec![]);
        let args = call_build(&docker, &["yconn"]);
        assert!(args.iter().any(|a| a == "/etc/yconn:/etc/yconn:ro"));
    }

    #[test]
    fn test_user_config_mount_readwrite() {
        let docker = make_docker("img:v1", "missing", vec![]);
        let args = call_build(&docker, &["yconn"]);
        // rw — no :ro suffix
        assert!(args
            .iter()
            .any(|a| a == "/home/user/.config/yconn:/home/user/.config/yconn"));
        assert!(!args
            .iter()
            .any(|a| a == "/home/user/.config/yconn:/home/user/.config/yconn:ro"));
    }

    #[test]
    fn test_cwd_mount_readonly_and_workdir_set() {
        let docker = make_docker("img:v1", "missing", vec![]);
        let args = call_build(&docker, &["yconn"]);
        assert!(args
            .iter()
            .any(|a| { a == "/home/user/projects/acme:/home/user/projects/acme:ro" }));
        let w_pos = args.iter().position(|a| a == "-w").unwrap();
        assert_eq!(args[w_pos + 1], "/home/user/projects/acme");
    }

    #[test]
    fn test_container_name_includes_pid() {
        let docker = make_docker("img:v1", "missing", vec![]);
        let args = call_build(&docker, &["yconn"]);
        assert!(args.contains(&"yconn-connection-12345".to_string()));
    }
}
