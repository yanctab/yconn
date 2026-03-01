//! All user-facing output lives here. No other module writes to stdout or
//! stderr directly — all output is routed through a [`Renderer`].

// Types and the Renderer are used by CLI command modules that are not yet
// implemented. Suppress dead_code until those modules are added.
#![allow(dead_code)]

// ─── Input types ─────────────────────────────────────────────────────────────

/// A row in the `yconn list` output table.
pub struct ConnectionRow {
    pub name: String,
    pub host: String,
    pub user: String,
    pub port: u16,
    pub auth: String,
    pub source: String,
    pub description: String,
    pub shadowed: bool,
}

/// Full detail for `yconn show <name>`.
pub struct ConnectionDetail {
    pub name: String,
    pub host: String,
    pub user: String,
    pub port: u16,
    pub auth: String,
    pub key: Option<String>,
    pub description: String,
    pub link: Option<String>,
    pub source_label: String,
    pub source_path: String,
}

/// Status of a single config layer, used by `yconn config`.
pub struct LayerInfo {
    pub label: String,
    pub path: String,
    /// `None` means the file was not found; `Some(n)` is the connection count.
    pub connection_count: Option<usize>,
}

/// Docker block status, used by `yconn config`.
pub struct DockerInfo {
    pub image: String,
    pub pull: String,
    pub source: String,
    pub will_bootstrap: bool,
}

/// Full status for `yconn config`.
pub struct ConfigStatus {
    pub group: String,
    /// `true` = read from `session.yml`; `false` = using the default.
    pub group_from_file: bool,
    pub layers: Vec<LayerInfo>,
    pub docker: Option<DockerInfo>,
}

/// A row in the `yconn group list` output table.
pub struct GroupRow {
    pub name: String,
    pub layers: Vec<String>,
}

/// A single layer entry for `yconn group current`.
pub struct LayerCurrentInfo {
    pub label: String,
    pub path: String,
    pub found: bool,
}

/// Full status for `yconn group current`.
pub struct GroupCurrentStatus {
    pub active_group: String,
    pub session_file: String,
    pub layers: Vec<LayerCurrentInfo>,
}

// ─── ANSI helpers ────────────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

/// Left-pad `s` with spaces to `width` characters (no-op if already wider).
fn pad(s: &str, width: usize) -> String {
    if s.len() >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - s.len()))
    }
}

// ─── Renderer ────────────────────────────────────────────────────────────────

/// Handles all user-facing output. No other module should write to stdout or
/// stderr directly.
pub struct Renderer {
    color: bool,
}

impl Renderer {
    /// Create a renderer. Pass `color: false` when `--no-color` is set or
    /// when stdout is not a TTY.
    pub fn new(color: bool) -> Self {
        Renderer { color }
    }

    fn maybe_bold(&self, s: &str) -> String {
        if self.color {
            format!("{BOLD}{s}{RESET}")
        } else {
            s.to_string()
        }
    }

    fn maybe_dim(&self, s: &str) -> String {
        if self.color {
            format!("{DIM}{s}{RESET}")
        } else {
            s.to_string()
        }
    }

    // ── list ─────────────────────────────────────────────────────────────────

