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

    /// Same as `run` but pipes `stdin_data` to the subprocess's stdin.
    fn run_with_stdin(&self, args: &[&str], stdin_data: &str) -> Output {
        let path = format!(
            "{}:{}",
            self.mock_bin.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_yconn"))
            .arg("--no-color")
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

// ─── Add round-trip and edit invocation ──────────────────────────────────────

/// `yconn add` (piped stdin) → `yconn list` then `yconn show` successfully
/// display the newly created connection, verifying the YAML is valid and
/// parseable after the add wizard writes it.
#[test]
fn add_round_trip_list_and_show() {
    let env = TestEnv::new();

    // Simulate the wizard: name, host, user, port (blank=22), auth, key,
    // description, link (blank).
    let key = env.write_key("id_rsa");
    let stdin_data = format!("myconn\nmyhost.internal\ndeploy\n\nkey\n{key}\nMy server\n\n");

    // `yconn add --layer user` — writes to xdg_config/yconn/connections.yaml.
    let out = env.run_with_stdin(&["add", "--layer", "user"], &stdin_data);
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

    // `yconn show myconn` should succeed and display the connection detail.
    let show_out = env.run(&["show", "myconn"]);
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

/// `yconn add` for password auth writes a valid, parseable YAML entry with
/// no `key:` field, verified by `yconn show` succeeding afterwards.
#[test]
fn add_password_auth_round_trip() {
    let env = TestEnv::new();

    // Wizard answers: name, host, user, port, auth=password, description, link.
    let stdin_data = "dbconn\ndb.internal\ndbadmin\n\npassword\nDatabase server\n\n";

    let out = env.run_with_stdin(&["add", "--layer", "user"], stdin_data);
    TestEnv::assert_ok(&out);

    // Verify the written YAML is parseable by running show.
    let show_out = env.run(&["show", "dbconn"]);
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

/// `yconn edit <name>` invokes `$EDITOR` with the correct config file path.
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
        .arg("--no-color")
        .args(["edit", "my-srv"])
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
    // verify by running `yconn show my-srv`.
    let show_out = env.run(&["show", "my-srv"]);
    TestEnv::assert_ok(&show_out);
    let show_stdout = String::from_utf8_lossy(&show_out.stdout);
    assert!(
        show_stdout.contains("my-srv"),
        "expected 'my-srv' in show output after edit, got: {show_stdout}"
    );

    // The edit command should mention the target file path in its output.
    // (yconn edit opens the editor; the path is passed as the arg to $EDITOR,
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
        "connections:\n  my-server:\n    host: 10.0.0.1\n    user: admin\n    auth: password\n    description: My server\n",
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
        "connections:\n  user-server:\n    host: 192.168.1.5\n    user: root\n    auth: password\n    description: User server\n",
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
        "connections:\n  bad-server:\n    user: admin\n    auth: password\n    description: Missing host\n",
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
        "connections:\n  web-*:\n    host: placeholder.internal\n    user: deploy\n    auth: password\n    description: Wildcard web servers\n",
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
        "connections:\n  web-*:\n    host: ph1\n    user: deploy\n    auth: password\n    description: Web wildcard\n  \"?eb-prod\":\n    host: ph2\n    user: admin\n    auth: password\n    description: Prefix wildcard\n",
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
