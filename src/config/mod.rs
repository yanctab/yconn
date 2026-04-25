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
use serde::{Deserialize, Serialize};

use crate::security;

// ─── Wire types (serde) ───────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct RawFile {
    #[serde(default)]
    docker: Option<RawDocker>,
    #[serde(default)]
    connections: HashMap<String, RawConn>,
    #[serde(default)]
    users: HashMap<String, String>,
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
    /// Required fields are `Option` so we can emit a clear error naming the
    /// missing field instead of the opaque serde_yaml "missing field" message.
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default)]
    auth: Option<Auth>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    link: Option<String>,
    /// Optional inline group tag. Connections without a `group:` field belong
    /// to no group and are always shown unless a group filter is active.
    #[serde(default)]
    group: Option<String>,
}

// ─── Auth enum ───────────────────────────────────────────────────────────────

/// Structured authentication configuration for a connection.
///
/// Serialised as an internally-tagged YAML mapping: `{ type: key, key: ... }`.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "type")]
pub enum Auth {
    /// Key-based authentication. `key` is the path to the private key file.
    /// `generate_key` is an optional shell command (parsed and stored only — not executed).
    #[serde(rename = "key")]
    Key {
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generate_key: Option<String>,
    },
    /// Password authentication — SSH prompts natively.
    #[serde(rename = "password")]
    Password,
    /// Identity-only authentication for ssh-config entries (e.g. git hosts).
    /// Produces `IdentityFile` + `IdentitiesOnly yes` in SSH config output.
    #[serde(rename = "identity")]
    Identity {
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generate_key: Option<String>,
    },
}

impl Auth {
    /// Return a human-readable label for the auth type.
    pub fn type_label(&self) -> &str {
        match self {
            Auth::Key { .. } => "key",
            Auth::Password => "password",
            Auth::Identity { .. } => "identity",
        }
    }

    /// Return the key path, if any.
    pub fn key(&self) -> Option<&str> {
        match self {
            Auth::Key { ref key, .. } | Auth::Identity { ref key, .. } => Some(key.as_str()),
            Auth::Password => None,
        }
    }

    /// Return the generate_key field, if any.
    ///
    /// This returns the raw, unexpanded value as stored in config. For display
    /// purposes the `${key}` token should be expanded to the connection's
    /// `auth.key` value — see [`Auth::generate_key_expanded`].
    pub fn generate_key(&self) -> Option<&str> {
        match self {
            Auth::Key {
                ref generate_key, ..
            }
            | Auth::Identity {
                ref generate_key, ..
            } => generate_key.as_deref(),
            Auth::Password => None,
        }
    }

    /// Return `generate_key` with the literal `${key}` token expanded to the
    /// connection's `auth.key` value.
    ///
    /// Expansion rules:
    /// - Every occurrence of the literal `${key}` is replaced with the value
    ///   returned by [`Auth::key`].
    /// - For variants without a key ([`Auth::Password`]), `${key}` expands to
    ///   an empty string. This invariant must hold if a future variant adds
    ///   `generate_key` without a `key` field.
    /// - Other `${...}` tokens are passed through unchanged — only the exact
    ///   literal `${key}` sequence is expanded.
    ///
    /// The stored enum data is not mutated: this is a view, not a rewrite.
    /// `--dump` serialises the raw enum fields and therefore still emits the
    /// original unexpanded value.
    pub fn generate_key_expanded(&self) -> Option<String> {
        let raw = self.generate_key()?;
        let key_value = self.key().unwrap_or("");
        Some(raw.replace("${key}", key_value))
    }

    /// Return `generate_key` with both `${key}` and `${user}` expanded in a
    /// single pass.
    ///
    /// Single-pass means substituted text is never re-scanned: if a
    /// substitution introduces text that itself looks like a placeholder
    /// (e.g. a `user` value of `${key}`), it is left in the output verbatim.
    ///
    /// Unknown `${...}` tokens are passed through unchanged.
    /// `${key}` expands to the value of [`Auth::key`] (empty string when
    /// absent). The raw enum data is not mutated; this is a view.
    pub fn generate_key_rendered(&self, user: &str) -> Option<String> {
        let raw = self.generate_key()?;
        let key_value = self.key().unwrap_or("");
        Some(render_placeholders(raw, key_value, user))
    }
}

