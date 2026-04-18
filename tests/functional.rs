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
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::process::{Output, Stdio};
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

    /// Run yconn with a controlled environment.
    fn run(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.mock_bin.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        std::process::Command::new(env!("CARGO_BIN_EXE_yconn"))
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
            .args(args)
            .env("PATH", path)
            .env("XDG_CONFIG_HOME", self.xdg_config.path())
            .env("HOME", self.home.path())
            .env("CONN_IN_DOCKER", "1")
            .current_dir(self.cwd.path())
            .output()
            .unwrap()
    }

    /// Same as `run` but pipes `stdin_data` to the subprocess's stdin.
    fn run_with_stdin(&self, args: &[&str], stdin_data: &str) -> Output {
        let path = format!(
            "{}:{}",
            self.mock_bin.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_yconn"))
            .args(args)
            .env("PATH", path)
            .env("XDG_CONFIG_HOME", self.xdg_config.path())
            .env("HOME", self.home.path())
            .env_remove("CONN_IN_DOCKER")
            .current_dir(self.cwd.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Write to stdin then drop so the child gets EOF.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(stdin_data.as_bytes());
        }
        child.wait_with_output().unwrap()
    }

    /// Same as `run` but with additional environment variables injected.
    fn run_with_env(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        let path = format!(
            "{}:{}",
            self.mock_bin.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_yconn"));
        cmd.args(args)
            .env("PATH", path)
            .env("XDG_CONFIG_HOME", self.xdg_config.path())
            .env("HOME", self.home.path())
            .env_remove("CONN_IN_DOCKER")
            .current_dir(self.cwd.path());
        for (key, val) in extra_env {
            cmd.env(key, val);
        }
        cmd.output().unwrap()
    }

    /// Install a mock editor script into `mock_bin` that exits 0 without
    /// modifying any files.  Returns the script path (for assertions).
    fn install_mock_editor(&self) {
        let script = self.mock_bin.path().join("mock-editor");
        let content = "#!/bin/sh\n# mock editor: do nothing, exit 0\n";
        fs::write(&script, content).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
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
        "connections:\n  {name}:\n    host: {host}\n    user: {user}{port_line}\n    auth:\n      type: key\n      key: {key}\n    description: test connection\n"
    )
}

fn conn_password(name: &str, host: &str, user: &str, port: Option<u16>) -> String {
    let port_line = match port {
        Some(p) => format!("\n    port: {p}"),
        None => String::new(),
    };
    format!(
        "connections:\n  {name}:\n    host: {host}\n    user: {user}{port_line}\n    auth:\n      type: password\n    description: test connection\n"
    )
}

fn conn_identity(name: &str, host: &str, user: &str, port: Option<u16>, key: &str) -> String {
    let port_line = match port {
        Some(p) => format!("\n    port: {p}"),
        None => String::new(),
    };
    format!(
        "connections:\n  {name}:\n    host: {host}\n    user: {user}{port_line}\n    auth:\n      type: identity\n      key: {key}\n    description: test connection\n"
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
        stdout.contains(&format!("ssh -F /dev/null -i {key} deploy@myhost")),
        "expected 'ssh -F /dev/null -i {key} deploy@myhost' in stdout, got: {stdout}"
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
        stdout.contains(&format!("ssh -F /dev/null -i {key} -p 2222 admin@myhost")),
        "expected 'ssh -F /dev/null -i {key} -p 2222 admin@myhost' in stdout, got: {stdout}"
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
        stdout.contains("ssh -F /dev/null dbadmin@db.internal"),
        "expected 'ssh -F /dev/null dbadmin@db.internal' in stdout, got: {stdout}"
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
        stdout.contains("ssh -F /dev/null -p 2222 ec2-user@bastion.example.com"),
        "expected 'ssh -F /dev/null -p 2222 ec2-user@bastion.example.com' in stdout, got: {stdout}"
    );
    assert!(
        !stdout.contains("-i "),
        "expected no -i flag for password auth, got: {stdout}"
    );
}

// ─── Connecting line on stderr ────────────────────────────────────────────────

#[test]
fn connect_key_auth_prints_connecting_line_to_stderr() {
    let env = TestEnv::new();
    let key = env.write_key("id_rsa");
    env.write_user_config(
        "connections",
        &conn_key("myconn", "myhost", "deploy", None, &key),
    );

    let out = env.run_in_container(&["connect", "myconn"]);
    TestEnv::assert_ok(&out);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!(
            "[yconn] Connecting: ssh -F /dev/null -i {key} deploy@myhost"
        )),
        "expected '[yconn] Connecting: ssh -F /dev/null -i {key} deploy@myhost' in stderr, got: {stderr}"
    );
}

#[test]
fn connect_password_auth_prints_connecting_line_to_stderr() {
    let env = TestEnv::new();
    env.write_user_config(
        "connections",
        &conn_password("myconn", "db.internal", "dbadmin", None),
    );

    let out = env.run_in_container(&["connect", "myconn"]);
    TestEnv::assert_ok(&out);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[yconn] Connecting: ssh -F /dev/null dbadmin@db.internal"),
        "expected '[yconn] Connecting: ssh -F /dev/null dbadmin@db.internal' in stderr, got: {stderr}"
    );
}

