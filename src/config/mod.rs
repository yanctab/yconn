//! Config layer loading, upward walk, and merge logic.
//!
//! Loads `connections.yaml` from three layers in priority order (project >
//! user > system), merges into a flat connection map with source tracking,
//! and retains shadowed entries for `--all` display. Surfaces the resolved
//! `docker` block when present.
//!
//! Groups are now inline: each connection may carry an optional `group` field.
//! The active group (stored in `session.yml`) acts as a filter on the merged
//! connection list, not as a file-name selector.

// Public API is consumed by CLI command modules not yet implemented.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::security;

// ─── Wire types (serde) ───────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct RawFile {
    #[serde(default)]
    docker: Option<RawDocker>,
    #[serde(default)]
    connections: HashMap<String, RawConn>,
}

#[derive(Deserialize, Clone, Debug)]
struct RawDocker {
    image: String,
    #[serde(default = "default_pull")]
    pull: String,
    #[serde(default)]
    args: Vec<String>,
}

fn default_pull() -> String {
    "missing".to_string()
}

#[derive(Deserialize, Clone, Debug)]
struct RawConn {
    host: String,
    user: String,
    #[serde(default = "default_port")]
    port: u16,
    auth: String,
    #[serde(default)]
    key: Option<String>,
    description: String,
    #[serde(default)]
    link: Option<String>,
    /// Optional inline group tag. Connections without a `group:` field belong
    /// to no group and are always shown unless a group filter is active.
    #[serde(default)]
    group: Option<String>,
}

fn default_port() -> u16 {
    22
}

// ─── Public types ─────────────────────────────────────────────────────────────

/// Which config layer a connection or docker block was loaded from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    Project,
    User,
    System,
}

impl Layer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::System => "system",
        }
    }
}

/// A resolved SSH connection entry.
#[derive(Clone, Debug)]
pub struct Connection {
    pub name: String,
    pub host: String,
    pub user: String,
    pub port: u16,
    pub auth: String,
    pub key: Option<String>,
    pub description: String,
    pub link: Option<String>,
    /// Optional inline group tag from the YAML `group:` field.
    pub group: Option<String>,
    /// The layer this entry was loaded from.
    pub layer: Layer,
    /// Path to the config file that defines this entry.
    pub source_path: PathBuf,
    /// `true` if a higher-priority layer defines a connection with the same name.
    pub shadowed: bool,
}

/// The resolved `docker` block from the highest-priority non-user layer.
#[derive(Clone, Debug)]
pub struct DockerConfig {
    pub image: String,
    pub pull: String,
    pub args: Vec<String>,
    pub layer: Layer,
    pub source_path: PathBuf,
}

/// Status of one config layer (used by `yconn config` output).
#[derive(Debug)]
pub struct LayerStatus {
    pub layer: Layer,
    /// Path that was (or would be) loaded for this layer.
    pub path: PathBuf,
    /// `None` = file not found; `Some(n)` = number of connections in the file.
    pub connection_count: Option<usize>,
}

/// Everything a CLI command needs after loading and merging all config layers.
#[derive(Debug)]
pub struct LoadedConfig {
    /// Active connections — one per name, highest-priority layer wins.
    /// This always contains the full unfiltered set; callers apply group
    /// filtering via the helper methods.
    pub connections: Vec<Connection>,
    /// Active + shadowed connections interleaved for `--all` display.
    /// Shadowed entries appear immediately after their active counterpart.
    pub all_connections: Vec<Connection>,
    /// Resolved docker config (project or system only; user layer ignored).
    pub docker: Option<DockerConfig>,
    /// Status of each layer in priority order [project, user, system].
    pub layers: [LayerStatus; 3],
    /// The `.yconn/` directory found by the upward walk (for group discovery).
    pub project_dir: Option<PathBuf>,
    /// Security warnings collected during loading.
    pub warnings: Vec<security::Warning>,
    /// The active group name (locked group from session.yml), if any.
    /// `None` means no lock — show all connections by default.
    pub group: Option<String>,
    /// `true` = group was read from `session.yml`; `false` = using the default.
    pub group_from_file: bool,
}