/// Single-pass placeholder expander. Walks `input` left-to-right and replaces
/// each occurrence of `${key}` with `key_value` and `${user}` with `user_value`.
/// Substituted text is appended directly to the output and is not rescanned,
/// so a user value containing `${key}` is preserved verbatim.
fn render_placeholders(input: &str, key_value: &str, user_value: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = input[i + 2..].find('}') {
                let token = &input[i + 2..i + 2 + end];
                match token {
                    "key" => {
                        out.push_str(key_value);
                        i += 2 + end + 1;
                        continue;
                    }
                    "user" => {
                        out.push_str(user_value);
                        i += 2 + end + 1;
                        continue;
                    }
                    _ => {}
                }
            }
        }
        // Push the next char as-is. Use char_indices semantics to keep utf-8
        // intact even though placeholders are ASCII-only.
        let ch = input[i..].chars().next().expect("non-empty remainder");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
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
    pub auth: Auth,
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

/// A resolved user entry from the `users:` map, with source tracking.
#[derive(Clone, Debug)]
pub struct UserEntry {
    pub key: String,
    pub value: String,
    pub layer: Layer,
    pub source_path: PathBuf,
    /// `true` if a higher-priority layer defines a user entry with the same key.
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
    /// Merged user entries from the `users:` map across all layers.
    /// Active entries only (one per key, highest-priority layer wins).
    pub users: HashMap<String, UserEntry>,
    /// Active + shadowed user entries interleaved for `user show --all` display.
    pub all_users: Vec<UserEntry>,
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

/// Parse a `[N..M]` numeric range suffix from a connection name.
///
/// Returns `(prefix, start, end)` when the name ends with `[N..M]` where
/// `N` and `M` are unsigned integers. Returns `None` for any other form.
///
/// Examples:
/// - `"server[1..10]"` → `Some(("server", 1, 10))`
/// - `"web-[01..99]"` → `Some(("web-", 1, 99))`
/// - `"server*"`      → `None`
fn parse_range_pattern(name: &str) -> Option<(&str, u64, u64)> {
    let bracket = name.rfind('[')?;
    let prefix = &name[..bracket];
    let rest = &name[bracket + 1..];
    // `]` must be the very last character.
    if !rest.ends_with(']') {
        return None;
    }
    let range_str = &rest[..rest.len() - 1];
    let (start_str, end_str) = range_str.split_once("..")?;
    let start: u64 = start_str.parse().ok()?;
    let end: u64 = end_str.parse().ok()?;
    Some((prefix, start, end))
}

/// Return `true` if `input` matches the numeric range pattern `conn_name`.
///
/// The pattern must have the form `<prefix>[N..M]`. The input must start with
/// `<prefix>` and the remaining suffix must parse as a `u64` in `[N, M]`
/// inclusive. Returns `false` for empty ranges (end < start), non-range
/// names, and inputs whose suffix is not a non-negative integer.
fn range_matches(conn_name: &str, input: &str) -> bool {
    match parse_range_pattern(conn_name) {
        None => false,
        Some((prefix, start, end)) => {
            if end < start {
                return false;
            }
            match input.strip_prefix(prefix) {
                None => false,
                Some(suffix) => suffix.parse::<u64>().is_ok_and(|n| n >= start && n <= end),
            }
        }
    }
}

impl LoadedConfig {
    /// Find a connection by name (active connections only).
    pub fn find(&self, name: &str) -> Option<&Connection> {
        self.connections.iter().find(|c| c.name == name)
    }