    fn render_list(&self, rows: &[ConnectionRow]) -> String {
        const HEADERS: [&str; 7] = [
            "NAME",
            "HOST",
            "USER",
            "PORT",
            "AUTH",
            "SOURCE",
            "DESCRIPTION",
        ];
        const GAP: &str = "   ";

        // Compute column widths (description is last; it is not padded).
        let mut col = [
            HEADERS[0].len(),
            HEADERS[1].len(),
            HEADERS[2].len(),
            HEADERS[3].len(),
            HEADERS[4].len(),
            HEADERS[5].len(),
            0usize,
        ];
        for row in rows {
            col[0] = col[0].max(row.name.len());
            col[1] = col[1].max(row.host.len());
            col[2] = col[2].max(row.user.len());
            col[3] = col[3].max(row.port.to_string().len());
            col[4] = col[4].max(row.auth.len());
            col[5] = col[5].max(row.source.len());
        }

        let header_cells: Vec<String> = vec![
            pad(HEADERS[0], col[0]),
            pad(HEADERS[1], col[1]),
            pad(HEADERS[2], col[2]),
            pad(HEADERS[3], col[3]),
            pad(HEADERS[4], col[4]),
            pad(HEADERS[5], col[5]),
            HEADERS[6].to_string(),
        ];
        let header_plain = header_cells.join(GAP);
        let separator: String = "─".repeat(header_plain.len());

        let mut out = String::new();
        out.push_str(&self.maybe_bold(&header_plain));
        out.push('\n');
        out.push_str(&separator);
        out.push('\n');

        for row in rows {
            let desc = if row.shadowed {
                format!("{} [shadowed]", row.description)
            } else {
                row.description.clone()
            };
            let cells: Vec<String> = vec![
                pad(&row.name, col[0]),
                pad(&row.host, col[1]),
                pad(&row.user, col[2]),
                pad(&row.port.to_string(), col[3]),
                pad(&row.auth, col[4]),
                pad(&row.source, col[5]),
                desc,
            ];
            let line = cells.join(GAP);
            if row.shadowed {
                out.push_str(&self.maybe_dim(&line));
            } else {
                out.push_str(&line);
            }
            out.push('\n');
        }

        out
    }

    // ── show ─────────────────────────────────────────────────────────────────

    fn render_show(&self, detail: &ConnectionDetail) -> String {
        // "Description:" is 12 chars — the longest label.
        const LW: usize = 12;
        let mut out = String::new();
        out.push_str(&format!("Connection: {}\n", detail.name));
        out.push_str(&format!("  {}  {}\n", pad("Host:", LW), detail.host));
        out.push_str(&format!("  {}  {}\n", pad("User:", LW), detail.user));
        out.push_str(&format!("  {}  {}\n", pad("Port:", LW), detail.port));
        out.push_str(&format!("  {}  {}\n", pad("Auth:", LW), detail.auth));
        if let Some(key) = &detail.key {
            out.push_str(&format!("  {}  {}\n", pad("Key:", LW), key));
        }
        out.push_str(&format!(
            "  {}  {}\n",
            pad("Description:", LW),
            detail.description
        ));
        if let Some(link) = &detail.link {
            out.push_str(&format!("  {}  {}\n", pad("Link:", LW), link));
        }
        out.push_str(&format!(
            "  {}  {} ({})\n",
            pad("Source:", LW),
            detail.source_label,
            detail.source_path
        ));
        out
    }

    // ── config ───────────────────────────────────────────────────────────────

    fn render_config_status(&self, status: &ConfigStatus) -> String {
        // "[project]" is 9 chars — the widest layer label.
        const LABEL_W: usize = 9;
        let mut out = String::new();

        if status.group_from_file {
            out.push_str(&format!(
                "Group:   {}  (set in ~/.config/yconn/session.yml)\n",
                status.group
            ));
        } else {
            out.push_str(&format!("Group:   {}  (default)\n", status.group));
        }
        out.push('\n');

        out.push_str("Active config files (highest to lowest priority):\n");
        for layer in &status.layers {
            let label = format!("[{}]", layer.label);
            match layer.connection_count {
                Some(n) => out.push_str(&format!(
                    "  {}  {}    ({} connections)\n",
                    pad(&label, LABEL_W),
                    layer.path,
                    n
                )),
                None => out.push_str(&format!(
                    "  {}  {}    (not found)\n",
                    pad(&label, LABEL_W),
                    layer.path
                )),
            }
        }

        if let Some(docker) = &status.docker {
            out.push('\n');
            out.push_str("Docker:\n");
            out.push_str(&format!("  Image:   {}\n", docker.image));
            out.push_str(&format!("  Pull:    {}\n", docker.pull));
            out.push_str(&format!("  Source:  {}\n", docker.source));
            let status_line = if docker.will_bootstrap {
                "will bootstrap into container on connect"
            } else {
                "already inside container"
            };
            out.push_str(&format!("  Status:  {status_line}\n"));
        }

        out
    }

    // ── group list ────────────────────────────────────────────────────────────

