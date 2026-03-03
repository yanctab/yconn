// src/commands/remove.rs
// Handler for `yconn remove <name>` — remove a connection, prompting for
// layer if ambiguous.

use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::config::{Layer, LoadedConfig};
use crate::display::Renderer;

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn run(cfg: &LoadedConfig, renderer: &Renderer, name: &str, layer: Option<&str>) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_impl(
        cfg,
        renderer,
        name,
        layer,
        &mut stdin.lock(),
        &mut stdout.lock(),
    )
}

// ─── Testable impl ────────────────────────────────────────────────────────────

pub(crate) fn run_impl(
    cfg: &LoadedConfig,
    _renderer: &Renderer,
    name: &str,
    layer: Option<&str>,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<()> {
    // Collect all occurrences of the named connection across all layers.
    let all: Vec<_> = cfg
        .all_connections
        .iter()
        .filter(|c| c.name == name)
        .collect();

    if all.is_empty() {
        bail!("no connection named '{name}'");
    }

    // If --layer is given, restrict to that layer only.
    let target = if let Some(layer_str) = layer {
        let target_layer = parse_layer(layer_str)?;
        all.iter()
            .find(|c| c.layer == target_layer)
            .copied()
            .ok_or_else(|| {
                anyhow!(
                    "no connection named '{name}' in the {} layer",
                    target_layer.label()
                )
            })?
    } else if all.len() == 1 {
        all[0]
    } else {
        // Ambiguous: multiple layers define this name — ask the user.
        prompt_layer_choice(name, &all, input, output)?
    };

    remove_from_file(&target.source_path, name)?;

    writeln!(
        output,
        "Removed '{name}' from {}",
        target.source_path.display()
    )?;

    Ok(())
}

// ─── Layer parsing ────────────────────────────────────────────────────────────

fn parse_layer(s: &str) -> Result<Layer> {
    match s {
        "project" => Ok(Layer::Project),
        "user" => Ok(Layer::User),
        "system" => Ok(Layer::System),
        other => bail!("unknown layer '{other}'; use system, user, or project"),
    }
}

// ─── Ambiguity prompt ─────────────────────────────────────────────────────────

fn prompt_layer_choice<'a>(
    name: &str,
    options: &[&'a crate::config::Connection],
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<&'a crate::config::Connection> {
    writeln!(
        output,
        "'{name}' exists in multiple layers. Which one do you want to remove?"
    )?;
    for (i, c) in options.iter().enumerate() {
        writeln!(
            output,
            "  [{}] {} ({})",
            i + 1,
            c.layer.label(),
            c.source_path.display()
        )?;
    }

    loop {
        write!(output, "  Enter number [1-{}]: ", options.len())?;
        output.flush()?;

        let mut line = String::new();
        input.read_line(&mut line)?;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            bail!("aborted");
        }

        match trimmed.parse::<usize>() {
            Ok(n) if n >= 1 && n <= options.len() => return Ok(options[n - 1]),
            _ => writeln!(
                output,
                "  Please enter a number between 1 and {}",
                options.len()
            )?,
        }
    }
}

// ─── YAML surgery ────────────────────────────────────────────────────────────

/// Remove the named connection block from the YAML file at `path`.
///
/// The strategy: collect all lines; find the `  <name>:` key line; remove it
/// and all subsequent lines that are indented more deeply (i.e. belong to the
/// same mapping value), stopping at the next same-or-lower-indentation line.
fn remove_from_file(path: &Path, name: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let updated = remove_entry(&content, name)
        .ok_or_else(|| anyhow!("connection '{name}' not found in {}", path.display()))?;

    std::fs::write(path, updated).with_context(|| format!("failed to write {}", path.display()))?;

    Ok(())
}