    /// Find a connection by exact name, wildcard pattern, or numeric range pattern.
    ///
    /// Resolution order:
    /// 1. Try an exact name lookup first — an exact match always wins.
    /// 2. Scan all active (non-shadowed) connections for patterns that match
    ///    `input`. Two pattern kinds are recognised:
    ///    - **Glob** — name contains `*` or `?`; matched with `WildMatch`.
    ///    - **Range** — name ends with `[N..M]`; matched by numeric suffix.
    /// 3. If exactly one pattern matches, return a clone of that connection
    ///    with `host` resolved: if it contains `${name}`, only that token is
    ///    replaced with the input; otherwise the whole field is replaced.
    /// 4. If two or more *different* patterns match, return an error naming
    ///    each conflicting pattern and its source layer/file.
    /// 5. Same-pattern shadowing across layers is handled by the existing
    ///    priority merge — only the winning entry is in `self.connections`,
    ///    so the same pattern name never appears twice here.
    pub fn find_with_wildcard(&self, input: &str) -> anyhow::Result<Connection> {
        use wildmatch::WildMatch;

        // Step 1: exact name lookup.
        if let Some(conn) = self.find(input) {
            return Ok(conn.clone());
        }

        // Step 2: scan glob and range patterns in the active (non-shadowed) set.
        let mut matches: Vec<&Connection> = Vec::new();
        for conn in &self.connections {
            let is_glob = conn.name.contains('*') || conn.name.contains('?');
            let matched = if is_glob {
                WildMatch::new(&conn.name).matches(input)
            } else {
                range_matches(&conn.name, input)
            };
            if matched {
                matches.push(conn);
            }
        }

        match matches.len() {
            0 => Err(anyhow::anyhow!("no connection named '{input}'")),
            1 => {
                let mut resolved = matches[0].clone();
                // Substitute the matched input into the host field.
                // If the host contains "${name}", replace only that token;
                // otherwise fall back to replacing the entire host field.
                resolved.host = if resolved.host.contains("${name}") {
                    resolved.host.replace("${name}", input)
                } else {
                    input.to_string()
                };
                Ok(resolved)
            }
            _ => {
                // Two or more *different* patterns matched — that is a conflict.
                // (Same-pattern shadowing never reaches here because merge keeps
                // only one winner per pattern name in self.connections.)
                let conflict_list: Vec<String> = matches
                    .iter()
                    .map(|c| {
                        format!(
                            "  '{}' (layer: {}, file: {})",
                            c.name,
                            c.layer.label(),
                            c.source_path.display()
                        )
                    })
                    .collect();
                Err(anyhow::anyhow!(
                    "connection name '{}' matches multiple patterns:\n{}",
                    input,
                    conflict_list.join("\n")
                ))
            }
        }
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

    /// Expand `${<key>}` templates in a `user` field using the merged users map
    /// plus any inline overrides supplied at invocation time.
    ///
    /// Resolution order (one level only — no recursive expansion):
    /// 1. Named entries: for each `${key}` token where `key != "user"`, look up
    ///    `key` in `inline_overrides` first, then `self.users`. Replace if found.
    /// 2. `${user}` (lowercase, literal): replaced with the `$USER` environment
    ///    variable. If `$USER` is unset, `${user}` passes through unchanged.
    ///    `--user user:<name>` in `inline_overrides` overrides this step too.
    ///
    /// When a template token remains unresolved after both steps, a warning
    /// message is returned in the `Vec<String>` so the caller can emit it.
    ///
    /// `inline_overrides` is a `HashMap<String, String>` of `key → value` pairs
    /// from `--user key:value` CLI flags. It shadows `self.users` for the
    /// duration of this call only.
    pub fn expand_user_field(
        &self,
        field: &str,
        inline_overrides: &HashMap<String, String>,
    ) -> (String, Vec<String>) {
        let mut result = field.to_string();
        let mut warnings: Vec<String> = Vec::new();

        // Step 1: resolve named entries (everything except the literal `${user}`).
        // We scan for ${...} tokens and replace any whose key is in the users map
        // (via inline_overrides or self.users), skipping the key "user" so it
        // is handled separately in step 2.
        let mut i = 0;
        let chars: Vec<char> = result.chars().collect();
        let mut new_result = String::new();
        let s = result.clone();
        let bytes = s.as_bytes();
        let len = bytes.len();
        while i < len {
            // Look for "${"
            if i + 1 < len && bytes[i] == b'$' && bytes[i + 1] == b'{' {
                if let Some(close) = s[i + 2..].find('}') {
                    let key = &s[i + 2..i + 2 + close];
                    // Skip the literal `${user}` token — handled in step 2.
                    if key == "user" {
                        new_result.push_str("${user}");
                        i += 2 + close + 1;
                        continue;
                    }
                    // Look up key in inline_overrides, then self.users.
                    if let Some(val) = inline_overrides
                        .get(key)
                        .map(|s| s.as_str())
                        .or_else(|| self.users.get(key).map(|e| e.value.as_str()))
                    {
                        new_result.push_str(val);
                    } else {
                        // Unresolved — pass through and warn.
                        let token = format!("${{{key}}}");
                        warnings.push(format!(
                            "user field template '{}' is unresolved: no users: entry for key '{key}'",
                            token
                        ));
                        new_result.push_str(&token);
                    }
                    i += 2 + close + 1;
                    continue;
                }
            }
            // Not a template token — copy the byte literally.
            new_result.push(bytes[i] as char);
            i += 1;
        }
        // Suppress the unused variable warning from chars above.
        let _ = chars;
        result = new_result;

        // Step 2: resolve `${user}` using inline_overrides["user"] or $USER env var.
        if result.contains("${user}") {
            // Check inline_overrides first (--user user:<name> overrides env var).
            if let Some(val) = inline_overrides.get("user") {
                result = result.replace("${user}", val);
            } else {
                match std::env::var("USER") {
                    Ok(env_user) => {
                        result = result.replace("${user}", &env_user);
                    }
                    Err(_) => {
                        // $USER is unset — pass through unchanged.
                        warnings.push(
                            "user field contains '${user}' but $USER env var is unset; \
                             passing through unchanged"
                                .to_string(),
                        );
                    }
                }
            }
        }

        (result, warnings)
    }

    /// Return unique group values present across all active connections.
    /// Used by `yconn groups list`.
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
///
/// The system-layer directory defaults to `/etc/yconn`, but can be overridden
/// at runtime by setting the `YCONN_SYSTEM_CONFIG_DIR` environment variable.
/// This override exists primarily to allow integration tests to point the
/// system layer at a per-test temp directory; it is also documented for any
/// production caller that needs to relocate the system layer (e.g. packaged
/// installs that ship configs in a non-standard location).
pub fn load_from(cwd: &Path) -> Result<LoadedConfig> {
    let ag = crate::group::active_group()?;
    let user_dir = dirs::config_dir().map(|d| d.join("yconn"));
    let system_dir = system_config_dir();
    load_impl(
        cwd,
        ag.name.as_deref(),
        ag.from_file,
        user_dir.as_deref(),
        &system_dir,
    )
}

/// Resolve the system-layer config directory.
///
/// When `YCONN_SYSTEM_CONFIG_DIR` is set in the environment, its value is
/// used verbatim. Otherwise the default `/etc/yconn` path is returned.
pub fn system_config_dir() -> PathBuf {
    std::env::var("YCONN_SYSTEM_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/etc/yconn"))
}

// ─── Internal implementation ──────────────────────────────────────────────────

/// One element of the three-layer raw-connection array: (connections, layer, source path).
type RawLayer = (Vec<(String, RawConn)>, Layer, PathBuf);

/// One element of the three-layer raw-users array: (entries, layer, source path).
type RawUserLayer = (Vec<(String, String)>, Layer, PathBuf);

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