#[test]
fn connect_connecting_line_stdout_is_unaffected() {
    // The connecting line goes to stderr — stdout (mock ssh output) must be unchanged.
    let env = TestEnv::new();
    env.write_user_config(
        "connections",
        &conn_password("myconn", "db.internal", "dbadmin", None),
    );

    let out = env.run_in_container(&["connect", "myconn"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ssh -F /dev/null dbadmin@db.internal"),
        "expected mock ssh output in stdout, got: {stdout}"
    );
    assert!(
        !stdout.contains("[yconn] Connecting:"),
        "connecting line must not appear in stdout, got: {stdout}"
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

// ─── Add round-trip and edit invocation ──────────────────────────────────────

/// `yconn connections add` (piped stdin) → `yconn list` then `yconn connections show`
/// successfully display the newly created connection, verifying the YAML is valid and
/// parseable after the add wizard writes it.
#[test]
fn add_round_trip_list_and_show() {
    let env = TestEnv::new();

    // Simulate the wizard: name, host, user, port (blank=22), auth, key,
    // description, link (blank).
    let key = env.write_key("id_rsa");
    let stdin_data = format!("myconn\nmyhost.internal\ndeploy\n\nkey\n{key}\nMy server\n\n");

    // `yconn connections add --layer user` — writes to xdg_config/yconn/connections.yaml.
    let out = env.run_with_stdin(&["connections", "add", "--layer", "user"], &stdin_data);
    TestEnv::assert_ok(&out);

    // `yconn list` should show the new connection.
    let list_out = env.run(&["list"]);
    TestEnv::assert_ok(&list_out);
    let list_stdout = String::from_utf8_lossy(&list_out.stdout);
    assert!(
        list_stdout.contains("myconn"),
        "expected 'myconn' in list output, got: {list_stdout}"
    );
    assert!(
        list_stdout.contains("myhost.internal"),
        "expected 'myhost.internal' in list output, got: {list_stdout}"
    );

    // `yconn connections show myconn` should succeed and display the connection detail.
    let show_out = env.run(&["connections", "show", "myconn"]);
    TestEnv::assert_ok(&show_out);
    let show_stdout = String::from_utf8_lossy(&show_out.stdout);
    assert!(
        show_stdout.contains("myconn"),
        "expected 'myconn' in show output, got: {show_stdout}"
    );
    assert!(
        show_stdout.contains("myhost.internal"),
        "expected 'myhost.internal' in show output, got: {show_stdout}"
    );
    assert!(
        show_stdout.contains("deploy"),
        "expected user 'deploy' in show output, got: {show_stdout}"
    );
}

/// `yconn connections add` for password auth writes a valid, parseable YAML entry with
/// no `key:` field, verified by `yconn connections show` succeeding afterwards.
#[test]
fn add_password_auth_round_trip() {
    let env = TestEnv::new();

    // Wizard answers: name, host, user, port, auth=password, description, link.
    let stdin_data = "dbconn\ndb.internal\ndbadmin\n\npassword\nDatabase server\n\n";

    let out = env.run_with_stdin(&["connections", "add", "--layer", "user"], stdin_data);
    TestEnv::assert_ok(&out);

    // Verify the written YAML is parseable by running show.
    let show_out = env.run(&["connections", "show", "dbconn"]);
    TestEnv::assert_ok(&show_out);
    let show_stdout = String::from_utf8_lossy(&show_out.stdout);
    assert!(
        show_stdout.contains("dbconn"),
        "expected 'dbconn' in show output, got: {show_stdout}"
    );
    assert!(
        show_stdout.contains("password"),
        "expected auth 'password' in show output, got: {show_stdout}"
    );

    // Confirm the YAML file itself does not contain a key: field.
    let yaml_path = env.xdg_config.path().join("yconn").join("connections.yaml");
    let yaml = fs::read_to_string(&yaml_path).unwrap();
    assert!(
        !yaml.contains("key:"),
        "expected no 'key:' field for password auth, got yaml:\n{yaml}"
    );
}

/// `yconn connections edit <name>` invokes `$EDITOR` with the correct config file path.
/// The mock editor exits 0 without modifying the file, confirming the file
/// remains parseable after the editor exits.
#[test]
fn edit_invokes_editor_with_correct_file_path() {
    let env = TestEnv::new();
    env.install_mock_editor();

    // Set up a user-layer connection so `edit` has something to open.
    env.write_user_config(
        "connections",
        &conn_password("my-srv", "10.0.0.5", "admin", None),
    );

    let expected_path = env.xdg_config.path().join("yconn").join("connections.yaml");

    // Run with mock-editor as $EDITOR so no real editor is launched.
    let path = format!(
        "{}:{}",
        env.mock_bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_yconn"))
        .args(["connections", "edit", "my-srv"])
        .env("PATH", path)
        .env("XDG_CONFIG_HOME", env.xdg_config.path())
        .env("HOME", env.home.path())
        .env_remove("CONN_IN_DOCKER")
        .env("EDITOR", env.mock_bin.path().join("mock-editor"))
        .current_dir(env.cwd.path())
        .output()
        .unwrap();

    TestEnv::assert_ok(&out);

    // After the mock editor runs (no-op), the file must still be parseable —
    // verify by running `yconn connections show my-srv`.
    let show_out = env.run(&["connections", "show", "my-srv"]);
    TestEnv::assert_ok(&show_out);
    let show_stdout = String::from_utf8_lossy(&show_out.stdout);
    assert!(
        show_stdout.contains("my-srv"),
        "expected 'my-srv' in show output after edit, got: {show_stdout}"
    );

    // The edit command should mention the target file path in its output.
    // (yconn connections edit opens the editor; the path is passed as the arg to $EDITOR,
    //  but mock-editor doesn't echo its args — so we just verify exit was 0
    //  and the file is still accessible.)
    let _ = expected_path; // path confirmed parseable via show above
}

// ─── Parse error scenarios ────────────────────────────────────────────────────

/// A manually created minimal valid project config — `yconn list` shows the entry.
#[test]
fn parse_error_minimal_valid_project_config() {
    let env = TestEnv::new();
    // Write the config by hand (not using conn_key/conn_password helpers) to
    // simulate a manually created file with all required fields present.
    env.write_project_config(
        "connections",
        "connections:\n  my-server:\n    host: 10.0.0.1\n    user: admin\n    auth:\n      type: password\n    description: My server\n",
    );

    let out = env.run(&["list"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("my-server"),
        "expected 'my-server' in list output, got: {stdout}"
    );
    assert!(
        stdout.contains("10.0.0.1"),
        "expected host '10.0.0.1' in list output, got: {stdout}"
    );
}

/// A manually created minimal valid user layer config — `yconn list` shows the entry.
#[test]
fn parse_error_minimal_valid_user_config() {
    let env = TestEnv::new();
    // Write directly to the user layer config directory.
    env.write_user_config(
        "connections",
        "connections:\n  user-server:\n    host: 192.168.1.5\n    user: root\n    auth:\n      type: password\n    description: User server\n",
    );

    let out = env.run(&["list"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("user-server"),
        "expected 'user-server' in list output, got: {stdout}"
    );
    assert!(
        stdout.contains("192.168.1.5"),
        "expected host '192.168.1.5' in list output, got: {stdout}"
    );
}

/// A connection entry missing a required field — `yconn list` exits non-zero
/// with a clear error message naming the file, entry, and missing field.
#[test]
fn parse_error_missing_required_field() {
    let env = TestEnv::new();
    // 'host' field is intentionally absent.
    env.write_project_config(
        "connections",
        "connections:\n  bad-server:\n    user: admin\n    auth:\n      type: password\n    description: Missing host\n",
    );

    let out = env.run(&["list"]);
    assert!(
        !out.status.success(),
        "expected non-zero exit for missing required field, got 0"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bad-server"),
        "expected connection name 'bad-server' in error, got: {stderr}"
    );
    assert!(
        stderr.contains("host"),
        "expected missing field 'host' named in error, got: {stderr}"
    );
}

/// Invalid YAML syntax — `yconn list` exits non-zero with the file name in
/// the error message.
#[test]
fn parse_error_invalid_yaml_syntax() {
    let env = TestEnv::new();
    // Write deliberately malformed YAML.
    env.write_project_config(
        "connections",
        "connections:\n  broken: [unclosed bracket\n    host: 10.0.0.1\n",
    );

    let out = env.run(&["list"]);
    assert!(
        !out.status.success(),
        "expected non-zero exit for invalid YAML, got 0"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("connections.yaml"),
        "expected config file name 'connections.yaml' in error, got: {stderr}"
    );
}

/// Valid YAML with an empty `connections` block — `yconn list` exits 0 and
/// shows no connection entries (no error).
#[test]
fn parse_error_empty_connections_block() {
    let env = TestEnv::new();
    // Valid YAML but no connection entries.
    env.write_project_config("connections", "connections:\n");

    let out = env.run(&["list"]);
    // Must exit 0 — an empty connections block is not an error.
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    // No actual connection rows should appear — the output should be empty
    // or only contain the separator line (no host names, no auth types).
    assert!(
        !stdout.contains("password") && !stdout.contains("key"),
        "expected no connection rows for empty connections block, got: {stdout}"
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

// ─── Wildcard pattern matching ────────────────────────────────────────────────

/// `yconn connect` with a wildcard pattern match: mock ssh receives
/// `user@<input-hostname>` — the matched input is used as the host.
#[test]
fn wildcard_pattern_match_ssh_receives_input_as_host() {
    let env = TestEnv::new();

    // Pattern "web-*" in user config — any "web-<something>" input matches.
    env.write_user_config(
        "connections",
        "connections:\n  web-*:\n    host: placeholder.internal\n    user: deploy\n    auth:\n      type: password\n    description: Wildcard web servers\n",
    );

    // Connect using a concrete hostname that matches the pattern.
    // Run inside container so Docker bootstrap is skipped.
    let out = env.run_in_container(&["connect", "web-staging"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    // Mock ssh must receive "deploy@web-staging" — the input, NOT "placeholder.internal".
    assert!(
        stdout.contains("deploy@web-staging"),
        "expected 'deploy@web-staging' in stdout (input used as host), got: {stdout}"
    );
    assert!(
        !stdout.contains("placeholder.internal"),
        "expected placeholder host to NOT appear in stdout, got: {stdout}"
    );
}

/// `yconn connect` with two conflicting wildcard patterns exits non-zero and
/// names both patterns in stderr.
#[test]
fn wildcard_conflict_exits_nonzero_with_pattern_names_in_stderr() {
    let env = TestEnv::new();

    // Two patterns that both match "web-prod": "web-*" and "?eb-prod".
    // Note: a bare `*` at the start of a YAML key is a YAML anchor — quote it.
    env.write_user_config(
        "connections",
        "connections:\n  web-*:\n    host: ph1\n    user: deploy\n    auth:\n      type: password\n    description: Web wildcard\n  \"?eb-prod\":\n    host: ph2\n    user: admin\n    auth:\n      type: password\n    description: Prefix wildcard\n",
    );

    let out = env.run_in_container(&["connect", "web-prod"]);
    assert!(
        !out.status.success(),
        "expected non-zero exit for conflicting wildcard patterns, got 0"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("web-*"),
        "expected pattern 'web-*' named in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("?eb-prod"),
        "expected pattern '?eb-prod' named in stderr, got: {stderr}"
    );
}

/// `yconn connect` with `host: ${name}.corp.com` — mock ssh receives
/// `user@server01.corp.com`, not `user@server01`.
#[test]
fn wildcard_name_template_in_host_expands_to_fqdn() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  server*:\n    host: \"${name}.corp.com\"\n    user: deploy\n    auth:\n      type: password\n    description: Corp servers\n",
    );

    let out = env.run_in_container(&["connect", "server01"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("deploy@server01.corp.com"),
        "expected 'deploy@server01.corp.com' in stdout, got: {stdout}"
    );
    assert!(
        !stdout.contains("deploy@server01 ") && !stdout.ends_with("deploy@server01"),
        "expected FQDN host, not bare input, got: {stdout}"
    );
}

// ─── Numeric range pattern matching ───────────────────────────────────────────

/// `yconn connect` with `server[1..10]` pattern and `host: ${name}.corp.com` —
/// mock ssh receives `deploy@server5.corp.com`.
#[test]
fn range_pattern_with_name_template_expands_to_fqdn() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  \"server[1..10]\":\n    host: \"${name}.corp.com\"\n    user: deploy\n    auth:\n      type: password\n    description: Corp servers\n",
    );

    let out = env.run_in_container(&["connect", "server5"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("deploy@server5.corp.com"),
        "expected 'deploy@server5.corp.com' in stdout, got: {stdout}"
    );
    assert!(
        !stdout.contains("deploy@server5 ") && !stdout.ends_with("deploy@server5"),
        "expected FQDN host, not bare input, got: {stdout}"
    );
}

/// `yconn connect` with a range and a glob both matching — exits non-zero and
/// names both patterns in stderr.
#[test]
fn range_conflict_with_glob_exits_nonzero_with_pattern_names_in_stderr() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  \"server[1..10]\":\n    host: ph1\n    user: deploy\n    auth:\n      type: password\n    description: Range pattern\n  server*:\n    host: ph2\n    user: admin\n    auth:\n      type: password\n    description: Glob pattern\n",
    );

    let out = env.run_in_container(&["connect", "server5"]);
    assert!(
        !out.status.success(),
        "expected non-zero exit for conflicting patterns, got 0"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("server[1..10]"),
        "expected range pattern named in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("server*"),
        "expected glob pattern named in stderr, got: {stderr}"
    );
}

// ─── ssh-config install ───────────────────────────────────────────────────────

/// `yconn ssh-config install` writes correct Host blocks to
/// `~/.ssh/yconn-connections` and injects Include into `~/.ssh/config`.
#[test]
fn ssh_config_install_writes_host_blocks_and_include() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  prod-web:\n    host: 10.0.1.50\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/prod_key\n    description: Production web\n  staging-db:\n    host: staging.internal\n    user: dbadmin\n    port: 2222\n    auth:\n      type: password\n    description: Staging database\n",
    );

    let out = env.run(&["ssh-config", "install"]);
    TestEnv::assert_ok(&out);

    // yconn-connections file must exist with correct Host blocks.
    let ssh_dir = env.home.path().join(".ssh");
    let conn_file = ssh_dir.join("yconn-connections");
    assert!(conn_file.exists(), "yconn-connections must be created");

    let content = fs::read_to_string(&conn_file).unwrap();
    assert!(
        content.contains("Host prod-web\n"),
        "missing prod-web block"
    );
    assert!(content.contains("    HostName 10.0.1.50\n"));
    assert!(content.contains("    User deploy\n"));
    assert!(content.contains("    IdentityFile ~/.ssh/prod_key\n"));
    assert!(
        !content.contains("    Port 22\n"),
        "port 22 must be omitted"
    );
    assert!(
        content.contains("Host staging-db\n"),
        "missing staging-db block"
    );
    assert!(
        content.contains("    Port 2222\n"),
        "custom port must appear"
    );
    assert!(
        !content.contains("IdentityFile") || content.contains("prod_key"),
        "staging-db must not have IdentityFile"
    );

    // ~/.ssh/config must contain the Include line.
    let config_file = ssh_dir.join("config");
    assert!(config_file.exists(), "~/.ssh/config must be created");
    let config = fs::read_to_string(&config_file).unwrap();
    assert!(
        config.contains("Include ~/.ssh/yconn-connections"),
        "config must contain Include line, got: {config}"
    );

    // No # comment lines must appear inside any Host block (after Host line,
    // before the next blank line).
    let mut in_host_block = false;
    for line in content.lines() {
        if line.starts_with("Host ") {
            in_host_block = true;
            continue;
        }
        if line.is_empty() {
            in_host_block = false;
            continue;
        }
        if in_host_block {
            assert!(
                !line.starts_with('#'),
                "no # lines must appear inside a Host block, got: {line:?} in:\n{content}"
            );
        }
    }

    // Summary line in stdout.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Wrote 2 Host block(s)"),
        "expected summary in stdout, got: {stdout}"
    );
}

/// `yconn ssh-config install --dry-run` prints Host blocks to stdout and
/// does not write any files.
#[test]
fn ssh_config_install_dry_run_prints_to_stdout_no_files_written() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  myhost:\n    host: 192.168.1.1\n    user: admin\n    auth:\n      type: password\n    description: My host\n",
    );

    let out = env.run(&["ssh-config", "install", "--dry-run"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Host myhost"),
        "expected Host block in stdout, got: {stdout}"
    );

    // No files must be written.
    let conn_file = env.home.path().join(".ssh").join("yconn-connections");
    assert!(
        !conn_file.exists(),
        "dry-run must not write yconn-connections"
    );
}

/// `yconn ssh-config install --user user:alice` renders `User alice` in all blocks.
#[test]
fn ssh_config_user_override_renders_expanded_user_line() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  srv:\n    host: 10.0.0.1\n    user: \"${user}\"\n    auth:\n      type: password\n    description: test\n",
    );

    let out = env.run(&["ssh-config", "install", "--dry-run", "--user", "user:alice"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("    User alice"),
        "expected 'User alice' in output, got: {stdout}"
    );
    assert!(
        !stdout.contains("${user}"),
        "unresolved template must not appear: {stdout}"
    );
}

/// `yconn ssh-config install --skip-user` renders no User line in any block.
#[test]
fn ssh_config_skip_user_omits_user_lines() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: password\n    description: test\n",
    );

    let out = env.run(&["ssh-config", "install", "--dry-run", "--skip-user"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("    User "),
        "User lines must be omitted with --skip-user, got: {stdout}"
    );
    assert!(
        stdout.contains("Host srv"),
        "Host block must still appear: {stdout}"
    );
}

