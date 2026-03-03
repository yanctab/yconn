// Handler for `yconn connect <name>` — resolve a connection and invoke SSH,
// optionally bootstrapping into Docker first.

use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::config::LoadedConfig;
use crate::display::Renderer;
use crate::{connect, docker, security};

// ─── Public command entry point ───────────────────────────────────────────────

pub fn run(cfg: &LoadedConfig, renderer: &Renderer, name: &str, verbose: bool) -> Result<()> {
    let conn = cfg
        .find(name)
        .ok_or_else(|| anyhow!("no connection named '{name}'"))?;

    // Security: validate the key file before trying to connect.
    // Expand a leading `~` so that the existence and permission checks operate
    // on the real path — Path::new("~/.ssh/id_rsa") does not exist literally.
    if conn.auth == "key" {
        if let Some(ref key) = conn.key {
            let expanded = expand_tilde(key);
            for w in security::check_key_file(&expanded) {
                renderer.warn(&w.message);
            }
        }
    }

    // Docker bootstrap path: re-invoke inside container.
    if let Some(ref docker_cfg) = cfg.docker {
        if !docker::in_container() {
            let original_argv: Vec<String> = std::env::args().collect();
            docker::exec(docker_cfg, &original_argv, verbose, renderer)?;
            unreachable!("docker::exec replaced the process");
        }
    }

    // Direct SSH path: replace the current process with ssh.
    if verbose {
        let ssh_args = connect::build_args(conn);
        renderer.verbose_ssh_cmd(&ssh_args);
    }
    connect::exec(conn)
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Expand a leading `~` to the current user's home directory.
///
/// Only a literal leading `~/` (or the bare string `"~"`) is expanded.
/// `~username` forms are not supported. If `dirs::home_dir()` returns `None`,
/// the path is returned unchanged.
fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    use crate::config;
    use crate::display::Renderer;

    /// What `connect` would execute, without actually exec-ing.
    ///
    /// `in_container` is injected so tests can control container detection
    /// without relying on `/.dockerenv` or the `CONN_IN_DOCKER` env var.
    #[derive(Debug)]
    enum ConnectPlan {
        Docker(Vec<String>),
        Ssh(Vec<String>),
    }

    fn plan(
        cfg: &LoadedConfig,
        name: &str,
        original_argv: &[String],
        in_container: bool,
    ) -> Result<ConnectPlan> {
        let conn = cfg
            .find(name)
            .ok_or_else(|| anyhow!("no connection named '{name}'"))?;

        if let Some(ref docker_cfg) = cfg.docker {
            if !in_container {
                let args = docker::build_args(docker_cfg, original_argv)?;
                return Ok(ConnectPlan::Docker(args));
            }
        }

        Ok(ConnectPlan::Ssh(connect::build_args(conn)))
    }

    fn write_yaml(dir: &std::path::Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    fn no_color() -> Renderer {
        Renderer::new(false)
    }

    fn load(
        cwd: &std::path::Path,
        user: Option<&std::path::Path>,
        sys: &std::path::Path,
    ) -> config::LoadedConfig {
        config::load_impl(cwd, Some("connections"), false, user, sys).unwrap()
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    // ── Unknown name ──────────────────────────────────────────────────────────

    #[test]
    fn test_connect_unknown_name_returns_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());

        let err = plan(
            &cfg,
            "does-not-exist",
            &argv(&["yconn", "connect", "does-not-exist"]),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("does-not-exist"));
    }

    #[test]
    fn test_connect_error_message_contains_name() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());

        let err = plan(
            &cfg,
            "my-server",
            &argv(&["yconn", "connect", "my-server"]),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("my-server"));
    }

    // ── Non-Docker SSH path ───────────────────────────────────────────────────

    #[test]
    fn test_connect_no_docker_produces_ssh_plan() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  prod:\n    host: 10.0.0.1\n    user: deploy\n    auth: key\n    key: ~/.ssh/id_rsa\n    description: Prod\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        let p = plan(&cfg, "prod", &argv(&["yconn", "connect", "prod"]), false).unwrap();
        assert!(matches!(p, ConnectPlan::Ssh(_)));
        if let ConnectPlan::Ssh(args) = p {
            assert_eq!(args[0], "ssh");
            assert!(args.contains(&"-i".to_string()));
            assert!(args.contains(&"~/.ssh/id_rsa".to_string()));
            assert!(args.last().unwrap().contains("deploy@10.0.0.1"));
        }
    }

    #[test]
    fn test_connect_key_auth_default_port_ssh_args() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  srv:\n    host: myhost\n    user: admin\n    auth: key\n    key: ~/.ssh/id_ed25519\n    description: Server\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        let p = plan(&cfg, "srv", &argv(&["yconn", "connect", "srv"]), false).unwrap();
        if let ConnectPlan::Ssh(args) = p {
            assert_eq!(args, vec!["ssh", "-i", "~/.ssh/id_ed25519", "admin@myhost"]);
        }
    }

    #[test]
    fn test_connect_password_auth_ssh_args() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  db:\n    host: db.internal\n    user: dbadmin\n    auth: password\n    description: DB\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        let p = plan(&cfg, "db", &argv(&["yconn", "connect", "db"]), false).unwrap();
        if let ConnectPlan::Ssh(args) = p {
            assert_eq!(args, vec!["ssh", "dbadmin@db.internal"]);
        }
    }

    // ── Docker bootstrap path ─────────────────────────────────────────────────

    #[test]
    fn test_connect_docker_not_in_container_produces_docker_plan() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "docker:\n  image: ghcr.io/org/keys:latest\nconnections:\n  prod:\n    host: 10.0.0.1\n    user: deploy\n    auth: key\n    key: ~/.ssh/id_rsa\n    description: Prod\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        assert!(cfg.docker.is_some());

        // in_container = false → should bootstrap into Docker
        let p = plan(&cfg, "prod", &argv(&["yconn", "connect", "prod"]), false).unwrap();
        assert!(matches!(p, ConnectPlan::Docker(_)));
        if let ConnectPlan::Docker(args) = p {
            assert_eq!(args[0], "docker");
            assert_eq!(args[1], "run");
            assert!(args.contains(&"ghcr.io/org/keys:latest".to_string()));
            assert!(args.contains(&"CONN_IN_DOCKER=1".to_string()));
        }
    }

    #[test]
    fn test_connect_docker_in_container_produces_ssh_plan() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "docker:\n  image: ghcr.io/org/keys:latest\nconnections:\n  prod:\n    host: 10.0.0.1\n    user: deploy\n    auth: key\n    key: ~/.ssh/id_rsa\n    description: Prod\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        // in_container = true → Docker skipped, SSH invoked directly
        let p = plan(&cfg, "prod", &argv(&["yconn", "connect", "prod"]), true).unwrap();
        assert!(matches!(p, ConnectPlan::Ssh(_)));
    }

    #[test]
    fn test_connect_docker_argv_passed_through() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "docker:\n  image: myimage:v1\nconnections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    key: ~/.ssh/k\n    description: d\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());

        let orig = argv(&["yconn", "connect", "srv"]);
        let p = plan(&cfg, "srv", &orig, false).unwrap();
        if let ConnectPlan::Docker(args) = p {
            let img_pos = args.iter().position(|a| a == "myimage:v1").unwrap();
            assert_eq!(&args[img_pos + 1..], &["yconn", "connect", "srv"]);
        }
    }

    #[test]
    fn test_connect_no_docker_block_goes_ssh() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  srv:\n    host: h\n    user: u\n    auth: password\n    description: d\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        assert!(cfg.docker.is_none());
        let p = plan(&cfg, "srv", &argv(&["yconn", "connect", "srv"]), false).unwrap();
        assert!(matches!(p, ConnectPlan::Ssh(_)));
    }

    // ── run() error path (no exec involved) ───────────────────────────────────

    #[test]
    fn test_run_unknown_name_returns_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());

        let err = run(&cfg, &no_color(), "no-such-server", false).unwrap_err();
        assert!(err.to_string().contains("no-such-server"));
    }

    // ── expand_tilde ──────────────────────────────────────────────────────────

    #[test]
    fn test_expand_tilde_prefix_joins_home() {
        // "~/foo/bar" must resolve to <home>/foo/bar, not the literal string.
        let result = expand_tilde("~/foo/bar");
        let home = dirs::home_dir().expect("home dir must be set in test environment");
        assert_eq!(result, home.join("foo/bar"));
    }

    #[test]
    fn test_expand_tilde_bare_returns_home() {
        let result = expand_tilde("~");
        let home = dirs::home_dir().expect("home dir must be set in test environment");
        assert_eq!(result, home);
    }

    #[test]
    fn test_expand_tilde_absolute_path_unchanged() {
        let result = expand_tilde("/home/user/.ssh/id_rsa");
        assert_eq!(result, std::path::PathBuf::from("/home/user/.ssh/id_rsa"));
    }

    #[test]
    fn test_expand_tilde_no_tilde_unchanged() {
        let result = expand_tilde("relative/path/key");
        assert_eq!(result, std::path::PathBuf::from("relative/path/key"));
    }

    /// A tilde key path that resolves to an existing file should produce no
    /// "does not exist" warning from `security::check_key_file`.
    #[test]
    fn test_tilde_key_exists_no_warning() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let key_path = dir.path().join("id_rsa");
        fs::write(&key_path, "KEY").unwrap();
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();

        let expanded = key_path.clone();
        let warnings = security::check_key_file(&expanded);
        // File exists and has safe permissions — no warnings.
        assert!(
            warnings.is_empty(),
            "unexpected warnings for existing key: {:?}",
            warnings
        );
    }

    /// A tilde key path whose resolved path does not exist must still emit the
    /// "key file does not exist" warning.
    #[test]
    fn test_tilde_key_missing_warns() {
        let dir = TempDir::new().unwrap();
        // Construct a path that does not exist.
        let missing = dir.path().join("no_such_key");

        let warnings = security::check_key_file(&missing);
        assert_eq!(warnings.len(), 1, "expected exactly one warning");
        assert!(
            warnings[0].message.contains("does not exist"),
            "warning message should say 'does not exist': {}",
            warnings[0].message
        );
    }

    /// Verify the full expand_tilde → check_key_file pipeline: a path written
    /// as "~/..." in config must not trigger a false "does not exist" warning
    /// when the file is actually present under the real home directory.
    #[test]
    fn test_expand_tilde_then_check_existing_key_no_warning() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let key_path = dir.path().join("id_ed25519");
        fs::write(&key_path, "KEY DATA").unwrap();
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();

        // Simulate expand_tilde on an already-absolute path (as produced by
        // a non-tilde config entry) to verify the pipeline handles it correctly.
        let expanded = expand_tilde(key_path.to_str().unwrap());
        assert_eq!(expanded, key_path);

        let warnings = security::check_key_file(&expanded);
        assert!(
            warnings.is_empty(),
            "absolute path to existing key must not warn: {:?}",
            warnings
        );
    }
}