    // ── Merge users ──────────────────────────────────────────────────────────
    let user_layers: [RawUserLayer; 3] = [
        (
            proj.users,
            Layer::Project,
            project_file
                .clone()
                .unwrap_or_else(|| PathBuf::from(".yconn")),
        ),
        (
            user.users,
            Layer::User,
            user_file
                .clone()
                .unwrap_or_else(|| PathBuf::from("~/.config/yconn")),
        ),
        (sys.users, Layer::System, system_file.clone()),
    ];
    let (users, all_users) = merge_users(&user_layers);

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
        users,
        all_users,
        docker,
        layers,
        project_dir,
        warnings,
        group: group.map(str::to_owned),
        group_from_file,
    })
}

// ─── Layer loading ────────────────────────────────────────────────────────────

/// Validate that every connection entry in `connections` has all required
/// fields present. Returns a clear error naming the config file, the
/// connection entry, and the first missing required field.
fn validate_connections(path: &Path, connections: &HashMap<String, RawConn>) -> Result<()> {
    for (name, raw) in connections {
        let file = path.display();
        if raw.host.is_none() {
            anyhow::bail!("{file}: connection '{name}' is missing required field 'host'");
        }
        if raw.user.is_none() {
            anyhow::bail!("{file}: connection '{name}' is missing required field 'user'");
        }
        if raw.auth.is_none() {
            anyhow::bail!("{file}: connection '{name}' is missing required field 'auth'");
        }
        if raw.description.is_none() {
            anyhow::bail!("{file}: connection '{name}' is missing required field 'description'");
        }
    }
    Ok(())
}

/// Default auth value used when auth is None (should not happen after
/// validation, but provides a safe fallback).
fn default_auth() -> Auth {
    Auth::Password
}

struct LayerData {
    found: bool,
    count: usize,
    connections: Vec<(String, RawConn)>,
    users: Vec<(String, String)>,
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
                users: Vec::new(),
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
        .with_context(|| format!("failed to parse {}: invalid YAML syntax", path.display()))?;