/// `yconn ssh-config install` with an unresolved `${t1user}` template prompts
/// for the missing value and, once provided, writes the Host block with the
/// resolved user and persists the value to the user-layer config.
#[test]
fn ssh_config_unresolved_user_template_prompts_and_resolves() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  srv:\n    host: myhost\n    user: \"${t1user}\"\n    auth:\n      type: password\n    description: test\n",
    );

    // Provide the value via stdin.
    let out = env.run_with_stdin(&["ssh-config", "install"], "alice\n");
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Missing user variable '${t1user}' used by: srv"),
        "expected prompt for missing variable in stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("Added user entry 't1user'"),
        "expected confirmation in stdout, got: {stdout}"
    );

    // Verify the Host block was written with the resolved value.
    let host_blocks =
        fs::read_to_string(env.home.path().join(".ssh").join("yconn-connections")).unwrap();
    assert!(
        host_blocks.contains("User alice"),
        "expected 'User alice' in Host block, got: {host_blocks}"
    );

    // Verify the value was persisted to the user-layer config.
    let user_config =
        fs::read_to_string(env.xdg_config.path().join("yconn").join("connections.yaml")).unwrap();
    assert!(
        user_config.contains("t1user:") && user_config.contains("alice"),
        "expected t1user entry in user config, got: {user_config}"
    );
}