impl LoadedConfig {
    /// Find a connection by name (active connections only).
    pub fn find(&self, name: &str) -> Option<&Connection> {
        self.connections.iter().find(|c| c.name == name)
    }

    /// Return the connections that should be shown by default.
    ///
    /// - If `group_filter` is `Some(name)`, return only connections whose
    ///   `group` field equals `name`.
    /// - If `group_filter` is `None`, return all active connections.
    ///
    /// The `group_filter` may come from `--group <name>` (CLI flag, highest
    /// priority) or from the locked group in `session.yml`.
    pub fn filtered_connections(&self, group_filter: Option<&str>) -> Vec<&Connection> {
        match group_filter {
            Some(g) => self
                .connections
                .iter()
                .filter(|c| c.group.as_deref() == Some(g))
                .collect(),
            None => self.connections.iter().collect(),
        }
    }

    /// Determine the effective group filter to apply for `yconn list`.
    ///
    /// Precedence (highest to lowest):
    ///   1. `--all` flag → no filter (returns `None`)
    ///   2. `--group <name>` CLI flag → `Some(name)`
    ///   3. Locked group from `session.yml` → `Some(name)`
    ///   4. Default → no filter (`None`)
    pub fn effective_group_filter<'a>(
        &'a self,
        all_flag: bool,
        group_flag: Option<&'a str>,
    ) -> Option<&'a str> {
        if all_flag {
            return None;
        }
        if let Some(g) = group_flag {
            return Some(g);
        }
        self.group.as_deref()
    }

    /// Return unique group values present across all active connections.
    /// Used by `yconn group list`.
    pub fn discover_groups(&self) -> Vec<crate::group::GroupEntry> {
        use std::collections::BTreeMap;
        // BTreeMap keeps groups sorted by name for stable output.
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for conn in &self.connections {
            if let Some(ref g) = conn.group {
                let layer_label = conn.layer.label().to_string();
                map.entry(g.clone()).or_default().push(layer_label.clone());
            }
        }

        // Deduplicate layer labels while preserving insertion order.
        map.into_iter()
            .map(|(name, raw_layers)| {
                let mut seen = std::collections::HashSet::new();
                let layers: Vec<String> = raw_layers
                    .into_iter()
                    .filter(|l| seen.insert(l.clone()))
                    .collect();
                crate::group::GroupEntry { name, layers }
            })
            .collect()
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Load config using the current working directory and standard layer paths.
pub fn load() -> Result<LoadedConfig> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    load_from(&cwd)
}

/// Load config from `cwd`, using standard user and system layer paths.
///
/// This is the main entry point; `load()` delegates here with the real CWD.
/// Tests pass explicit `cwd` values to control the upward walk.
pub fn load_from(cwd: &Path) -> Result<LoadedConfig> {
    let ag = crate::group::active_group()?;
    let user_dir = dirs::config_dir().map(|d| d.join("yconn"));
    let system_dir = PathBuf::from("/etc/yconn");
    load_impl(
        cwd,
        ag.name.as_deref(),
        ag.from_file,
        user_dir.as_deref(),
        &system_dir,
    )
}

// ─── Internal implementation ──────────────────────────────────────────────────

/// One element of the three-layer raw-connection array: (connections, layer, source path).
type RawLayer = (Vec<(String, RawConn)>, Layer, PathBuf);