    // Validate required connection fields — emit clear errors naming the file,
    // connection entry, and the missing field rather than opaque serde messages.
    validate_connections(path, &raw.connections)?;

    let docker_present = raw.docker.is_some();
    // User-layer docker is suppressed (caller emits the warning).
    let docker = if layer == Layer::User {
        None
    } else {
        raw.docker
    };

    let count = raw.connections.len();
    let connections: Vec<(String, RawConn)> = raw.connections.into_iter().collect();
    let users: Vec<(String, String)> = raw.users.into_iter().collect();

    Ok(LayerData {
        found: true,
        count,
        connections,
        users,
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
    // SAFETY: validate_connections() is called before build_connection() in
    // load_layer(), so all required Option fields are guaranteed to be Some.
    Connection {
        name: name.to_string(),
        host: raw.host.clone().unwrap_or_default(),
        user: raw.user.clone().unwrap_or_default(),
        port: raw.port,
        auth: raw.auth.clone().unwrap_or_else(default_auth),
        description: raw.description.clone().unwrap_or_default(),
        link: raw.link.clone(),
        group: raw.group.clone(),
        layer,
        source_path: path.to_path_buf(),
        shadowed,
    }
}

/// Merge `users:` maps from three layers (project, user, system — highest first).
///
/// Returns `(active, all)`:
/// - `active`: one entry per key, from the highest-priority layer (as a HashMap).
/// - `all`: active entries with shadowed entries interleaved immediately after.
fn merge_users(layers: &[RawUserLayer; 3]) -> (HashMap<String, UserEntry>, Vec<UserEntry>) {
    // Collect all entries in priority order.
    let mut all_raw: Vec<UserEntry> = Vec::new();
    for (entries, layer, path) in layers {
        for (key, value) in entries {
            all_raw.push(UserEntry {
                key: key.clone(),
                value: value.clone(),
                layer: *layer,
                source_path: path.clone(),
                shadowed: false,
            });
        }
    }

    // Active: first occurrence per key.
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut active: HashMap<String, UserEntry> = HashMap::new();
    let mut active_order: Vec<String> = Vec::new();
    for entry in &all_raw {
        if !seen.contains_key(&entry.key) {
            seen.insert(entry.key.clone(), ());
            active.insert(entry.key.clone(), entry.clone());
            active_order.push(entry.key.clone());
        }
    }

    // All: active entries with their shadowed versions interleaved.
    let mut all: Vec<UserEntry> = Vec::new();
    for key in &active_order {
        let active_entry = active.get(key).unwrap();
        all.push(active_entry.clone());
        for raw_entry in &all_raw {
            if raw_entry.key == *key && raw_entry.layer != active_entry.layer {
                let mut shadowed = raw_entry.clone();
                shadowed.shadowed = true;
                all.push(shadowed);
            }
        }
    }

    (active, all)
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
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n"
        )
    }

