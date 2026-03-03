//! Functional tests for the compiled `yconn` binary.
//!
//! These tests run the real `yconn` binary in a controlled environment, using
//! mock `ssh` and `docker` scripts in a prepended PATH directory to intercept
//! exec calls without making any real SSH connections or Docker invocations.
//!
//! The mock scripts print their invocation to stdout and exit 0. Because
//! `execvp` replaces the yconn process, the mock script's stdout becomes
//! the subprocess's captured output.
//!
//! Environment control (set per subprocess — no race conditions between tests):
//!   PATH           — mock_bin prepended so mock ssh/docker shadow the real ones
//!   XDG_CONFIG_HOME — redirects dirs::config_dir() to our temp user-config dir
//!   HOME           — bounds the project-config upward walk
//!   CONN_IN_DOCKER — simulates being inside a container

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Output;
use tempfile::TempDir;

// ─── TestEnv ─────────────────────────────────────────────────────────────────

struct TestEnv {
    /// CWD for yconn; `.yconn/` is placed here for project config.
    cwd: TempDir,
    /// XDG_CONFIG_HOME; user config lives at `xdg_config/yconn/`.
    xdg_config: TempDir,
    /// Contains mock `ssh` and `docker` scripts.
    mock_bin: TempDir,
    /// HOME; prevents the upward walk from escaping the temp tree.
    home: TempDir,
}

impl TestEnv {
    fn new() -> Self {
        let mock_bin = TempDir::new().unwrap();
        write_mock_script(&mock_bin.path().join("ssh"), "ssh");
        write_mock_script(&mock_bin.path().join("docker"), "docker");
        Self {
            cwd: TempDir::new().unwrap(),
            xdg_config: TempDir::new().unwrap(),
            mock_bin,
            home: TempDir::new().unwrap(),
        }
    }

    /// Write `yaml` to `<cwd>/.yconn/<group>.yaml`.
    fn write_project_config(&self, group: &str, yaml: &str) {
        let dir = self.cwd.path().join(".yconn");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{group}.yaml")), yaml).unwrap();
    }

    /// Write `yaml` to `<xdg_config>/yconn/<group>.yaml`.
    fn write_user_config(&self, group: &str, yaml: &str) {
        let dir = self.xdg_config.path().join("yconn");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{group}.yaml")), yaml).unwrap();
    }