    fn render_group_list(&self, groups: &[GroupRow]) -> String {
        const HEADER_NAME: &str = "GROUP";
        const HEADER_LAYERS: &str = "LAYERS";
        const GAP: &str = "   ";

        let name_w = groups
            .iter()
            .map(|g| g.name.len())
            .max()
            .unwrap_or(0)
            .max(HEADER_NAME.len());

        let layers_w = groups
            .iter()
            .map(|g| g.layers.join(", ").len())
            .max()
            .unwrap_or(0)
            .max(HEADER_LAYERS.len());

        let header_plain = format!("{}{}{}", pad(HEADER_NAME, name_w), GAP, HEADER_LAYERS);
        let separator: String = "─".repeat(name_w + GAP.len() + layers_w);

        let mut out = String::new();
        out.push_str(&self.maybe_bold(&header_plain));
        out.push('\n');
        out.push_str(&separator);
        out.push('\n');

        for group in groups {
            out.push_str(&format!(
                "{}{}{}\n",
                pad(&group.name, name_w),
                GAP,
                group.layers.join(", ")
            ));
        }

        out
    }

    // ── group current ─────────────────────────────────────────────────────────

    fn render_group_current(&self, status: &GroupCurrentStatus) -> String {
        const LABEL_W: usize = 9; // "[project]" is 9 chars
        let mut out = String::new();

        out.push_str(&format!("Active group: {}\n", status.active_group));
        out.push_str(&format!("Lock file:    {}\n", status.session_file));
        out.push('\n');
        out.push_str("Resolved config files:\n");

        for layer in &status.layers {
            let label = format!("[{}]", layer.label);
            if layer.found {
                out.push_str(&format!(
                    "  {}  {}    \u{2713} found\n",
                    pad(&label, LABEL_W),
                    layer.path
                ));
            } else {
                out.push_str(&format!(
                    "  {}  {}    \u{2717} not found\n",
                    pad(&label, LABEL_W),
                    layer.path
                ));
            }
        }

        out
    }

    // ── public API ────────────────────────────────────────────────────────────

    /// Print the connection list table (`yconn list`).
    pub fn list(&self, rows: &[ConnectionRow]) {
        print!("{}", self.render_list(rows));
    }

    /// Print a connection detail block (`yconn show <name>`).
    pub fn show(&self, detail: &ConnectionDetail) {
        print!("{}", self.render_show(detail));
    }

    /// Print the config status block (`yconn config`).
    pub fn config_status(&self, status: &ConfigStatus) {
        print!("{}", self.render_config_status(status));
    }

    /// Print the group list table (`yconn group list`).
    pub fn group_list(&self, groups: &[GroupRow]) {
        print!("{}", self.render_group_list(groups));
    }

    /// Print the group current status (`yconn group current`).
    pub fn group_current(&self, status: &GroupCurrentStatus) {
        print!("{}", self.render_group_current(status));
    }

    /// Print a `[yconn] …` verbose log line to stderr.
    pub fn verbose(&self, msg: &str) {
        eprintln!("[yconn] {msg}");
    }

    /// Print the full `docker run` command to stderr (for `--verbose`).
    pub fn verbose_docker_cmd(&self, args: &[String]) {
        if let Some((first, rest)) = args.split_first() {
            let mut line = format!("[yconn] Running: {}", first);
            for arg in rest {
                line.push_str(&format!(" \\\n         {}", arg));
            }
            eprintln!("{line}");
        }
    }

    /// Print a non-blocking warning to stderr.
    pub fn warn(&self, msg: &str) {
        eprintln!("warning: {msg}");
    }