/// Remove the `  <name>:` block from YAML text, returning the updated string,
/// or `None` if the key was not found.
pub(crate) fn remove_entry(content: &str, name: &str) -> Option<String> {
    let key_line = format!("  {name}:");
    let lines: Vec<&str> = content.lines().collect();

    // Find the index of the key line.
    let start = lines
        .iter()
        .position(|l| *l == key_line || l.starts_with(&format!("{key_line} ")))?;

    // Find the end: the first subsequent line that is NOT more deeply indented
    // than the key (i.e. indentation <= 2 spaces, or empty line followed by
    // something at <= 2 spaces).
    let end = lines[start + 1..]
        .iter()
        .position(|l| {
            if l.is_empty() {
                false // blank lines inside the block are fine
            } else {
                // Count leading spaces.
                let indent = l.len() - l.trim_start().len();
                indent <= 2
            }
        })
        .map(|rel| start + 1 + rel)
        .unwrap_or(lines.len());

    // Also trim any trailing blank lines immediately before `end` that belong
    // to the removed block.
    let mut real_end = end;
    while real_end > start + 1 && lines[real_end - 1].trim().is_empty() {
        real_end -= 1;
    }

    let mut result: Vec<&str> = Vec::new();
    result.extend_from_slice(&lines[..start]);
    result.extend_from_slice(&lines[real_end..]);

    // Preserve the original trailing newline behaviour.
    let trailing_newline = content.ends_with('\n');
    let joined = result.join("\n");
    Some(if trailing_newline {
        format!("{joined}\n")
    } else {
        joined
    })
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

    fn run_with_input(
        cfg: &config::LoadedConfig,
        name: &str,
        layer: Option<&str>,
        answers: &[&str],
    ) -> Result<String> {
        let input_str = answers.join("\n") + "\n";
        let mut input = input_str.as_bytes();
        let mut output = Vec::new();
        run_impl(cfg, &no_color(), name, layer, &mut input, &mut output)?;
        Ok(String::from_utf8(output).unwrap())
    }

    // ── remove_entry helper ───────────────────────────────────────────────────

    #[test]
    fn test_remove_entry_single_connection() {
        let content = "version: 1\n\nconnections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n";
        let result = remove_entry(content, "srv").unwrap();
        assert!(!result.contains("srv:"));
        assert!(result.contains("connections:"));
    }

    #[test]
    fn test_remove_entry_leaves_other_connections() {
        let content = "connections:\n  alpha:\n    host: a\n    user: u\n    auth: key\n    description: d\n  beta:\n    host: b\n    user: u\n    auth: key\n    description: d\n";
        let result = remove_entry(content, "alpha").unwrap();
        assert!(!result.contains("alpha:"));
        assert!(result.contains("beta:"));
    }

    #[test]
    fn test_remove_entry_returns_none_when_not_found() {
        let content = "connections:\n  other:\n    host: h\n";
        assert!(remove_entry(content, "missing").is_none());
    }

    // ── run_impl: basic remove ─────────────────────────────────────────────────

    #[test]
    fn test_remove_single_layer_no_prompt() {
        let dir = TempDir::new().unwrap();
        let yconn = dir.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "version: 1\n\nconnections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n",
        );
        let sys = TempDir::new().unwrap();
        let cfg = load(dir.path(), None, sys.path());
        // Confirm srv exists.
        assert!(cfg.find("srv").is_some());

        // Run remove with no layer flag and no interactive prompt needed (only one match).
        run_with_input(&cfg, "srv", None, &[]).unwrap();

        // Verify the file was updated.
        let content = fs::read_to_string(yconn.join("connections.yaml")).unwrap();
        assert!(!content.contains("  srv:"));
    }

    #[test]
    fn test_remove_unknown_name_returns_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());

        let err = run_with_input(&cfg, "no-such", None, &[]).unwrap_err();
        assert!(err.to_string().contains("no-such"));
    }

    // ── --layer flag ──────────────────────────────────────────────────────────

    #[test]
    fn test_remove_with_layer_flag_targets_correct_layer() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  srv:\n    host: proj\n    user: u\n    auth: key\n    description: d\n",
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            "connections:\n  srv:\n    host: sys\n    user: u\n    auth: key\n    description: d\n",
        );

        let cfg = load(root.path(), None, sys.path());

        // Remove from system layer only.
        run_with_input(&cfg, "srv", Some("system"), &[]).unwrap();

        // Project file unchanged.
        let proj_content = fs::read_to_string(yconn.join("connections.yaml")).unwrap();
        assert!(proj_content.contains("srv:"));

        // System file updated.
        let sys_content = fs::read_to_string(sys.path().join("connections.yaml")).unwrap();
        assert!(!sys_content.contains("  srv:"));
    }

    #[test]
    fn test_remove_unknown_layer_returns_error() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        let err = run_with_input(&cfg, "srv", Some("bogus"), &[]).unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    // ── ambiguous prompt ──────────────────────────────────────────────────────

    #[test]
    fn test_remove_ambiguous_prompts_user_and_removes_chosen() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  srv:\n    host: proj\n    user: u\n    auth: key\n    description: d\n",
        );

        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            "connections:\n  srv:\n    host: sys\n    user: u\n    auth: key\n    description: d\n",
        );

        let cfg = load(root.path(), None, sys.path());
        // Two occurrences → should prompt. Choose option 1 (project layer).
        run_with_input(&cfg, "srv", None, &["1"]).unwrap();

        // The project file should have srv removed.
        let proj_content = fs::read_to_string(yconn.join("connections.yaml")).unwrap();
        assert!(!proj_content.contains("  srv:"));

        // The system file should be untouched.
        let sys_content = fs::read_to_string(sys.path().join("connections.yaml")).unwrap();
        assert!(sys_content.contains("srv:"));
    }

    #[test]
    fn test_remove_ambiguous_empty_input_aborts() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(
            &yconn,
            "connections.yaml",
            "connections:\n  srv:\n    host: proj\n    user: u\n    auth: key\n    description: d\n",
        );
        let sys = TempDir::new().unwrap();
        write_yaml(
            sys.path(),
            "connections.yaml",
            "connections:\n  srv:\n    host: sys\n    user: u\n    auth: key\n    description: d\n",
        );
        let cfg = load(root.path(), None, sys.path());

        let err = run_with_input(&cfg, "srv", None, &[""]).unwrap_err();
        assert!(err.to_string().contains("aborted"));
    }
}