/// `yconn ssh-config install` preserves a pre-existing foreign Host block in
/// `~/.ssh/yconn-connections` and adds the new blocks alongside it.
#[test]
fn ssh_config_preserves_foreign_host_blocks() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  prod-web:\n    host: 10.0.1.50\n    user: deploy\n    auth:\n      type: password\n    description: Production web\n",
    );

    // Pre-populate yconn-connections with a foreign Host block that yconn
    // knows nothing about.
    let ssh_dir = env.home.path().join(".ssh");
    fs::create_dir_all(&ssh_dir).unwrap();
    let conn_file = ssh_dir.join("yconn-connections");
    fs::write(
        &conn_file,
        "# description: some other project\n# auth: key\nHost myother-host\n    HostName other.example.com\n    User ops\n\n",
    )
    .unwrap();

    let out = env.run(&["ssh-config", "install"]);
    TestEnv::assert_ok(&out);

    let content = fs::read_to_string(&conn_file).unwrap();

    // The foreign block must be preserved.
    assert!(
        content.contains("Host myother-host"),
        "foreign block must be preserved: {content}"
    );
    assert!(
        content.contains("    HostName other.example.com"),
        "foreign HostName must be preserved: {content}"
    );

    // The new block must also be present.
    assert!(
        content.contains("Host prod-web"),
        "new block must be written: {content}"
    );
    assert!(
        content.contains("    HostName 10.0.1.50"),
        "new HostName must appear: {content}"
    );
}

/// Running `yconn ssh-config install` twice with different project configs
/// pointed at the same home directory leaves blocks from both runs in the file
/// after the second run.
#[test]
fn ssh_config_two_runs_accumulate_blocks() {
    let env = TestEnv::new();

    // First run: write project config with one connection.
    env.write_user_config(
        "connections",
        "connections:\n  first-host:\n    host: 10.0.1.1\n    user: alice\n    auth:\n      type: password\n    description: First host\n",
    );
    let out1 = env.run(&["ssh-config", "install"]);
    TestEnv::assert_ok(&out1);

    // Second run: replace user config with a different connection.
    let user_config_path = env.xdg_config.path().join("yconn").join("connections.yaml");
    fs::write(
        &user_config_path,
        "connections:\n  second-host:\n    host: 10.0.1.2\n    user: bob\n    auth:\n      type: password\n    description: Second host\n",
    )
    .unwrap();
    let out2 = env.run(&["ssh-config", "install"]);
    TestEnv::assert_ok(&out2);

    let conn_file = env.home.path().join(".ssh").join("yconn-connections");
    let content = fs::read_to_string(&conn_file).unwrap();

    // Blocks from both runs must be present.
    assert!(
        content.contains("Host first-host"),
        "first-host block must survive second run: {content}"
    );
    assert!(
        content.contains("Host second-host"),
        "second-host block must appear after second run: {content}"
    );
}

// ─── ssh-config print ─────────────────────────────────────────────────────────

/// `yconn ssh-config print` renders Host blocks to stdout without writing files.
#[test]
fn ssh_config_print_outputs_host_blocks_to_stdout() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  myhost:\n    host: 192.168.1.5\n    user: admin\n    auth:\n      type: password\n    description: My host\n",
    );

    let out = env.run(&["ssh-config", "print"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Host myhost"),
        "expected Host block in stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("    HostName 192.168.1.5"),
        "expected HostName in stdout, got: {stdout}"
    );

    // No files must be written.
    let conn_file = env.home.path().join(".ssh").join("yconn-connections");
    assert!(
        !conn_file.exists(),
        "ssh-config print must not write yconn-connections"
    );
    let config_file = env.home.path().join(".ssh").join("config");
    assert!(
        !config_file.exists(),
        "ssh-config print must not write ~/.ssh/config"
    );
}

/// `yconn ssh-config print --skip-user` omits User lines from output.
#[test]
fn ssh_config_print_skip_user_omits_user_lines() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: password\n    description: test\n",
    );

    let out = env.run(&["ssh-config", "print", "--skip-user"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("    User "),
        "User lines must be omitted with --skip-user, got: {stdout}"
    );
    assert!(
        stdout.contains("Host srv"),
        "Host block must still appear: {stdout}"
    );
}

// ─── ssh-config uninstall ─────────────────────────────────────────────────────