    fn conn_with_group(name: &str, host: &str, group: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n    group: {group}\n"
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

    // ── Auth::generate_key_expanded ─────────────────────────────────────────

    #[test]
    fn test_generate_key_expanded_key_auth_replaces_placeholder() {
        let auth = Auth::Key {
            key: "~/.ssh/foo".to_string(),
            generate_key: Some("vault read ssh/foo > ${key}".to_string()),
        };
        assert_eq!(
            auth.generate_key_expanded().as_deref(),
            Some("vault read ssh/foo > ~/.ssh/foo")
        );
    }

    #[test]
    fn test_generate_key_expanded_identity_auth_replaces_placeholder() {
        let auth = Auth::Identity {
            key: "~/.ssh/gitlab_key".to_string(),
            generate_key: Some("op read secret > ${key}".to_string()),
        };
        assert_eq!(
            auth.generate_key_expanded().as_deref(),
            Some("op read secret > ~/.ssh/gitlab_key")
        );
    }

    #[test]
    fn test_generate_key_expanded_password_auth_returns_none() {
        // Auth::Password has no generate_key field — the expanded accessor
        // must return None (not Some("")) because there is nothing to expand.
        let auth = Auth::Password;
        assert_eq!(auth.generate_key_expanded(), None);
    }

    #[test]
    fn test_generate_key_expanded_without_placeholder_returns_verbatim() {
        let auth = Auth::Key {
            key: "~/.ssh/foo".to_string(),
            generate_key: Some("vault read ssh/foo".to_string()),
        };
        assert_eq!(
            auth.generate_key_expanded().as_deref(),
            Some("vault read ssh/foo")
        );
    }

    #[test]
    fn test_generate_key_expanded_multiple_occurrences_all_replaced() {
        let auth = Auth::Key {
            key: "~/.ssh/id".to_string(),
            generate_key: Some("cp ${key}.src ${key}".to_string()),
        };
        assert_eq!(
            auth.generate_key_expanded().as_deref(),
            Some("cp ~/.ssh/id.src ~/.ssh/id")
        );
    }

    #[test]
    fn test_generate_key_expanded_leaves_other_tokens_unchanged() {
        let auth = Auth::Key {
            key: "~/.ssh/id".to_string(),
            generate_key: Some("echo ${other} > ${key}".to_string()),
        };
        // Only ${key} is expanded — ${other} is passed through verbatim.
        assert_eq!(
            auth.generate_key_expanded().as_deref(),
            Some("echo ${other} > ~/.ssh/id")
        );
    }

    #[test]
    fn test_generate_key_expanded_none_field_returns_none() {
        let auth = Auth::Key {
            key: "~/.ssh/foo".to_string(),
            generate_key: None,
        };
        assert_eq!(auth.generate_key_expanded(), None);
    }

    #[test]
    fn test_generate_key_raw_accessor_preserves_placeholder() {
        // The raw accessor must NOT expand ${key} — it is used by callers that
        // need the original config value (e.g. --dump serialisation).
        let auth = Auth::Key {
            key: "~/.ssh/foo".to_string(),
            generate_key: Some("vault read ssh/foo > ${key}".to_string()),
        };
        assert_eq!(auth.generate_key(), Some("vault read ssh/foo > ${key}"));
    }

    // ── Auth::generate_key_rendered (issue #83) ─────────────────────────────

    /// Both `${key}` and `${user}` are expanded together in a single render.
    #[test]
    fn test_generate_key_rendered_expands_both_placeholders() {
        let auth = Auth::Key {
            key: "~/.ssh/foo".to_string(),
            generate_key: Some("vault read ssh/${user} > ${key}".to_string()),
        };
        assert_eq!(
            auth.generate_key_rendered("alice").as_deref(),
            Some("vault read ssh/alice > ~/.ssh/foo")
        );
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
                "  beta:\n    host: 2.0.0.2\n    user: u\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: d\n"
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
            "connections:\n  a:\n    host: h\n    user: u\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: d\n  b:\n    host: h2\n    user: u2\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: d2\n",
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
            "  plain-srv:\n    host: 10.0.0.2\n    user: user\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n"
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
            "  plain-srv:\n    host: 10.0.0.2\n    user: user\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n"
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
            "  private-srv:\n    host: 10.0.0.2\n    user: user\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n    group: private\n"
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
            "  a-srv:\n    host: 10.0.0.2\n    user: user\n    auth:\n      type: key\n      key: ~/.ssh/id_rsa\n    description: desc\n    group: alpha\n"
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

    // ── wildcard pattern matching ─────────────────────────────────────────────

    /// A single wildcard pattern matches the input — connection proceeds with
    /// the input as the host.
    #[test]
    fn test_wildcard_single_pattern_matches() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        // Pattern "web-*" should match "web-prod".
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  web-*:\n    host: placeholder\n    user: deploy\n    auth:\n      type: password\n    description: Wildcard web\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        let conn = cfg.find_with_wildcard("web-prod").unwrap();
        // The matched input must replace the host field.
        assert_eq!(conn.host, "web-prod");
        assert_eq!(conn.user, "deploy");
        // The connection name stays as the pattern.
        assert_eq!(conn.name, "web-*");
    }

    /// No exact name and no wildcard pattern matches — error returned.
    #[test]
    fn test_wildcard_no_match_returns_error() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  web-*:\n    host: placeholder\n    user: deploy\n    auth:\n      type: password\n    description: Wildcard web\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        let err = cfg.find_with_wildcard("db-prod").unwrap_err();
        assert!(
            err.to_string().contains("db-prod"),
            "error must name the input: {err}"
        );
    }