/// Core load logic with all paths explicit — used directly by tests and
/// command-layer integration tests.
///
/// `group` is now `Option<&str>`: `None` means no lock (show all by default),
/// `Some(name)` means a group is locked in session.yml.
pub(crate) fn load_impl(
    cwd: &Path,
    group: Option<&str>,
    group_from_file: bool,
    user_dir: Option<&Path>,
    system_dir: &Path,
) -> Result<LoadedConfig> {
    let mut warnings: Vec<security::Warning> = Vec::new();

    // ── Resolve paths ────────────────────────────────────────────────────────
    // Always load from `connections.yaml` — groups are inline fields now.
    let (project_dir, project_file) = upward_walk(cwd);
    let user_file = user_dir.map(|d| d.join("connections.yaml"));
    let system_file = system_dir.join("connections.yaml");

    // ── Load each layer ──────────────────────────────────────────────────────
    let proj = load_layer(project_file.as_deref(), Layer::Project, true, &mut warnings)?;
    let user = load_layer(user_file.as_deref(), Layer::User, false, &mut warnings)?;
    let sys = load_layer(Some(&system_file), Layer::System, false, &mut warnings)?;

    // ── Merge connections ────────────────────────────────────────────────────
    let raw_layers: [RawLayer; 3] = [
        (
            proj.connections,
            Layer::Project,
            project_file
                .clone()
                .unwrap_or_else(|| PathBuf::from(".yconn")),
        ),
        (
            user.connections,
            Layer::User,
            user_file
                .clone()
                .unwrap_or_else(|| PathBuf::from("~/.config/yconn")),
        ),
        (sys.connections, Layer::System, system_file.clone()),
    ];
    let (connections, all_connections) = merge_connections(&raw_layers);

    // ── Resolve docker (project > system; user ignored with warning) ─────────
    if user.docker_present {
        let path = user_file.as_deref().unwrap_or(Path::new("~/.config/yconn"));
        warnings.push(security::check_docker_in_user_layer(path));
    }
    let docker = proj
        .docker
        .map(|d| {
            docker_config(
                d,
                Layer::Project,
                project_file.as_deref().unwrap_or(Path::new(".yconn")),
            )
        })
        .or_else(|| {
            sys.docker
                .map(|d| docker_config(d, Layer::System, &system_file))
        });

    // ── Layer status ─────────────────────────────────────────────────────────
    let layers = [
        LayerStatus {
            layer: Layer::Project,
            path: project_file
                .clone()
                .unwrap_or_else(|| PathBuf::from(".yconn/connections.yaml")),
            connection_count: proj.found.then_some(proj.count),
        },
        LayerStatus {
            layer: Layer::User,
            path: user_file
                .clone()
                .unwrap_or_else(|| PathBuf::from("~/.config/yconn/connections.yaml")),
            connection_count: user.found.then_some(user.count),
        },
        LayerStatus {
            layer: Layer::System,
            path: system_file.clone(),
            connection_count: sys.found.then_some(sys.count),
        },
    ];

    Ok(LoadedConfig {
        connections,
        all_connections,
        docker,
        layers,
        project_dir,
        warnings,
        group: group.map(str::to_owned),
        group_from_file,
    })
}

// ─── Layer loading ────────────────────────────────────────────────────────────

struct LayerData {
    found: bool,
    count: usize,
    connections: Vec<(String, RawConn)>,
    docker: Option<RawDocker>,
    /// Whether a docker block was present (even if it was suppressed).
    docker_present: bool,
}

/// Load a single layer file. `is_project` enables credential field scanning.
fn load_layer(
    path: Option<&Path>,
    layer: Layer,
    is_project: bool,
    warnings: &mut Vec<security::Warning>,
) -> Result<LayerData> {
    let path = match path {
        Some(p) if p.exists() => p,
        _ => {
            return Ok(LayerData {
                found: false,
                count: 0,
                connections: Vec::new(),
                docker: None,
                docker_present: false,
            })
        }
    };

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    // Security checks
    if let Some(w) = security::check_file_permissions(path) {
        warnings.push(w);
    }
    if is_project {
        warnings.extend(security::check_credential_fields(path, &content));
    }

    let raw: RawFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    let docker_present = raw.docker.is_some();
    // User-layer docker is suppressed (caller emits the warning).
    let docker = if layer == Layer::User {
        None
    } else {
        raw.docker
    };

    let count = raw.connections.len();
    let connections: Vec<(String, RawConn)> = raw.connections.into_iter().collect();

    Ok(LayerData {
        found: true,
        count,
        connections,
        docker,
        docker_present,
    })
}

// ─── Upward walk ─────────────────────────────────────────────────────────────