    /// Write a fake SSH key with 600 permissions into cwd; return its absolute path.
    fn write_key(&self, filename: &str) -> String {
        let path = self.cwd.path().join(filename);
        fs::write(&path, "fake key content").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Run yconn with a controlled environment. `--no-color` is always prepended.
    fn run(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.mock_bin.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        std::process::Command::new(env!("CARGO_BIN_EXE_yconn"))
            .arg("--no-color")
            .args(args)
            .env("PATH", path)
            .env("XDG_CONFIG_HOME", self.xdg_config.path())
            .env("HOME", self.home.path())
            .env_remove("CONN_IN_DOCKER")
            .current_dir(self.cwd.path())
            .output()
            .unwrap()
    }

    /// Same as `run` but with `CONN_IN_DOCKER=1` to simulate being inside a container.
    fn run_in_container(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.mock_bin.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        std::process::Command::new(env!("CARGO_BIN_EXE_yconn"))
            .arg("--no-color")
            .args(args)
            .env("PATH", path)
            .env("XDG_CONFIG_HOME", self.xdg_config.path())
            .env("HOME", self.home.path())
            .env("CONN_IN_DOCKER", "1")
            .current_dir(self.cwd.path())
            .output()
            .unwrap()
    }

    /// Panic with a diagnostic if the binary exited non-zero.
    fn assert_ok(output: &Output) {
        if !output.status.success() {
            panic!(
                "yconn exited with {:?}\nstdout: {}\nstderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Write a mock script that prints `<name> <args...>` on one line and exits 0.
///
/// Uses `echo {name} "$@"` so each positional parameter is a separate word
/// passed to echo, which joins them with spaces — giving one line of output.
fn write_mock_script(path: &std::path::Path, name: &str) {
    let content = format!("#!/bin/sh\necho {name} \"$@\"\n");
    fs::write(path, &content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

// ─── Config fixture helpers ───────────────────────────────────────────────────

fn conn_key(name: &str, host: &str, user: &str, port: Option<u16>, key: &str) -> String {
    let port_line = match port {
        Some(p) => format!("\n    port: {p}"),
        None => String::new(),
    };
    format!(
        "connections:\n  {name}:\n    host: {host}\n    user: {user}{port_line}\n    auth: key\n    key: {key}\n    description: test connection\n"
    )
}

fn conn_password(name: &str, host: &str, user: &str, port: Option<u16>) -> String {
    let port_line = match port {
        Some(p) => format!("\n    port: {p}"),
        None => String::new(),
    };
    format!(
        "connections:\n  {name}:\n    host: {host}\n    user: {user}{port_line}\n    auth: password\n    description: test connection\n"
    )
}

fn conn_key_with_link(name: &str, host: &str, user: &str, key: &str, link: &str) -> String {
    format!(
        "connections:\n  {name}:\n    host: {host}\n    user: {user}\n    auth: key\n    key: {key}\n    description: test connection\n    link: {link}\n"
    )
}

/// Wrap a connections YAML block in a docker section.
///
/// `connections_yaml` must start with `connections:\n`.
fn with_docker(
    image: &str,
    pull: Option<&str>,
    extra_args: &[&str],
    connections_yaml: &str,
) -> String {
    let pull_line = match pull {
        Some(p) => format!("\n  pull: {p}"),
        None => String::new(),
    };
    let args_section = if extra_args.is_empty() {
        String::new()
    } else {
        let items: String = extra_args
            .iter()
            .map(|a| format!("    - \"{a}\"\n"))
            .collect();
        format!("  args:\n{items}")
    };
    format!("docker:\n  image: {image}{pull_line}\n{args_section}{connections_yaml}")
}

// ─── SSH scenarios ────────────────────────────────────────────────────────────

#[test]
fn ssh_key_auth_default_port() {
    let env = TestEnv::new();
    let key = env.write_key("id_rsa");
    env.write_user_config(
        "connections",
        &conn_key("myconn", "myhost", "deploy", None, &key),
    );

    // CONN_IN_DOCKER=1 skips docker; no docker block → goes straight to ssh.
    let out = env.run_in_container(&["connect", "myconn"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("ssh -i {key} deploy@myhost")),
        "expected 'ssh -i {key} deploy@myhost' in stdout, got: {stdout}"
    );
    assert!(
        !stdout.contains("-p "),
        "expected no -p flag for default port, got: {stdout}"
    );
}

#[test]
fn ssh_key_auth_custom_port() {
    let env = TestEnv::new();
    let key = env.write_key("id_ed25519");
    env.write_user_config(
        "connections",
        &conn_key("myconn", "myhost", "admin", Some(2222), &key),
    );

    let out = env.run_in_container(&["connect", "myconn"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("ssh -i {key} -p 2222 admin@myhost")),
        "expected 'ssh -i {key} -p 2222 admin@myhost' in stdout, got: {stdout}"
    );
}

#[test]
fn ssh_password_auth_default_port() {
    let env = TestEnv::new();
    env.write_user_config(
        "connections",
        &conn_password("myconn", "db.internal", "dbadmin", None),
    );

    let out = env.run_in_container(&["connect", "myconn"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ssh dbadmin@db.internal"),
        "expected 'ssh dbadmin@db.internal' in stdout, got: {stdout}"
    );
    assert!(
        !stdout.contains("-i "),
        "expected no -i flag for password auth, got: {stdout}"
    );
    assert!(
        !stdout.contains("-p "),
        "expected no -p flag for default port, got: {stdout}"
    );
}

#[test]
fn ssh_password_auth_custom_port() {
    let env = TestEnv::new();
    env.write_user_config(
        "connections",
        &conn_password("myconn", "bastion.example.com", "ec2-user", Some(2222)),
    );

    let out = env.run_in_container(&["connect", "myconn"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ssh -p 2222 ec2-user@bastion.example.com"),
        "expected 'ssh -p 2222 ec2-user@bastion.example.com' in stdout, got: {stdout}"
    );
    assert!(
        !stdout.contains("-i "),
        "expected no -i flag for password auth, got: {stdout}"
    );
}

// ─── Docker scenarios ─────────────────────────────────────────────────────────

#[test]
fn docker_bootstrap_not_in_container() {
    let env = TestEnv::new();
    let yaml = with_docker(
        "ghcr.io/org/keys:latest",
        None,
        &[],
        &conn_password("prod", "10.0.0.1", "deploy", None),
    );
    env.write_project_config("connections", &yaml);

    // Run WITHOUT CONN_IN_DOCKER → yconn bootstraps into Docker.
    let out = env.run(&["connect", "prod"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("docker run"),
        "expected output to start with 'docker run', got: {stdout}"
    );
    assert!(
        stdout.contains("yconn-connection-"),
        "expected 'yconn-connection-' container name prefix in stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("-i") && stdout.contains("-t") && stdout.contains("--rm"),
        "expected -i -t --rm flags in docker run args, got: {stdout}"
    );
    assert!(
        stdout.contains("CONN_IN_DOCKER=1"),
        "expected CONN_IN_DOCKER=1 in docker run env args, got: {stdout}"
    );
    assert!(
        stdout.contains("ghcr.io/org/keys:latest"),
        "expected image name in stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("connect prod"),
        "expected original subcommand replayed after image in stdout, got: {stdout}"
    );
}

#[test]
fn docker_skipped_when_conn_in_docker() {
    let env = TestEnv::new();
    let yaml = with_docker(
        "ghcr.io/org/keys:latest",
        None,
        &[],
        &conn_password("prod", "10.0.0.1", "deploy", None),
    );
    env.write_project_config("connections", &yaml);

    // CONN_IN_DOCKER=1 → docker bootstrap skipped, SSH invoked directly.
    let out = env.run_in_container(&["connect", "prod"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("ssh "),
        "expected output to start with 'ssh ' when already in container, got: {stdout}"
    );
    assert!(
        !stdout.contains("docker"),
        "expected no 'docker' in stdout when already in container, got: {stdout}"
    );
}

#[test]
fn docker_pull_always() {
    let env = TestEnv::new();
    let yaml = with_docker(
        "ghcr.io/org/keys:latest",
        Some("always"),
        &[],
        &conn_password("prod", "10.0.0.1", "deploy", None),
    );
    env.write_project_config("connections", &yaml);

    let out = env.run(&["connect", "prod"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--pull always"),
        "expected '--pull always' in docker run args, got: {stdout}"
    );
}

#[test]
fn docker_extra_args_before_image() {
    let env = TestEnv::new();
    let yaml = with_docker(
        "ghcr.io/org/keys:latest",
        None,
        &["--network=host"],
        &conn_password("prod", "10.0.0.1", "deploy", None),
    );
    env.write_project_config("connections", &yaml);

    let out = env.run(&["connect", "prod"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let img_pos = stdout
        .find("ghcr.io/org/keys:latest")
        .expect("image name not found in stdout");
    let net_pos = stdout
        .find("--network=host")
        .expect("--network=host not found in stdout");
    assert!(
        net_pos < img_pos,
        "--network=host should appear before image name in docker run args, got: {stdout}"
    );
}

#[test]
fn no_docker_block_uses_ssh() {
    let env = TestEnv::new();
    env.write_user_config(
        "connections",
        &conn_password("prod", "10.0.0.1", "deploy", None),
    );

    // No CONN_IN_DOCKER set, but no docker block → SSH used directly.
    let out = env.run(&["connect", "prod"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("ssh "),
        "expected output to start with 'ssh ' when no docker block, got: {stdout}"
    );
    assert!(
        !stdout.contains("docker"),
        "expected no 'docker' in stdout when no docker block configured, got: {stdout}"
    );
}

// ─── List link column ─────────────────────────────────────────────────────────

#[test]
fn list_shows_link_column_when_connection_has_link() {
    let env = TestEnv::new();
    let key = env.write_key("id_rsa");
    let link = "https://wiki.internal/servers/prod-web";
    env.write_user_config(
        "connections",
        &conn_key_with_link("prod-web", "10.0.1.50", "deploy", &key, link),
    );

    let out = env.run(&["list"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("LINK"),
        "expected LINK column header in list output, got: {stdout}"
    );
    assert!(
        stdout.contains(link),
        "expected link URL '{link}' in list output, got: {stdout}"
    );
}

#[test]
fn list_omits_link_column_when_no_connection_has_link() {
    let env = TestEnv::new();
    env.write_user_config(
        "connections",
        &conn_password("prod-web", "10.0.1.50", "deploy", None),
    );

    let out = env.run(&["list"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("LINK"),
        "expected no LINK column header when no connection has a link, got: {stdout}"
    );
}

// ─── Config priority scenario ─────────────────────────────────────────────────

#[test]
fn project_layer_wins_over_user() {
    let env = TestEnv::new();

    // Project config has "srv" → project-host.
    env.write_project_config(
        "connections",
        &conn_password("srv", "project-host.internal", "deploy", None),
    );
    // User config has the same "srv" → user-host (should be shadowed).
    env.write_user_config(
        "connections",
        &conn_password("srv", "user-host.internal", "admin", None),
    );

    // Run in container so we go straight to SSH.
    let out = env.run_in_container(&["connect", "srv"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("deploy@project-host.internal"),
        "expected project layer host/user to win, got: {stdout}"
    );
    assert!(
        !stdout.contains("user-host.internal"),
        "expected user-host to be shadowed by project layer, got: {stdout}"
    );
}