    /// Two different wildcard patterns both match the same input — conflict error.
    #[test]
    fn test_wildcard_conflict_two_patterns_same_input() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        // Both "web-*" and "?eb-prod" match "web-prod".
        // Note: bare `*` at the start of a YAML key is an anchor — quote it.
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  web-*:\n    host: ph1\n    user: deploy\n    auth:\n      type: password\n    description: Web wildcard\n  \"?eb-prod\":\n    host: ph2\n    user: admin\n    auth:\n      type: password\n    description: Prefix wildcard\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        let err = cfg.find_with_wildcard("web-prod").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("web-*"),
            "error must name pattern 'web-*': {msg}"
        );
        assert!(
            msg.contains("?eb-prod"),
            "error must name pattern '?eb-prod': {msg}"
        );
    }

    /// Exact name always beats a matching wildcard pattern — no conflict check.
    #[test]
    fn test_wildcard_exact_name_beats_pattern() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        // "web-prod" is an exact entry AND "web-*" would also match.
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  web-prod:\n    host: exact-host\n    user: exact-user\n    auth:\n      type: password\n    description: Exact match\n  web-*:\n    host: wildcard-host\n    user: wildcard-user\n    auth:\n      type: password\n    description: Wildcard\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        let conn = cfg.find_with_wildcard("web-prod").unwrap();
        // Exact match wins — host is NOT replaced by the input.
        assert_eq!(conn.host, "exact-host");
        assert_eq!(conn.user, "exact-user");
        assert_eq!(conn.name, "web-prod");
    }

    /// Same pattern in two layers is shadowing (priority merge), NOT a conflict.
    /// Only the winning (higher-priority) entry should be in the active map, so
    /// `find_with_wildcard` sees exactly one match and succeeds.
    #[test]
    fn test_wildcard_same_pattern_in_two_layers_is_shadowing_not_conflict() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        // Project layer defines "host-*" with user "project-user".
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  host-*:\n    host: proj-host\n    user: project-user\n    auth:\n      type: password\n    description: Project wildcard\n",
        );

        let user = TempDir::new().unwrap();
        // User layer also defines "host-*" — this is shadowed by the project layer.
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  host-*:\n    host: user-host\n    user: user-user\n    auth:\n      type: password\n    description: User wildcard\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(root.path(), Some(user.path()), empty.path());
        // Should succeed: only the project entry is in the active set.
        let conn = cfg.find_with_wildcard("host-anything").unwrap();
        assert_eq!(conn.host, "host-anything", "input must replace host");
        assert_eq!(conn.user, "project-user", "project layer must win");
    }

    /// A `?` wildcard matches exactly one character.
    #[test]
    fn test_wildcard_question_mark_single_char() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        // "web-?" matches "web-1" but not "web-12".
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  web-?:\n    host: placeholder\n    user: deploy\n    auth:\n      type: password\n    description: Single char wildcard\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let conn = cfg.find_with_wildcard("web-1").unwrap();
        assert_eq!(conn.host, "web-1");

        let err = cfg.find_with_wildcard("web-12").unwrap_err();
        assert!(err.to_string().contains("web-12"));
    }

    /// `host: ${name}.corp.com` — the `${name}` token is replaced with the
    /// matched input, producing a FQDN.
    #[test]
    fn test_wildcard_host_with_name_template_is_expanded() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  server*:\n    host: \"${name}.corp.com\"\n    user: deploy\n    auth:\n      type: password\n    description: Corp servers\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let conn = cfg.find_with_wildcard("server01").unwrap();
        assert_eq!(conn.host, "server01.corp.com");
    }

    /// `host: placeholder` (no `${name}`) — entire host is replaced by input,
    /// preserving the original behaviour.
    #[test]
    fn test_wildcard_host_without_name_template_replaced_by_input() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  web-*:\n    host: placeholder\n    user: deploy\n    auth:\n      type: password\n    description: Web servers\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let conn = cfg.find_with_wildcard("web-prod").unwrap();
        assert_eq!(conn.host, "web-prod");
    }

    /// An exact-name entry with `host: ${name}.corp.com` is returned via the
    /// exact-match path — the `${name}` token is NOT expanded.
    #[test]
    fn test_wildcard_exact_match_name_template_not_expanded() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  myconn:\n    host: \"${name}.corp.com\"\n    user: deploy\n    auth:\n      type: password\n    description: My connection\n",
        );
        let empty = TempDir::new().unwrap();

        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let conn = cfg.find_with_wildcard("myconn").unwrap();
        assert_eq!(conn.host, "${name}.corp.com");
    }

    // ─── Numeric range pattern tests ──────────────────────────────────────────

    /// Range matches its lower bound.
    #[test]
    fn test_range_matches_lower_bound() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  \"server[1..10]\":\n    host: placeholder\n    user: deploy\n    auth:\n      type: password\n    description: Range servers\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let conn = cfg.find_with_wildcard("server1").unwrap();
        assert_eq!(conn.host, "server1");
    }

    /// Range matches its upper bound.
    #[test]
    fn test_range_matches_upper_bound() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  \"server[1..10]\":\n    host: placeholder\n    user: deploy\n    auth:\n      type: password\n    description: Range servers\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let conn = cfg.find_with_wildcard("server10").unwrap();
        assert_eq!(conn.host, "server10");
    }

    /// Range matches a midpoint value.
    #[test]
    fn test_range_matches_midpoint() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  \"server[1..10]\":\n    host: placeholder\n    user: deploy\n    auth:\n      type: password\n    description: Range servers\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let conn = cfg.find_with_wildcard("server5").unwrap();
        assert_eq!(conn.host, "server5");
    }

    /// Input outside the range does not match.
    #[test]
    fn test_range_outside_range_does_not_match() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  \"server[1..10]\":\n    host: placeholder\n    user: deploy\n    auth:\n      type: password\n    description: Range servers\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let err = cfg.find_with_wildcard("server11").unwrap_err();
        assert!(err.to_string().contains("server11"));
        let err0 = cfg.find_with_wildcard("server0").unwrap_err();
        assert!(err0.to_string().contains("server0"));
    }

    /// Range pattern conflicts with a glob pattern that matches the same input.
    #[test]
    fn test_range_conflict_with_glob_pattern() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  \"server[1..10]\":\n    host: ph1\n    user: deploy\n    auth:\n      type: password\n    description: Range\n  server*:\n    host: ph2\n    user: admin\n    auth:\n      type: password\n    description: Glob\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let err = cfg.find_with_wildcard("server5").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("server[1..10]"),
            "must name range pattern: {msg}"
        );
        assert!(msg.contains("server*"), "must name glob pattern: {msg}");
    }

    /// Exact name beats a matching range pattern.
    #[test]
    fn test_range_exact_name_beats_matching_range() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  server5:\n    host: exact-host\n    user: exact-user\n    auth:\n      type: password\n    description: Exact\n  \"server[1..10]\":\n    host: range-host\n    user: range-user\n    auth:\n      type: password\n    description: Range\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let conn = cfg.find_with_wildcard("server5").unwrap();
        assert_eq!(conn.host, "exact-host");
        assert_eq!(conn.user, "exact-user");
    }

    /// Same range pattern in two layers is shadowing, not a conflict.
    #[test]
    fn test_range_same_pattern_in_two_layers_is_shadowing_not_conflict() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  \"server[1..10]\":\n    host: proj-host\n    user: project-user\n    auth:\n      type: password\n    description: Project range\n",
        );
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  \"server[1..10]\":\n    host: user-host\n    user: user-user\n    auth:\n      type: password\n    description: User range\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load_test(root.path(), Some(user.path()), empty.path());

        // Project layer wins — only one entry in active set, no conflict.
        let conn = cfg.find_with_wildcard("server5").unwrap();
        assert_eq!(conn.user, "project-user");
    }

    /// Range pattern with `${name}` in host expands to the matched input.
    #[test]
    fn test_range_with_name_template_expands_host() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  \"server[1..10]\":\n    host: \"${name}.corp.com\"\n    user: deploy\n    auth:\n      type: password\n    description: Corp servers\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());

        let conn = cfg.find_with_wildcard("server5").unwrap();
        assert_eq!(conn.host, "server5.corp.com");
    }

    // ── identity auth type ───────────────────────────────────────────────────

    #[test]
    fn test_identity_connection_parsed_correctly() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  github:\n    host: github.com\n    user: git\n    auth:\n      type: identity\n      key: ~/.ssh/github_key\n    description: GitHub identity\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load_test(cwd.path(), Some(user.path()), empty.path());
        assert_eq!(cfg.connections.len(), 1);
        let conn = &cfg.connections[0];
        assert_eq!(conn.name, "github");
        assert_eq!(conn.auth.type_label(), "identity");
        assert_eq!(conn.auth.key(), Some("~/.ssh/github_key"));
    }

    #[test]
    fn test_identity_without_key_rejected() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  github:\n    host: github.com\n    user: git\n    auth:\n      type: identity\n    description: GitHub identity\n",
        );
        let empty = TempDir::new().unwrap();
        let result = load_impl(cwd.path(), None, false, Some(user.path()), empty.path());
        assert!(
            result.is_err(),
            "identity auth without key should be rejected"
        );
    }
}