/// Walk upward from `cwd` looking for a project config file, stopping at
/// `$HOME` or the filesystem root.
///
/// Checks three filename conventions per directory in priority order:
/// 1. `.yconn/connections.yaml`
/// 2. `.connections.yaml`
/// 3. `connections.yaml`
///
/// Returns `(yconn_dir, config_file)` where `yconn_dir` is the `.yconn/`
/// directory (used for group discovery), or `None` for the dotfile/plain
/// conventions where no `.yconn/` dir exists.
fn upward_walk(cwd: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let home = dirs::home_dir();
    let mut dir = cwd.to_path_buf();

    loop {
        // Priority 1: .yconn/connections.yaml
        let yconn_dir = dir.join(".yconn");
        let yconn_file = yconn_dir.join("connections.yaml");
        if yconn_file.exists() {
            return (Some(yconn_dir), Some(yconn_file));
        }

        // Priority 2: .connections.yaml
        let dotfile = dir.join(".connections.yaml");
        if dotfile.exists() {
            return (None, Some(dotfile));
        }

        // Priority 3: connections.yaml
        let plain = dir.join("connections.yaml");
        if plain.exists() {
            return (None, Some(plain));
        }

        // Stop at $HOME (don't walk into personal dirs above the project).
        if home.as_ref().is_some_and(|h| dir == *h) {
            break;
        }

        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break,
        }
    }

    (None, None)
}

// ─── Connection merge ─────────────────────────────────────────────────────────

/// Merge connections from three layers (project, user, system — highest first).
///
/// Returns `(active, all)`:
/// - `active`: one entry per name, from the highest-priority layer.
/// - `all`: active entries with shadowed entries interleaved immediately after.
fn merge_connections(layers: &[RawLayer; 3]) -> (Vec<Connection>, Vec<Connection>) {
    // Collect all connections in priority order.
    let mut all_raw: Vec<Connection> = Vec::new();
    for (conns, layer, path) in layers {
        for (name, raw) in conns {
            all_raw.push(build_connection(name, raw, *layer, path, false));
        }
    }

    // Active: first occurrence per name.
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut active: Vec<Connection> = Vec::new();
    for conn in &all_raw {
        if !seen.contains_key(&conn.name) {
            seen.insert(conn.name.clone(), ());
            active.push(conn.clone());
        }
    }

    // All: active entries with their shadowed versions interleaved.
    let mut all: Vec<Connection> = Vec::new();
    for active_conn in &active {
        all.push(active_conn.clone());
        for raw_conn in &all_raw {
            if raw_conn.name == active_conn.name && raw_conn.layer != active_conn.layer {
                let mut shadowed = raw_conn.clone();
                shadowed.shadowed = true;
                all.push(shadowed);
            }
        }
    }

    (active, all)
}

fn build_connection(
    name: &str,
    raw: &RawConn,
    layer: Layer,
    path: &Path,
    shadowed: bool,
) -> Connection {
    Connection {
        name: name.to_string(),
        host: raw.host.clone(),
        user: raw.user.clone(),
        port: raw.port,
        auth: raw.auth.clone(),
        key: raw.key.clone(),
        description: raw.description.clone(),
        link: raw.link.clone(),
        group: raw.group.clone(),
        layer,
        source_path: path.to_path_buf(),
        shadowed,
    }
}

