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
    pub link: Option<String>,
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
    /// The locked group name, or `None` when no group is locked.
    pub group: Option<String>,
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
    /// The locked group name, or `None` when no group is locked.
    pub active_group: Option<String>,
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

    /// Truncate a URL to at most `max` characters, appending `…` if truncated.
    fn truncate_link(url: &str, max: usize) -> String {
        if url.len() <= max {
            url.to_string()
        } else {
            // Truncate to max-1 chars and append the ellipsis character.
            let truncated: String = url.chars().take(max - 1).collect();
            format!("{truncated}\u{2026}")
        }
    }

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
        const LINK_HEADER: &str = "LINK";
        const LINK_MAX: usize = 50;
        const GAP: &str = "   ";

        // Determine whether any row has a link; omit the column entirely if not.
        let show_link = rows.iter().any(|r| r.link.is_some());

        // Compute column widths (description is last non-link column; not padded).
        let mut col = [
            HEADERS[0].len(),
            HEADERS[1].len(),
            HEADERS[2].len(),
            HEADERS[3].len(),
            HEADERS[4].len(),
            HEADERS[5].len(),
            0usize, // description — not padded
            0usize, // link — not padded (it is the last column)
        ];
        for row in rows {
            col[0] = col[0].max(row.name.len());
            col[1] = col[1].max(row.host.len());
            col[2] = col[2].max(row.user.len());
            col[3] = col[3].max(row.port.to_string().len());
            col[4] = col[4].max(row.auth.len());
            col[5] = col[5].max(row.source.len());
        }

        // Build header row.
        let mut header_cells: Vec<String> = vec![
            pad(HEADERS[0], col[0]),
            pad(HEADERS[1], col[1]),
            pad(HEADERS[2], col[2]),
            pad(HEADERS[3], col[3]),
            pad(HEADERS[4], col[4]),
            pad(HEADERS[5], col[5]),
            HEADERS[6].to_string(),
        ];
        if show_link {
            header_cells.push(LINK_HEADER.to_string());
        }
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
            let mut cells: Vec<String> = vec![
                pad(&row.name, col[0]),
                pad(&row.host, col[1]),
                pad(&row.user, col[2]),
                pad(&row.port.to_string(), col[3]),
                pad(&row.auth, col[4]),
                pad(&row.source, col[5]),
                desc,
            ];
            if show_link {
                let link_cell = row
                    .link
                    .as_deref()
                    .map(|u| Self::truncate_link(u, LINK_MAX))
                    .unwrap_or_default();
                cells.push(link_cell);
            }
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

        match &status.group {
            Some(g) if status.group_from_file => {
                out.push_str(&format!(
                    "Group:   {}  (set in ~/.config/yconn/session.yml)\n",
                    g
                ));
            }
            Some(g) => {
                out.push_str(&format!("Group:   {}  (default)\n", g));
            }
            None => {
                out.push_str("Group:   (none — showing all connections)\n");
            }
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

        let group_display = status
            .active_group
            .as_deref()
            .unwrap_or("(none — showing all connections)");
        out.push_str(&format!("Active group: {}\n", group_display));
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

    /// Print the full `ssh` command to stderr (for `--verbose`).
    pub fn verbose_ssh_cmd(&self, args: &[String]) {
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
                link: None,
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
                link: None,
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
            link: None,
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

    #[test]
    fn test_list_link_column_present_when_any_row_has_link() {
        let mut rows = sample_rows();
        rows[0].link = Some("https://wiki.internal/servers/prod-web".into());
        let out = r().render_list(&rows);
        assert!(
            out.contains("LINK"),
            "expected LINK header when a row has a link"
        );
        assert!(
            out.contains("https://wiki.internal/servers/prod-web"),
            "expected link URL in output"
        );
    }

    #[test]
    fn test_list_link_column_absent_when_no_rows_have_link() {
        let out = r().render_list(&sample_rows());
        assert!(
            !out.contains("LINK"),
            "expected no LINK header when no row has a link"
        );
    }

    #[test]
    fn test_list_link_truncated_when_too_long() {
        let mut rows = sample_rows();
        // 60-character URL — exceeds the 50-char limit.
        let long_url = "https://wiki.internal/servers/this-is-a-very-long-path/extra";
        rows[0].link = Some(long_url.into());
        let out = r().render_list(&rows);
        // The ellipsis character must appear (truncation happened).
        assert!(
            out.contains('\u{2026}'),
            "expected ellipsis for truncated URL in output: {out}"
        );
        // The full URL must NOT appear verbatim.
        assert!(
            !out.contains(long_url),
            "expected long URL to be truncated, but found it verbatim: {out}"
        );
    }

    #[test]
    fn test_list_link_not_truncated_when_within_limit() {
        let mut rows = sample_rows();
        let short_url = "https://wiki.internal/srv";
        rows[0].link = Some(short_url.into());
        let out = r().render_list(&rows);
        assert!(out.contains(short_url), "short URL should appear verbatim");
        assert!(
            !out.contains('\u{2026}'),
            "no ellipsis expected for short URL"
        );
    }

    #[test]
    fn test_list_link_column_shown_for_shadowed_rows() {
        let mut rows = sample_rows();
        rows.push(ConnectionRow {
            name: "bastion".into(),
            host: "bastion.example.com".into(),
            user: "ec2-user".into(),
            port: 2222,
            auth: "key".into(),
            source: "system".into(),
            description: "Bastion host".into(),
            link: Some("https://wiki.internal/bastion".into()),
            shadowed: true,
        });
        let out = r().render_list(&rows);
        assert!(
            out.contains("LINK"),
            "expected LINK column when shadowed row has a link"
        );
        assert!(
            out.contains("https://wiki.internal/bastion"),
            "expected shadowed row's link in output"
        );
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
            group: Some("connections".to_string()),
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
            group: Some("work".to_string()),
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
            active_group: Some("work".to_string()),
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

    // ── verbose_ssh_cmd tests ─────────────────────────────────────────────────

    #[test]
    fn test_verbose_ssh_cmd_formats_args() {
        // Just test that the function doesn't panic; output goes to stderr.
        let renderer = Renderer::new(false);
        renderer.verbose_ssh_cmd(&[
            "ssh".into(),
            "-i".into(),
            "~/.ssh/id_rsa".into(),
            "deploy@myhost".into(),
        ]);
    }

    #[test]
    fn test_verbose_ssh_cmd_empty() {
        Renderer::new(false).verbose_ssh_cmd(&[]);
    }
}
