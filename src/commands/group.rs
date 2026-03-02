// Handlers for `yconn group` subcommands:
//   list    — show all groups found across all layers
//   use     — set the active group (persisted to ~/.config/yconn/session.yml)
//   clear   — remove active_group from session.yml, revert to default
//   current — print the active group name and its resolved config file paths

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::LoadedConfig;
use crate::display::{GroupCurrentStatus, GroupRow, LayerCurrentInfo, Renderer};

pub fn list(cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let user_dir = dirs::config_dir().map(|d| d.join("yconn"));
    let system_dir = PathBuf::from("/etc/yconn");

    // Build the list of (dir, label) pairs in priority order.
    let mut dirs: Vec<(PathBuf, &str)> = Vec::new();
    if let Some(ref pd) = cfg.project_dir {
        dirs.push((pd.clone(), "project"));
    }
    if let Some(ref ud) = user_dir {
        dirs.push((ud.clone(), "user"));
    }
    dirs.push((system_dir, "system"));

    let dir_refs: Vec<(&Path, &str)> = dirs.iter().map(|(p, l)| (p.as_path(), *l)).collect();
    let groups = crate::group::discover_groups(&dir_refs)?;

    let rows: Vec<GroupRow> = groups
        .iter()
        .map(|g| GroupRow {
            name: g.name.clone(),
            layers: g.layers.clone(),
        })
        .collect();

    renderer.group_list(&rows);
    Ok(())
}

pub fn current(cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let session_file = dirs::config_dir()
        .map(|d| d.join("yconn").join("session.yml").display().to_string())
        .unwrap_or_else(|| "~/.config/yconn/session.yml".to_string());

    let layers: Vec<LayerCurrentInfo> = cfg
        .layers
        .iter()
        .map(|l| LayerCurrentInfo {
            label: l.layer.label().to_string(),
            path: l.path.display().to_string(),
            found: l.connection_count.is_some(),
        })
        .collect();

    let status = GroupCurrentStatus {
        active_group: cfg.group.clone(),
        session_file,
        layers,
    };

    renderer.group_current(&status);
    Ok(())
}

pub fn use_group(_name: &str) -> Result<()> {
    todo!()
}

pub fn clear() -> Result<()> {
    todo!()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    use crate::config;
    use crate::display::Renderer;

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
        config::load_impl(cwd, "connections", false, user, sys).unwrap()
    }

    // ── group list ────────────────────────────────────────────────────────────

    #[test]
    fn test_group_list_no_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(&yconn, "connections.yaml", "connections: {}\n");

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        // project_dir is set when .yconn/connections.yaml exists
        assert!(cfg.project_dir.is_some());
        list(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_list_no_project_dir_no_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        assert!(cfg.project_dir.is_none());
        list(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_list_groups_discovered_from_project() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(&yconn, "connections.yaml", "connections: {}\n");
        write_yaml(&yconn, "work.yaml", "connections: {}\n");

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        // Discover manually to verify the expected groups exist.
        let dirs = [(yconn.as_path(), "project")];
        let groups = crate::group::discover_groups(&dirs).unwrap();
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"connections"));
        assert!(names.contains(&"work"));

        list(&cfg, &no_color()).unwrap();
    }

    // ── group current ─────────────────────────────────────────────────────────

    #[test]
    fn test_group_current_default_group_no_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        assert_eq!(cfg.group, "connections");
        current(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_current_active_group_from_file_no_error() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "work.yaml",
            "connections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n",
        );
        let empty = TempDir::new().unwrap();
        // Explicitly request the "work" group (as if session.yml said so)
        let cfg =
            config::load_impl(cwd.path(), "work", true, Some(user.path()), empty.path()).unwrap();
        assert_eq!(cfg.group, "work");
        assert!(cfg.group_from_file);
        current(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_current_layer_found_status() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        // Project not found, user found, system not found.
        assert_eq!(cfg.layers[0].connection_count, None);
        assert!(cfg.layers[1].connection_count.is_some());
        assert_eq!(cfg.layers[2].connection_count, None);
        current(&cfg, &no_color()).unwrap();
    }
}