fn docker_config(raw: RawDocker, layer: Layer, path: &Path) -> DockerConfig {
    DockerConfig {
        image: raw.image,
        pull: raw.pull,
        args: raw.args,
        layer,
        source_path: path.to_path_buf(),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn write_yaml(dir: &Path, filename: &str, content: &str) -> PathBuf {
        let path = dir.join(filename);
        fs::write(&path, content).unwrap();
        path
    }

    fn simple_conn(name: &str, host: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth: key\n    description: desc\n"
        )
    }

    fn conn_with_group(name: &str, host: &str, group: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth: key\n    description: desc\n    group: {group}\n"
        )
    }

    fn load_test(cwd: &Path, user_dir: Option<&Path>, system_dir: &Path) -> LoadedConfig {
        load_impl(cwd, None, false, user_dir, system_dir).unwrap()
    }

    fn load_test_with_group(
        cwd: &Path,
        user_dir: Option<&Path>,
        system_dir: &Path,
        group: Option<&str>,
    ) -> LoadedConfig {
        load_impl(cwd, group, group.is_some(), user_dir, system_dir).unwrap()
    }

    // ── upward walk ───────────────────────────────────────────────────────────

    #[test]
    fn test_upward_walk_finds_at_root() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(&yconn, "connections.yaml", &simple_conn("srv", "1.2.3.4"));

        let nested = root.path().join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();

        let (dir, file) = upward_walk(&nested);
        assert_eq!(dir.unwrap(), yconn);
        assert!(file.unwrap().exists());
    }

    #[test]
    fn test_upward_walk_no_config() {
        let dir = TempDir::new().unwrap();
        let (d, f) = upward_walk(dir.path());
        assert!(d.is_none());
        assert!(f.is_none());
    }

    #[test]
    fn test_upward_walk_finds_dotfile_convention() {
        let root = TempDir::new().unwrap();
        let dotfile = root.path().join(".connections.yaml");
        fs::write(&dotfile, simple_conn("srv", "1.2.3.4")).unwrap();

        let nested = root.path().join("sub");
        fs::create_dir_all(&nested).unwrap();

        let (dir, file) = upward_walk(&nested);
        assert!(dir.is_none(), "no .yconn dir for dotfile convention");
        assert_eq!(file.unwrap(), dotfile);
    }

    #[test]
    fn test_upward_walk_finds_plain_convention() {
        let root = TempDir::new().unwrap();
        let plain = root.path().join("connections.yaml");
        fs::write(&plain, simple_conn("srv", "1.2.3.4")).unwrap();

        let nested = root.path().join("sub");
        fs::create_dir_all(&nested).unwrap();

        let (dir, file) = upward_walk(&nested);
        assert!(dir.is_none(), "no .yconn dir for plain convention");
        assert_eq!(file.unwrap(), plain);
    }

    #[test]
    fn test_upward_walk_yconn_beats_dotfile_same_dir() {
        let root = TempDir::new().unwrap();
        // Both present — .yconn/connections.yaml must win.
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &simple_conn("yconn-srv", "1.1.1.1"),
        );
        fs::write(
            root.path().join(".connections.yaml"),
            simple_conn("dotfile-srv", "2.2.2.2"),
        )
        .unwrap();

        let (_, file) = upward_walk(root.path());
        let content = fs::read_to_string(file.unwrap()).unwrap();
        assert!(
            content.contains("yconn-srv"),
            ".yconn convention must beat dotfile"
        );
    }

    #[test]
    fn test_upward_walk_dotfile_beats_plain_same_dir() {
        let root = TempDir::new().unwrap();
        // .connections.yaml beats connections.yaml.
        fs::write(
            root.path().join(".connections.yaml"),
            simple_conn("dotfile-srv", "2.2.2.2"),
        )
        .unwrap();
        fs::write(
            root.path().join("connections.yaml"),
            simple_conn("plain-srv", "3.3.3.3"),
        )
        .unwrap();

        let (_, file) = upward_walk(root.path());
        let content = fs::read_to_string(file.unwrap()).unwrap();
        assert!(
            content.contains("dotfile-srv"),
            "dotfile convention must beat plain"
        );
    }

    #[test]
    fn test_upward_walk_finds_closest_ancestor() {
        let root = TempDir::new().unwrap();
        // Place config at two levels — walk should stop at the deeper one.
        let outer_yconn = root.path().join(".yconn");
        fs::create_dir_all(&outer_yconn).unwrap();
        write_yaml(
            &outer_yconn,
            "connections.yaml",
            &simple_conn("outer", "1.1.1.1"),
        );

        let inner = root.path().join("inner");
        let inner_yconn = inner.join(".yconn");
        fs::create_dir_all(&inner_yconn).unwrap();
        write_yaml(
            &inner_yconn,
            "connections.yaml",
            &simple_conn("inner", "2.2.2.2"),
        );

        let (_, file) = upward_walk(&inner);
        let content = fs::read_to_string(file.unwrap()).unwrap();
        assert!(content.contains("inner"));
    }

    // ── single layer ──────────────────────────────────────────────────────────

    #[test]
    fn test_single_project_layer() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(&yconn, "connections.yaml", &simple_conn("prod", "10.0.0.1"));

        let empty = TempDir::new().unwrap();
        let cfg = load_test(root.path(), None, empty.path());
        assert_eq!(cfg.connections.len(), 1);
        assert_eq!(cfg.connections[0].name, "prod");
        assert_eq!(cfg.connections[0].layer, Layer::Project);
    }

    #[test]
    fn test_single_user_layer() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("my-box", "192.168.1.5"),
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        assert_eq!(cfg.connections.len(), 1);
        assert_eq!(cfg.connections[0].name, "my-box");
        assert_eq!(cfg.connections[0].layer, Layer::User);
    }

    #[test]
    fn test_single_system_layer() {
        let cwd = TempDir::new().unwrap();
        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("bastion", "10.0.0.254"),
        );

        let cfg = load_test(cwd.path(), None, sys.path());
        assert_eq!(cfg.connections.len(), 1);
        assert_eq!(cfg.connections[0].layer, Layer::System);
    }

    // ── collision priority scenarios ──────────────────────────────────────────

    #[test]
    fn test_project_overrides_user() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &simple_conn("srv", "project-host"),
        );

        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("srv", "user-host"),
        );

        let empty = TempDir::new().unwrap();
        let cfg = load_test(root.path(), Some(user.path()), empty.path());

        let conn = cfg.find("srv").unwrap();
        assert_eq!(conn.host, "project-host");
        assert_eq!(conn.layer, Layer::Project);
    }

    #[test]
    fn test_project_overrides_system() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &simple_conn("srv", "project-host"),
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("srv", "system-host"),
        );

        let cfg = load_test(root.path(), None, sys.path());
        assert_eq!(cfg.find("srv").unwrap().host, "project-host");
    }

    #[test]
    fn test_user_overrides_system() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("srv", "user-host"),
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("srv", "system-host"),
        );

        let cfg = load_test(cwd.path(), Some(user.path()), sys.path());
        assert_eq!(cfg.find("srv").unwrap().host, "user-host");
        assert_eq!(cfg.find("srv").unwrap().layer, Layer::User);
    }

    #[test]
    fn test_project_overrides_both() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &simple_conn("srv", "project-host"),
        );

        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("srv", "user-host"),
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("srv", "system-host"),
        );

        let cfg = load_test(root.path(), Some(user.path()), sys.path());
        assert_eq!(cfg.find("srv").unwrap().host, "project-host");
    }

    #[test]
    fn test_no_collision_all_layers() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &simple_conn("proj-srv", "1.0.0.1"),
        );

        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("user-srv", "2.0.0.1"),
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("sys-srv", "3.0.0.1"),
        );

        let cfg = load_test(root.path(), Some(user.path()), sys.path());
        assert_eq!(cfg.connections.len(), 3);
        assert!(cfg.find("proj-srv").is_some());
        assert!(cfg.find("user-srv").is_some());
        assert!(cfg.find("sys-srv").is_some());
    }

    #[test]
    fn test_name_only_in_system() {
        let cwd = TempDir::new().unwrap();
        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("sys-only", "10.0.0.1"),
        );

        let cfg = load_test(cwd.path(), None, sys.path());
        let conn = cfg.find("sys-only").unwrap();
        assert_eq!(conn.layer, Layer::System);
    }

    #[test]
    fn test_name_only_in_user() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("user-only", "10.0.0.2"),
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        let conn = cfg.find("user-only").unwrap();
        assert_eq!(conn.layer, Layer::User);
    }

    // ── missing files ─────────────────────────────────────────────────────────

    #[test]
    fn test_missing_layer_silently_skipped() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("srv", "1.2.3.4"),
        );

        // System dir exists but has no connections.yaml — should be skipped.
        let sys = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), Some(user.path()), sys.path());
        assert_eq!(cfg.connections.len(), 1);
        assert!(cfg.layers[2].connection_count.is_none());
    }

    // ── shadowed entries ──────────────────────────────────────────────────────

    #[test]
    fn test_shadowed_entries_in_all_connections() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &simple_conn("srv", "project-host"),
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("srv", "system-host"),
        );

        let cfg = load_test(root.path(), None, sys.path());
        assert_eq!(cfg.connections.len(), 1);
        assert_eq!(cfg.all_connections.len(), 2);

        let active = cfg.all_connections.iter().find(|c| !c.shadowed).unwrap();
        assert_eq!(active.host, "project-host");

        let shadowed = cfg.all_connections.iter().find(|c| c.shadowed).unwrap();
        assert_eq!(shadowed.host, "system-host");
        assert_eq!(shadowed.layer, Layer::System);
    }

    #[test]
    fn test_shadowed_entry_interleaved_after_active() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &format!(
                "{}\n{}",
                simple_conn("alpha", "1.0.0.1"),
                "  beta:\n    host: 2.0.0.2\n    user: u\n    auth: key\n    description: d\n"
            ),
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            &simple_conn("alpha", "3.0.0.3"),
        );

        let cfg = load_test(root.path(), None, sys.path());
        // alpha (active), alpha (shadowed), beta — shadowed appears right after active
        let shadowed_idx = cfg.all_connections.iter().position(|c| c.shadowed).unwrap();
        let active_idx = cfg
            .all_connections
            .iter()
            .position(|c| c.name == "alpha" && !c.shadowed)
            .unwrap();
        assert_eq!(shadowed_idx, active_idx + 1);
    }

    // ── docker block ──────────────────────────────────────────────────────────

    #[test]
    fn test_docker_from_project_layer() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "docker:\n  image: ghcr.io/org/keys:latest\nconnections: {}\n",
        );

        let empty = TempDir::new().unwrap();
        let cfg = load_test(root.path(), None, empty.path());
        let docker = cfg.docker.unwrap();
        assert_eq!(docker.image, "ghcr.io/org/keys:latest");
        assert_eq!(docker.layer, Layer::Project);
    }

    #[test]
    fn test_docker_from_system_layer() {
        let cwd = TempDir::new().unwrap();
        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            "docker:\n  image: registry/img:v1\nconnections: {}\n",
        );

        let cfg = load_test(cwd.path(), None, sys.path());
        assert!(cfg.docker.is_some());
        assert_eq!(cfg.docker.unwrap().layer, Layer::System);
    }

    #[test]
    fn test_docker_project_takes_priority_over_system() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "docker:\n  image: project-image\nconnections: {}\n",
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            "docker:\n  image: system-image\nconnections: {}\n",
        );

        let cfg = load_test(root.path(), None, sys.path());
        assert_eq!(cfg.docker.unwrap().image, "project-image");
    }

    #[test]
    fn test_docker_in_user_layer_ignored_with_warning() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "docker:\n  image: bad-image\nconnections: {}\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        assert!(cfg.docker.is_none());
        assert!(!cfg.warnings.is_empty());
        assert!(cfg.warnings.iter().any(|w| w.message.contains("docker")));
    }

    #[test]
    fn test_no_docker_block() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("srv", "1.2.3.4"),
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        assert!(cfg.docker.is_none());
    }

    // ── docker pull default ───────────────────────────────────────────────────

    #[test]
    fn test_docker_pull_defaults_to_missing() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "docker:\n  image: img\nconnections: {}\n",
        );

        let empty = TempDir::new().unwrap();
        let cfg = load_test(root.path(), None, empty.path());
        assert_eq!(cfg.docker.unwrap().pull, "missing");
    }

    // ── connection field defaults ─────────────────────────────────────────────

    #[test]
    fn test_port_defaults_to_22() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("srv", "1.2.3.4"),
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        assert_eq!(cfg.connections[0].port, 22);
    }

    // ── layer status ──────────────────────────────────────────────────────────

    #[test]
    fn test_layer_status_counts() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  a:\n    host: h\n    user: u\n    auth: key\n    description: d\n  b:\n    host: h2\n    user: u2\n    auth: key\n    description: d2\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        assert_eq!(cfg.layers[0].connection_count, None); // project not found
        assert_eq!(cfg.layers[1].connection_count, Some(2)); // user: 2 connections
        assert_eq!(cfg.layers[2].connection_count, None); // system not found
    }

    // ── inline group field ────────────────────────────────────────────────────

    #[test]
    fn test_group_field_round_trip() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &conn_with_group("work-srv", "10.0.0.1", "work"),
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        let conn = cfg.find("work-srv").unwrap();
        assert_eq!(conn.group.as_deref(), Some("work"));
    }

    #[test]
    fn test_group_field_absent_is_none() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("srv", "1.2.3.4"),
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        let conn = cfg.find("srv").unwrap();
        assert!(conn.group.is_none());
    }

    // ── group filtering ───────────────────────────────────────────────────────

    #[test]
    fn test_filtered_connections_no_filter() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        let yaml = format!(
            "{}\n{}",
            conn_with_group("work-srv", "10.0.0.1", "work"),
            // Simple conn in same connections block — just append raw
            "  plain-srv:\n    host: 10.0.0.2\n    user: user\n    auth: key\n    description: desc\n"
        );
        write_yaml(user.path(), "connections.yaml", &yaml);
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        // No filter → all connections returned
        let filtered = cfg.filtered_connections(None);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filtered_connections_with_group_filter() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        let yaml = format!(
            "{}\n{}",
            conn_with_group("work-srv", "10.0.0.1", "work"),
            "  plain-srv:\n    host: 10.0.0.2\n    user: user\n    auth: key\n    description: desc\n"
        );
        write_yaml(user.path(), "connections.yaml", &yaml);
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        // Filter by "work" group → only work-srv returned
        let filtered = cfg.filtered_connections(Some("work"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "work-srv");
    }

    #[test]
    fn test_effective_group_filter_all_overrides_everything() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load_test_with_group(cwd.path(), None, empty.path(), Some("work"));
        // --all overrides locked group
        assert_eq!(cfg.effective_group_filter(true, None), None);
    }

    #[test]
    fn test_effective_group_filter_group_flag_overrides_lock() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load_test_with_group(cwd.path(), None, empty.path(), Some("work"));
        // --group private overrides locked "work"
        assert_eq!(
            cfg.effective_group_filter(false, Some("private")),
            Some("private")
        );
    }

    #[test]
    fn test_effective_group_filter_locked_group_used() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load_test_with_group(cwd.path(), None, empty.path(), Some("work"));
        // No --all, no --group flag → locked group used
        assert_eq!(cfg.effective_group_filter(false, None), Some("work"));
    }

    #[test]
    fn test_effective_group_filter_no_lock_no_flags() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), None, empty.path());
        // No lock, no flags → None (show all)
        assert_eq!(cfg.effective_group_filter(false, None), None);
    }

    // ── discover_groups ───────────────────────────────────────────────────────

    #[test]
    fn test_discover_groups_empty() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), None, empty.path());
        let groups = cfg.discover_groups();
        assert!(groups.is_empty());
    }

    #[test]
    fn test_discover_groups_from_connections() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        let yaml = format!(
            "{}\n{}",
            conn_with_group("work-srv", "10.0.0.1", "work"),
            "  private-srv:\n    host: 10.0.0.2\n    user: user\n    auth: key\n    description: desc\n    group: private\n"
        );
        write_yaml(user.path(), "connections.yaml", &yaml);
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        let groups = cfg.discover_groups();
        assert_eq!(groups.len(), 2);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"work"));
        assert!(names.contains(&"private"));
    }

    #[test]
    fn test_discover_groups_sorted_by_name() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        let yaml = format!(
            "{}\n{}",
            conn_with_group("z-srv", "10.0.0.1", "zebra"),
            "  a-srv:\n    host: 10.0.0.2\n    user: user\n    auth: key\n    description: desc\n    group: alpha\n"
        );
        write_yaml(user.path(), "connections.yaml", &yaml);
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        let groups = cfg.discover_groups();
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zebra"]);
    }

    #[test]
    fn test_discover_groups_tracks_layers() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            &conn_with_group("p-srv", "10.0.0.1", "work"),
        );

        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &conn_with_group("u-srv", "10.0.0.2", "work"),
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(root.path(), Some(user.path()), empty.path());
        let groups = cfg.discover_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "work");
        // Both project and user layers have a "work" connection
        let layers = &groups[0].layers;
        assert!(layers.contains(&"project".to_string()));
        assert!(layers.contains(&"user".to_string()));
    }
}