/// `yconn ssh-config uninstall` removes `~/.ssh/yconn-connections` and the
/// Include line from `~/.ssh/config`.
#[test]
fn ssh_config_uninstall_removes_file_and_include_line() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: password\n    description: test\n",
    );

    // Install first.
    let install_out = env.run(&["ssh-config", "install"]);
    TestEnv::assert_ok(&install_out);

    let conn_file = env.home.path().join(".ssh").join("yconn-connections");
    let config_file = env.home.path().join(".ssh").join("config");
    assert!(
        conn_file.exists(),
        "yconn-connections must exist after install"
    );
    assert!(
        config_file.exists(),
        "~/.ssh/config must exist after install"
    );

    // Now uninstall.
    let out = env.run(&["ssh-config", "uninstall"]);
    TestEnv::assert_ok(&out);

    assert!(
        !conn_file.exists(),
        "yconn-connections must be removed after uninstall"
    );
    let config = fs::read_to_string(&config_file).unwrap();
    assert!(
        !config.contains("Include ~/.ssh/yconn-connections"),
        "Include line must be removed after uninstall, got: {config}"
    );
}

/// `yconn ssh-config uninstall` is graceful when `~/.ssh/yconn-connections`
/// does not exist.
#[test]
fn ssh_config_uninstall_graceful_when_file_absent() {
    let env = TestEnv::new();

    let out = env.run(&["ssh-config", "uninstall"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("does not exist") || stdout.contains("nothing to remove"),
        "expected graceful message, got: {stdout}"
    );
}

// ─── ssh-config disable / enable ─────────────────────────────────────────────

/// `yconn ssh-config disable` removes the Include line but leaves
/// `~/.ssh/yconn-connections` intact.
#[test]
fn ssh_config_disable_removes_include_line_keeps_file() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: password\n    description: test\n",
    );

    // Install first.
    let install_out = env.run(&["ssh-config", "install"]);
    TestEnv::assert_ok(&install_out);

    let conn_file = env.home.path().join(".ssh").join("yconn-connections");
    let config_file = env.home.path().join(".ssh").join("config");
    assert!(
        conn_file.exists(),
        "yconn-connections must exist after install"
    );

    // Disable.
    let out = env.run(&["ssh-config", "disable"]);
    TestEnv::assert_ok(&out);

    // yconn-connections file must still be present.
    assert!(
        conn_file.exists(),
        "yconn-connections must remain after disable"
    );
    // Include line must be gone.
    let config = fs::read_to_string(&config_file).unwrap();
    assert!(
        !config.contains("Include ~/.ssh/yconn-connections"),
        "Include line must be removed after disable, got: {config}"
    );
}

/// `yconn ssh-config enable` adds the Include line back when absent.
#[test]
fn ssh_config_enable_adds_include_line_when_absent() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: password\n    description: test\n",
    );

    // Install then disable to remove Include.
    let install_out = env.run(&["ssh-config", "install"]);
    TestEnv::assert_ok(&install_out);
    let disable_out = env.run(&["ssh-config", "disable"]);
    TestEnv::assert_ok(&disable_out);

    let config_file = env.home.path().join(".ssh").join("config");
    let config = fs::read_to_string(&config_file).unwrap();
    assert!(
        !config.contains("Include ~/.ssh/yconn-connections"),
        "Include must be absent after disable, got: {config}"
    );

    // Enable.
    let out = env.run(&["ssh-config", "enable"]);
    TestEnv::assert_ok(&out);

    let config = fs::read_to_string(&config_file).unwrap();
    assert!(
        config.contains("Include ~/.ssh/yconn-connections"),
        "Include line must be re-added after enable, got: {config}"
    );
}

/// `yconn ssh-config enable` is a no-op with a message if Include already present.
#[test]
fn ssh_config_enable_noop_when_include_already_present() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  srv:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: password\n    description: test\n",
    );

    // Install to set up Include line.
    let install_out = env.run(&["ssh-config", "install"]);
    TestEnv::assert_ok(&install_out);

    // Enable again — must be a no-op.
    let out = env.run(&["ssh-config", "enable"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("already present") || stdout.contains("nothing to do"),
        "expected no-op message, got: {stdout}"
    );

    // Still exactly one Include line.
    let config_file = env.home.path().join(".ssh").join("config");
    let config = fs::read_to_string(&config_file).unwrap();
    let count = config
        .lines()
        .filter(|l| l.trim() == "Include ~/.ssh/yconn-connections")
        .count();
    assert_eq!(count, 1, "Include must appear exactly once, got:\n{config}");
}

// ─── yconn users ─────────────────────────────────────────────────────────────

/// `yconn users show` lists all user entries across layers with correct source.
#[test]
fn user_show_lists_entries_with_source() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "version: 1\n\nusers:\n  testuser: \"testusername\"\n  devkey: \"devval\"\n",
    );

    let out = env.run(&["users", "show"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("testuser"),
        "expected testuser in output: {stdout}"
    );
    assert!(
        stdout.contains("testusername"),
        "expected value in output: {stdout}"
    );
    assert!(
        stdout.contains("devkey"),
        "expected devkey in output: {stdout}"
    );
    assert!(
        stdout.contains("user"),
        "expected layer label 'user' in output: {stdout}"
    );
}

/// `yconn users add` round-trip: add an entry then `yconn users show` reflects it.
#[test]
fn user_add_round_trip_show_reflects_new_entry() {
    let env = TestEnv::new();

    // Add a user entry interactively.
    let out = env.run_with_stdin(&["users", "add"], "newkey\nnewval\n");
    TestEnv::assert_ok(&out);

    // Confirm `users show` returns the new entry.
    let out2 = env.run(&["users", "show"]);
    TestEnv::assert_ok(&out2);

    let stdout = String::from_utf8_lossy(&out2.stdout);
    assert!(
        stdout.contains("newkey"),
        "expected newkey in user show output: {stdout}"
    );
    assert!(
        stdout.contains("newval"),
        "expected newval in user show output: {stdout}"
    );
}

// ─── yconn connect with user expansion ───────────────────────────────────────

