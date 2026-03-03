// Handler for `yconn config` — show which config files are active, their
// paths, and Docker status.

use anyhow::Result;

use crate::config::LoadedConfig;
use crate::display::{ConfigStatus, DockerInfo, LayerInfo, Renderer};
use crate::docker;

pub fn run(cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let layers: Vec<LayerInfo> = cfg
        .layers
        .iter()
        .map(|l| LayerInfo {
            label: l.layer.label().to_string(),
            path: l.path.display().to_string(),
            connection_count: l.connection_count,
        })
        .collect();

    let docker = cfg.docker.as_ref().map(|d| DockerInfo {
        image: d.image.clone(),
        pull: d.pull.clone(),
        source: d.layer.label().to_string(),
        will_bootstrap: !docker::in_container(),
    });

    let status = ConfigStatus {
        group: cfg.group.clone(),
        group_from_file: cfg.group_from_file,
        layers,
        docker,
    };

    renderer.config_status(&status);
    Ok(())
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
        config::load_impl(cwd, Some("connections"), false, user, sys).unwrap()
    }

    #[test]
    fn test_config_with_project_layer_no_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n",
        );

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        run(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_config_no_layers_found_no_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());

        // All layers missing — should still render without error.
        assert!(cfg.layers.iter().all(|l| l.connection_count.is_none()));
        run(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_config_with_docker_block_no_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "docker:\n  image: ghcr.io/org/keys:latest\nconnections: {}\n",
        );

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        assert!(cfg.docker.is_some());
        run(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_config_layer_status_correct() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  a:\n    host: h\n    user: u\n    auth: key\n    description: d\n  b:\n    host: h2\n    user: u2\n    auth: key\n    description: d2\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        // Project not found, user has 2, system not found.
        assert_eq!(cfg.layers[0].connection_count, None);
        assert_eq!(cfg.layers[1].connection_count, Some(2));
        assert_eq!(cfg.layers[2].connection_count, None);
        run(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_config_group_from_file_false_for_default() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        // load_impl is called with group_from_file=false
        let cfg = load(cwd.path(), None, empty.path());
        assert!(!cfg.group_from_file);
        run(&cfg, &no_color()).unwrap();
    }
}