    /// Print an error message to stderr.
    pub fn error(&self, msg: &str) {
        eprintln!("error: {msg}");
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Renderer {
        Renderer::new(false)
    }

    fn sample_rows() -> Vec<ConnectionRow> {
        vec![
            ConnectionRow {
                name: "prod-web".into(),
                host: "10.0.1.50".into(),
                user: "deploy".into(),
                port: 22,
                auth: "key".into(),
                source: "project".into(),
                description: "Primary production web server".into(),
                shadowed: false,
            },
            ConnectionRow {
                name: "staging-db".into(),
                host: "staging.internal".into(),
                user: "dbadmin".into(),
                port: 22,
                auth: "password".into(),
                source: "user".into(),
                description: "Staging database server".into(),
                shadowed: false,
            },
        ]
    }

    // ── list tests ────────────────────────────────────────────────────────────

    #[test]
    fn test_list_header_and_separator() {
        let out = r().render_list(&sample_rows());
        assert!(out.contains("NAME"));
        assert!(out.contains("HOST"));
        assert!(out.contains("USER"));
        assert!(out.contains("PORT"));
        assert!(out.contains("AUTH"));
        assert!(out.contains("SOURCE"));
        assert!(out.contains("DESCRIPTION"));
        assert!(out.contains('─'));
    }

    #[test]
    fn test_list_connection_data() {
        let out = r().render_list(&sample_rows());
        assert!(out.contains("prod-web"));
        assert!(out.contains("10.0.1.50"));
        assert!(out.contains("deploy"));
        assert!(out.contains("Primary production web server"));
        assert!(out.contains("staging-db"));
        assert!(out.contains("password"));
    }

    #[test]
    fn test_list_empty() {
        let out = r().render_list(&[]);
        assert!(out.contains("NAME"));
        assert!(!out.contains("prod-web"));
    }

    #[test]
    fn test_list_shadowed_row_tagged() {
        let mut rows = sample_rows();
        rows.push(ConnectionRow {
            name: "bastion".into(),
            host: "bastion.example.com".into(),
            user: "ec2-user".into(),
            port: 2222,
            auth: "key".into(),
            source: "system".into(),
            description: "Bastion host".into(),
            shadowed: true,
        });
        let out = r().render_list(&rows);
        assert!(out.contains("[shadowed]"));
        assert!(out.contains("bastion"));
    }

    #[test]
    fn test_list_non_shadowed_row_not_tagged() {
        let out = r().render_list(&sample_rows());
        assert!(!out.contains("[shadowed]"));
    }

    // ── show tests ────────────────────────────────────────────────────────────

    fn full_detail() -> ConnectionDetail {
        ConnectionDetail {
            name: "prod-web".into(),
            host: "10.0.1.50".into(),
            user: "deploy".into(),
            port: 22,
            auth: "key".into(),
            key: Some("~/.ssh/prod_deploy_key".into()),
            description: "Primary production web server".into(),
            link: Some("https://wiki.internal/servers/prod-web".into()),
            source_label: "project".into(),
            source_path: "/home/user/projects/acme/.yconn/connections.yaml".into(),
        }
    }

    #[test]
    fn test_show_key_auth_all_fields() {
        let out = r().render_show(&full_detail());
        assert!(out.contains("Connection: prod-web"));
        assert!(out.contains("10.0.1.50"));
        assert!(out.contains("deploy"));
        assert!(out.contains("22"));
        assert!(out.contains("key"));
        assert!(out.contains("~/.ssh/prod_deploy_key"));
        assert!(out.contains("Primary production web server"));
        assert!(out.contains("https://wiki.internal/servers/prod-web"));
        assert!(out.contains("project"));
        assert!(out.contains("/home/user/projects/acme/.yconn/connections.yaml"));
    }

    #[test]
    fn test_show_password_auth_no_key_no_link() {
        let detail = ConnectionDetail {
            name: "staging-db".into(),
            host: "staging.internal".into(),
            user: "dbadmin".into(),
            port: 22,
            auth: "password".into(),
            key: None,
            description: "Staging DB".into(),
            link: None,
            source_label: "user".into(),
            source_path: "/home/user/.config/yconn/connections.yaml".into(),
        };
        let out = r().render_show(&detail);
        assert!(!out.contains("Key:"));
        assert!(!out.contains("Link:"));
        assert!(out.contains("password"));
    }

    // ── config tests ──────────────────────────────────────────────────────────

    #[test]
    fn test_config_status_default_group_no_docker() {
        let status = ConfigStatus {
            group: "connections".into(),
            group_from_file: false,
            layers: vec![
                LayerInfo {
                    label: "project".into(),
                    path: "/repo/.yconn/connections.yaml".into(),
                    connection_count: Some(2),
                },
                LayerInfo {
                    label: "user".into(),
                    path: "/home/user/.config/yconn/connections.yaml".into(),
                    connection_count: None,
                },
                LayerInfo {
                    label: "system".into(),
                    path: "/etc/yconn/connections.yaml".into(),
                    connection_count: None,
                },
            ],
            docker: None,
        };
        let out = r().render_config_status(&status);
        assert!(out.contains("connections"));
        assert!(out.contains("default"));
        assert!(out.contains("[project]"));
        assert!(out.contains("2 connections"));
        assert!(out.contains("not found"));
        assert!(!out.contains("Docker:"));
    }

    #[test]
    fn test_config_status_from_file_with_docker() {
        let status = ConfigStatus {
            group: "work".into(),
            group_from_file: true,
            layers: vec![LayerInfo {
                label: "project".into(),
                path: "/repo/.yconn/work.yaml".into(),
                connection_count: Some(4),
            }],
            docker: Some(DockerInfo {
                image: "ghcr.io/myorg/yconn-keys:latest".into(),
                pull: "missing".into(),
                source: "project".into(),
                will_bootstrap: true,
            }),
        };
        let out = r().render_config_status(&status);
        assert!(out.contains("set in ~/.config/yconn/session.yml"));
        assert!(out.contains("Docker:"));
        assert!(out.contains("ghcr.io/myorg/yconn-keys:latest"));
        assert!(out.contains("will bootstrap"));
    }

    // ── group list tests ──────────────────────────────────────────────────────

    #[test]
    fn test_group_list_header_and_data() {
        let groups = vec![
            GroupRow {
                name: "connections".into(),
                layers: vec!["project".into(), "user".into(), "system".into()],
            },
            GroupRow {
                name: "work".into(),
                layers: vec!["project".into(), "user".into()],
            },
            GroupRow {
                name: "private".into(),
                layers: vec!["user".into()],
            },
        ];
        let out = r().render_group_list(&groups);
        assert!(out.contains("GROUP"));
        assert!(out.contains("LAYERS"));
        assert!(out.contains('─'));
        assert!(out.contains("connections"));
        assert!(out.contains("project, user, system"));
        assert!(out.contains("work"));
        assert!(out.contains("private"));
        assert!(out.contains("user"));
    }

    #[test]
    fn test_group_list_empty() {
        let out = r().render_group_list(&[]);
        assert!(out.contains("GROUP"));
        assert!(!out.contains("connections"));
    }

    // ── group current tests ───────────────────────────────────────────────────

    #[test]
    fn test_group_current_found_and_not_found() {
        let status = GroupCurrentStatus {
            active_group: "work".into(),
            session_file: "~/.config/yconn/session.yml".into(),
            layers: vec![
                LayerCurrentInfo {
                    label: "project".into(),
                    path: "/repo/.yconn/work.yaml".into(),
                    found: true,
                },
                LayerCurrentInfo {
                    label: "user".into(),
                    path: "/home/user/.config/yconn/work.yaml".into(),
                    found: true,
                },
                LayerCurrentInfo {
                    label: "system".into(),
                    path: "/etc/yconn/work.yaml".into(),
                    found: false,
                },
            ],
        };
        let out = r().render_group_current(&status);
        assert!(out.contains("Active group: work"));
        assert!(out.contains("Lock file:"));
        assert!(out.contains("~/.config/yconn/session.yml"));
        assert!(out.contains("Resolved config files:"));
        assert!(out.contains("\u{2713} found"));
        assert!(out.contains("\u{2717} not found"));
        assert!(out.contains("[project]"));
        assert!(out.contains("[system]"));
    }

    // ── verbose / warn / error tests ──────────────────────────────────────────

    #[test]
    fn test_verbose_docker_cmd_formats_args() {
        // Just test that the function doesn't panic; output goes to stderr.
        let renderer = Renderer::new(false);
        renderer.verbose_docker_cmd(&[
            "docker".into(),
            "run".into(),
            "--rm".into(),
            "myimage".into(),
        ]);
    }

    #[test]
    fn test_verbose_docker_cmd_empty() {
        Renderer::new(false).verbose_docker_cmd(&[]);
    }
}