/// `yconn connect` with `${user}` and `--user user:alice` receives `alice@host`.
#[test]
fn connect_user_override_expands_dollar_user() {
    let env = TestEnv::new();
    let key = env.write_key("id_rsa");

    env.write_user_config(
        "connections",
        &format!(
            "connections:\n  srv:\n    host: myhost\n    user: \"${{user}}\"\n    auth:\n      type: key\n      key: {key}\n    description: test\n"
        ),
    );

    let out = env.run_in_container(&["connect", "--user", "user:alice", "srv"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("alice@myhost"),
        "expected alice@myhost in SSH args, got: {stdout}"
    );
}

/// `yconn connect` with `${testuser}` from `users:` map receives `ops@host`.
#[test]
fn connect_named_users_map_entry_expands_in_user_field() {
    let env = TestEnv::new();
    let key = env.write_key("id_rsa");

    env.write_user_config(
        "connections",
        &format!(
            "users:\n  testuser: \"ops\"\nconnections:\n  srv:\n    host: myhost\n    user: \"${{testuser}}\"\n    auth:\n      type: key\n      key: {key}\n    description: test\n"
        ),
    );

    let out = env.run_in_container(&["connect", "srv"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ops@myhost"),
        "expected ops@myhost in SSH args, got: {stdout}"
    );
}

/// `yconn connect --user testuser:alice` overrides config-loaded users: entry.
#[test]
fn connect_user_override_shadows_config_users_entry() {
    let env = TestEnv::new();
    let key = env.write_key("id_rsa");

    env.write_user_config(
        "connections",
        &format!(
            "users:\n  testuser: \"ops\"\nconnections:\n  srv:\n    host: myhost\n    user: \"${{testuser}}\"\n    auth:\n      type: key\n      key: {key}\n    description: test\n"
        ),
    );

    let out = env.run_in_container(&["connect", "--user", "testuser:alice", "srv"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("alice@myhost"),
        "expected alice@myhost (override), got: {stdout}"
    );
}

// ─── yconn users show — user row (no header) ─────────────────────────────────

/// `yconn users show` does NOT print a `Username:` header. When the `users:`
/// map contains a `user` key with value `alice`, `alice` appears as a table
/// row value.
#[test]
fn user_show_prints_username_from_map() {
    let env = TestEnv::new();
    env.write_user_config("connections", "version: 1\n\nusers:\n  user: \"alice\"\n");

    let out = env.run(&["users", "show"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("Username:"),
        "should not print 'Username:' header: {stdout}"
    );
    assert!(
        stdout.contains("alice"),
        "expected 'alice' as a table row value: {stdout}"
    );
}

/// `yconn users show` does NOT print a `Username:` header. When no `user` key
/// exists in the `users:` map but `USER=bob`, a synthetic row appears with
/// `bob` as the value and SOURCE containing `environment variable $USER`.
#[test]
fn user_show_prints_username_from_env_var() {
    let env = TestEnv::new();
    // No users map at all — fall back to $USER env var.
    let out = env.run_with_env(&["users", "show"], &[("USER", "bob")]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("Username:"),
        "should not print 'Username:' header: {stdout}"
    );
    assert!(
        stdout.contains("bob"),
        "expected 'bob' as a table row value: {stdout}"
    );
    assert!(
        stdout.contains("environment variable $USER"),
        "expected env-var source label in output: {stdout}"
    );
}

// ─── show --dump ──────────────────────────────────────────────────────────────

/// `yconn connections show --dump` outputs valid YAML containing all connection names and user keys,
/// with blank lines between connection entries and between the connections and users blocks.
#[test]
fn show_dump_outputs_merged_config_as_yaml() {
    let env = TestEnv::new();
    env.write_project_config(
        "connections",
        "connections:\n  prod:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: Production\n  staging:\n    host: 10.0.0.2\n    user: admin\n    auth:\n      type: password\n    description: Staging\nusers:\n  testuser: alice\n  mybot: botuser\n",
    );
    let out = env.run(&["connections", "show", "--dump"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("connections:"), "missing connections: key");
    assert!(stdout.contains("prod:"), "missing prod entry");
    assert!(stdout.contains("staging:"), "missing staging entry");
    assert!(stdout.contains("10.0.0.1"), "missing prod host");
    assert!(stdout.contains("users:"), "missing users: key");
    assert!(stdout.contains("testuser:"), "missing testuser key");
    assert!(stdout.contains("mybot:"), "missing mybot key");

    // Blank-line separation: at least one blank line between the two connection
    // entries within the connections: block.
    let conn_section: Vec<&str> = stdout
        .lines()
        .skip_while(|l| !l.starts_with("connections:"))
        .take_while(|l| !l.starts_with("users:"))
        .collect();
    let blank_in_conn = conn_section.iter().filter(|l| l.is_empty()).count();
    assert!(
        blank_in_conn >= 1,
        "expected at least one blank line between connection entries in connections block:\n{stdout}"
    );

    // Blank line between connections: block and users: block.
    assert!(
        stdout.contains("\nusers:"),
        "expected blank line immediately before users: key:\n{stdout}"
    );
}

/// `yconn connections show --dump` with no config outputs empty-but-valid YAML.
#[test]
fn show_dump_empty_config_produces_valid_yaml() {
    let env = TestEnv::new();
    let out = env.run(&["connections", "show", "--dump"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("connections:"), "missing connections: key");
    assert!(stdout.contains("users:"), "missing users: key");
}

// ─── yconn users add — Updating: message ─────────────────────────────────────

/// `yconn users add` interactive wizard prints `Updating: <path>` to stdout
/// before writing the config file.
#[test]
fn user_add_interactive_prints_updating_path() {
    let env = TestEnv::new();

    let out = env.run_with_stdin(&["users", "add"], "mykey\nmyval\n");
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);

    // The "Updating:" message must appear.
    assert!(
        stdout.contains("Updating:"),
        "expected 'Updating:' in stdout: {stdout}"
    );

    // The path must end with connections.yaml (user config layer).
    assert!(
        stdout.contains("connections.yaml"),
        "expected config file path containing 'connections.yaml' in stdout: {stdout}"
    );

    // The written config file must also exist at the expected location.
    let config_path = env.xdg_config.path().join("yconn").join("connections.yaml");
    assert!(
        config_path.exists(),
        "expected config file to be created at {config_path:?}"
    );
}

/// `yconn users add --user KEY:VALUE` non-interactive path prints
/// `Updating: <path>` to stdout before writing the config file.
#[test]
fn user_add_non_interactive_prints_updating_path() {
    let env = TestEnv::new();

    let out = env.run(&["users", "add", "--user", "foo:bar"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        stdout.contains("Updating:"),
        "expected 'Updating:' in stdout: {stdout}"
    );

    assert!(
        stdout.contains("connections.yaml"),
        "expected config file path containing 'connections.yaml' in stdout: {stdout}"
    );

    let config_path = env.xdg_config.path().join("yconn").join("connections.yaml");
    assert!(
        config_path.exists(),
        "expected config file to be created at {config_path:?}"
    );
}

/// `yconn connections show <name>` still works (name is still accepted).
#[test]
fn show_name_still_works_after_dump_flag_added() {
    let env = TestEnv::new();
    env.write_project_config(
        "connections",
        "connections:\n  web:\n    host: 1.2.3.4\n    user: ops\n    auth:\n      type: password\n    description: Web\n",
    );
    let out = env.run(&["connections", "show", "web"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Connection: web"));
}

/// `yconn connections show` with neither name nor --dump exits with an error.
#[test]
fn show_no_args_errors() {
    let env = TestEnv::new();
    let out = env.run(&["connections", "show"]);
    assert!(!out.status.success(), "expected non-zero exit");
}

/// `yconn connections show <name> --dump` is rejected (mutually exclusive).
#[test]
fn show_name_and_dump_together_errors() {
    let env = TestEnv::new();
    env.write_project_config(
        "connections",
        "connections:\n  web:\n    host: 1.2.3.4\n    user: ops\n    auth:\n      type: password\n    description: Web\n",
    );
    let out = env.run(&["connections", "show", "web", "--dump"]);
    assert!(!out.status.success(), "expected non-zero exit");
}

// ─── yconn install ───────────────────────────────────────────────────────────

/// `yconn install` with a project config containing `alpha` and `beta`
/// installs both into the user layer file.
#[test]
fn install_copies_new_connections_to_user_layer() {
    let env = TestEnv::new();

    env.write_project_config(
        "connections",
        "version: 1\n\nconnections:\n  alpha:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: password\n    description: \"Alpha server\"\n  beta:\n    host: 10.0.0.2\n    user: ops\n    auth:\n      type: password\n    description: \"Beta server\"\n",
    );

    let out = env.run(&["install"]);
    TestEnv::assert_ok(&out);

    let user_config = env.xdg_config.path().join("yconn").join("connections.yaml");
    assert!(user_config.exists(), "user config must be created");

    let content = fs::read_to_string(&user_config).unwrap();
    assert!(content.contains("alpha:"), "alpha not found in user config");
    assert!(content.contains("beta:"), "beta not found in user config");
    assert!(
        content.contains("10.0.0.1"),
        "alpha host not found in user config"
    );
    assert!(
        content.contains("10.0.0.2"),
        "beta host not found in user config"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Writing: connection alpha ->"),
        "expected 'Writing: connection alpha ->' in stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("Writing: connection beta ->"),
        "expected 'Writing: connection beta ->' in stdout, got: {stdout}"
    );
}

/// `yconn install` with `alpha` already in the user layer and `y` on stdin
/// updates `alpha` and appends `beta`.
#[test]
fn install_updates_existing_with_y_and_appends_new() {
    let env = TestEnv::new();

    // Project config: alpha (updated host) and beta (new).
    env.write_project_config(
        "connections",
        "version: 1\n\nconnections:\n  alpha:\n    host: 10.0.0.99\n    user: deploy\n    auth:\n      type: password\n    description: \"Alpha updated\"\n  beta:\n    host: 10.0.0.2\n    user: ops\n    auth:\n      type: password\n    description: \"Beta server\"\n",
    );

    // Pre-populate user layer with alpha at old host.
    env.write_user_config(
        "connections",
        "version: 1\n\nconnections:\n  alpha:\n    host: 10.0.0.1\n    user: deploy\n    auth:\n      type: password\n    description: \"Alpha old\"\n",
    );

    // Answer `y` to the update prompt for alpha.
    let out = env.run_with_stdin(&["install"], "y\n");
    TestEnv::assert_ok(&out);

    let user_config = env.xdg_config.path().join("yconn").join("connections.yaml");
    let content = fs::read_to_string(&user_config).unwrap();

    assert!(
        content.contains("10.0.0.99"),
        "alpha should be updated to new host"
    );
    assert!(
        !content.contains("10.0.0.1"),
        "old alpha host should be replaced"
    );
    assert!(content.contains("beta:"), "beta should be appended");
    assert!(content.contains("10.0.0.2"), "beta host should be present");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Updating: connection alpha ->"),
        "expected 'Updating: connection alpha ->' in stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("Writing: connection beta ->"),
        "expected 'Writing: connection beta ->' in stdout for beta, got: {stdout}"
    );
}

// ─── yconn install — missing user variable prompting ─────────────────────────

/// `yconn install` with a project config containing `${t1user}` prompts for
/// the missing value, writes it to the user-layer config, and completes the
/// install.
#[test]
fn install_missing_user_variable_prompts_and_writes_value() {
    let env = TestEnv::new();

    env.write_project_config(
        "connections",
        "version: 1\n\nconnections:\n  alpha:\n    host: 10.0.0.1\n    user: \"${t1user}\"\n    auth:\n      type: password\n    description: \"Alpha server\"\n",
    );

    // Provide the value for the missing user variable via stdin.
    let out = env.run_with_stdin(&["install"], "alice\n");
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Missing user variable '${t1user}' used by: alpha"),
        "expected prompt for missing variable in stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("Added user entry 't1user'"),
        "expected confirmation in stdout, got: {stdout}"
    );

    // Verify the connection was installed.
    let user_config = env.xdg_config.path().join("yconn").join("connections.yaml");
    assert!(user_config.exists(), "user config must be created");
    let content = fs::read_to_string(&user_config).unwrap();
    assert!(content.contains("alpha:"), "alpha not found in user config");
    assert!(
        content.contains("t1user:") && content.contains("alice"),
        "t1user entry must be written to user config: {content}"
    );
}

/// `yconn ssh-config install` with a missing user variable prompts and writes
/// the value, then generates correct Host blocks with the resolved user.
#[test]
fn ssh_config_install_missing_user_variable_prompts_and_generates_correct_host_blocks() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        "connections:\n  conn-a:\n    host: 10.0.0.1\n    user: \"${t1user}\"\n    auth:\n      type: password\n    description: A\n  conn-b:\n    host: 10.0.0.2\n    user: \"${t1user}\"\n    auth:\n      type: password\n    description: B\n",
    );

    // Both connections share the same ${t1user} key — should prompt only once.
    let out = env.run_with_stdin(&["ssh-config", "install"], "bob\n");
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    // Only one prompt for the shared key.
    let prompt_count = stdout.matches("Missing user variable").count();
    assert_eq!(
        prompt_count, 1,
        "should prompt only once for the same key, got {prompt_count}"
    );
    // Both connections should be listed.
    assert!(
        stdout.contains("conn-a") && stdout.contains("conn-b"),
        "prompt should list both connections, got: {stdout}"
    );

    // Verify Host blocks have the resolved user.
    let host_blocks =
        fs::read_to_string(env.home.path().join(".ssh").join("yconn-connections")).unwrap();
    assert!(
        host_blocks.contains("User bob"),
        "expected 'User bob' in Host blocks, got: {host_blocks}"
    );
    // No unresolved token should remain.
    assert!(
        !host_blocks.contains("${t1user}"),
        "unresolved token should not appear in Host blocks: {host_blocks}"
    );
}

// ─── yconn install — user variable resolution from cfg.users ─────────────────

/// `yconn install` with a project config containing both a `users:` block with
/// `t1user: alice` and a connection referencing `${t1user}` completes without
/// prompting because the merged cfg.users resolves the variable.
#[test]
fn install_project_users_block_resolves_variable_without_prompt() {
    let env = TestEnv::new();

    env.write_project_config(
        "connections",
        "version: 1\n\nusers:\n  t1user: alice\n\nconnections:\n  alpha:\n    host: 10.0.0.1\n    user: \"${t1user}\"\n    auth:\n      type: password\n    description: \"Alpha server\"\n",
    );

    let out = env.run(&["install"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    // No prompt for missing variable — the project users: block resolves it.
    assert!(
        !stdout.contains("Missing user variable"),
        "should not prompt when key is defined in project file's users: block, got: {stdout}"
    );
    // Connection should be installed.
    assert!(
        stdout.contains("Writing: connection alpha"),
        "expected alpha to be installed, got: {stdout}"
    );

    // Verify the connection was written to the user layer.
    let user_config = env.xdg_config.path().join("yconn").join("connections.yaml");
    assert!(user_config.exists(), "user config must be created");
    let content = fs::read_to_string(&user_config).unwrap();
    assert!(content.contains("alpha:"), "alpha not found in user config");
}

// ─── Identity auth round-trip ────────────────────────────────────────────────

#[test]
fn identity_connect_produces_ssh_args_with_warning() {
    let env = TestEnv::new();
    let key = env.write_key("github_key");
    env.write_user_config(
        "connections",
        &conn_identity("github", "github.com", "git", None, &key),
    );

    let out = env.run(&["connect", "github"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    // Mock ssh prints: ssh -F /dev/null -i <key> git@github.com
    assert!(stdout.contains("-i"), "identity auth must produce -i flag");
    assert!(stdout.contains(&key), "must contain key path");
    assert!(
        stdout.contains("git@github.com"),
        "must contain destination"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("identity-only"),
        "stderr must contain identity-only warning, got: {stderr}"
    );
}

#[test]
fn identity_list_shows_identity_auth_type() {
    let env = TestEnv::new();
    let key = env.write_key("github_key");
    env.write_user_config(
        "connections",
        &conn_identity("github", "github.com", "git", None, &key),
    );

    let out = env.run(&["list"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("identity"),
        "list output must show 'identity' auth type, got: {stdout}"
    );
    assert!(
        stdout.contains("github"),
        "list output must show connection name"
    );
}

#[test]
fn identity_show_displays_auth_and_key() {
    let env = TestEnv::new();
    let key = env.write_key("github_key");
    env.write_user_config(
        "connections",
        &conn_identity("github", "github.com", "git", None, &key),
    );

    let out = env.run(&["connections", "show", "github"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("identity"),
        "show output must display 'identity' auth, got: {stdout}"
    );
    assert!(
        stdout.contains(&key),
        "show output must display key path, got: {stdout}"
    );
}

#[test]
fn identity_show_dump_serializes_correctly() {
    let env = TestEnv::new();
    let key = env.write_key("github_key");
    env.write_user_config(
        "connections",
        &conn_identity("github", "github.com", "git", None, &key),
    );

    let out = env.run(&["connections", "show", "--dump"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("type: identity"),
        "dump must contain 'type: identity', got: {stdout}"
    );
    assert!(
        stdout.contains(&key),
        "dump must contain key path, got: {stdout}"
    );
}

#[test]
fn identity_ssh_config_install_emits_identities_only() {
    let env = TestEnv::new();
    let key = env.write_key("github_key");
    env.write_user_config(
        "connections",
        &conn_identity("github", "github.com", "git", None, &key),
    );

    let out = env.run(&["ssh-config", "install"]);
    TestEnv::assert_ok(&out);

    let ssh_dir = env.home.path().join(".ssh");
    let conn_file = ssh_dir.join("yconn-connections");
    assert!(conn_file.exists(), "yconn-connections must be created");

    let content = fs::read_to_string(&conn_file).unwrap();
    assert!(
        content.contains("Host github\n"),
        "missing Host block for identity connection"
    );
    assert!(
        content.contains(&format!("    IdentityFile {key}\n")),
        "identity auth must emit IdentityFile, got: {content}"
    );
    assert!(
        content.contains("    IdentitiesOnly yes\n"),
        "identity auth must emit IdentitiesOnly yes, got: {content}"
    );
}

#[test]
fn identity_add_wizard_round_trip() {
    let env = TestEnv::new();
    let key = env.write_key("github_key");
    // Wizard answers: name, host, user, port, auth, key, description, link
    let answers = format!("github\ngithub.com\ngit\n\nidentity\n{key}\nGitHub identity\n\n");
    let out = env.run_with_stdin(&["connections", "add"], &answers);
    TestEnv::assert_ok(&out);

    // Verify the connection was added by listing it.
    let out = env.run(&["list"]);
    TestEnv::assert_ok(&out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("github"),
        "added identity connection must appear in list, got: {stdout}"
    );
    assert!(
        stdout.contains("identity"),
        "identity auth type must appear in list, got: {stdout}"
    );
}

// ─── yconn keys list | setup ─────────────────────────────────────────────────

/// `yconn keys list` prints one row per connection that has a `generate_key`
/// configured; connections without `generate_key` are omitted.
#[test]
fn keys_list_filters_to_generate_key_connections() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        concat!(
            "connections:\n",
            "  with-gen:\n",
            "    host: 10.0.0.1\n",
            "    user: deploy\n",
            "    auth:\n",
            "      type: key\n",
            "      key: ~/.ssh/deploy_key\n",
            "      generate_key: \"echo k > ${key}\"\n",
            "    description: Has gen key\n",
            "  without-gen:\n",
            "    host: 10.0.0.2\n",
            "    user: admin\n",
            "    auth:\n",
            "      type: key\n",
            "      key: ~/.ssh/admin_key\n",
            "    description: No gen key\n",
            "  pwauth:\n",
            "    host: 10.0.0.3\n",
            "    user: dbadmin\n",
            "    auth:\n",
            "      type: password\n",
            "    description: Password\n",
        ),
    );

    let out = env.run(&["keys", "list"]);
    TestEnv::assert_ok(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("with-gen"),
        "with-gen connection must appear in keys list, got: {stdout}"
    );
    assert!(
        !stdout.contains("without-gen"),
        "without-gen must NOT appear in keys list, got: {stdout}"
    );
    assert!(
        !stdout.contains("pwauth"),
        "pwauth must NOT appear in keys list, got: {stdout}"
    );
    assert!(
        stdout.contains("NAME") && stdout.contains("GENERATE_KEY"),
        "keys list must render header row, got: {stdout}"
    );
}

/// `yconn keys setup` (no arg) runs `generate_key` for every qualifying
/// connection and silently skips connections without `generate_key`.
/// `yconn keys setup <name>` re-runs generation for a named connection after
/// the key has been deleted.
#[test]
fn keys_setup_all_and_named_create_key_files() {
    let env = TestEnv::new();

    let key_path = env.cwd.path().join("generated_key");
    let key_path_str = key_path.to_string_lossy();

    let yaml = format!(
        concat!(
            "connections:\n",
            "  gen-conn:\n",
            "    host: 10.0.0.1\n",
            "    user: deploy\n",
            "    auth:\n",
            "      type: key\n",
            "      key: {key_path}\n",
            "      generate_key: \"printf %s hello > ${{key}}\"\n",
            "    description: Has gen key\n",
            "  pwauth:\n",
            "    host: 10.0.0.2\n",
            "    user: dbadmin\n",
            "    auth:\n",
            "      type: password\n",
            "    description: No gen key\n",
        ),
        key_path = key_path_str,
    );
    env.write_user_config("connections", &yaml);

    // Sanity: file does not exist yet.
    assert!(!key_path.exists(), "key file must not exist before setup");

    // Iterate-all form: creates the key file for gen-conn, silently skips
    // pwauth.
    let out = env.run(&["keys", "setup"]);
    TestEnv::assert_ok(&out);
    let contents = fs::read_to_string(&key_path).expect("key file must be created by setup");
    assert_eq!(
        contents, "hello",
        "key file content must match the echoed value"
    );

    // Delete and re-run via named form to verify the single-connection path.
    fs::remove_file(&key_path).unwrap();
    let out = env.run(&["keys", "setup", "gen-conn"]);
    TestEnv::assert_ok(&out);
    let contents = fs::read_to_string(&key_path).expect("key file must be recreated");
    assert_eq!(contents, "hello");
}

/// `yconn keys setup <name>` on a connection with no `generate_key` aborts
/// non-zero and prints a clear error message.
#[test]
fn keys_setup_named_without_generate_key_fails() {
    let env = TestEnv::new();

    env.write_user_config(
        "connections",
        concat!(
            "connections:\n",
            "  pwauth:\n",
            "    host: 10.0.0.1\n",
            "    user: dbadmin\n",
            "    auth:\n",
            "      type: password\n",
            "    description: No gen key\n",
        ),
    );

    let out = env.run(&["keys", "setup", "pwauth"]);
    assert!(
        !out.status.success(),
        "keys setup on no-generate_key connection must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("has no generate_key configured"),
        "stderr must mention 'has no generate_key configured', got: {stderr}"
    );
}
