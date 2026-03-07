use anyhow::Result;
use clap::Parser;

mod cli;
mod commands;
mod config;
mod connect;
mod display;
mod docker;
mod group;
mod security;

use cli::{Cli, Commands, GroupCommands};

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Save Copy flags before cli.command is moved in the match below.
    let all = cli.all;
    let verbose = cli.verbose;
    let renderer = display::Renderer::new(true);

    match cli.command {
        Commands::List { group } => {
            let cfg = load_and_warn(&renderer, verbose)?;
            commands::list::run(&cfg, &renderer, all, group.as_deref())
        }
        Commands::Show { name } => {
            let cfg = load_and_warn(&renderer, verbose)?;
            commands::show::run(&cfg, &renderer, &name)
        }
        Commands::Config => {
            let cfg = load_and_warn(&renderer, verbose)?;
            commands::config::run(&cfg, &renderer)
        }
        Commands::Group { subcommand } => match subcommand {
            GroupCommands::List => {
                let cfg = load_and_warn(&renderer, verbose)?;
                commands::group::list(&cfg, &renderer)
            }
            GroupCommands::Current => {
                let cfg = load_and_warn(&renderer, verbose)?;
                commands::group::current(&cfg, &renderer)
            }
            GroupCommands::Use { name } => {
                let cfg = load_and_warn(&renderer, verbose)?;
                commands::group::use_group(&name, &cfg, &renderer)
            }
            GroupCommands::Clear => commands::group::clear(),
        },
        Commands::Connect { name } => {
            let cfg = load_and_warn(&renderer, verbose)?;
            commands::connect::run(&cfg, &renderer, &name, verbose)
        }
        Commands::Add { layer } => commands::add::run(layer),
        Commands::Edit { name, layer } => {
            let cfg = load_and_warn(&renderer, verbose)?;
            commands::edit::run(&cfg, &name, layer)
        }
        Commands::Remove { name, layer } => {
            let cfg = load_and_warn(&renderer, verbose)?;
            commands::remove::run(&cfg, &renderer, &name, layer)
        }
        Commands::Init { location } => commands::init::run(location),
        Commands::SshConfig { dry_run } => {
            let cfg = load_and_warn(&renderer, verbose)?;
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
            commands::ssh_config::run_generate(&cfg.connections, &renderer, dry_run, &home)
        }
    }
}

/// Load config and surface any security warnings through the renderer.
///
/// When `--verbose` is set, also prints a brief summary of the active group
/// and which layer files were found.
fn load_and_warn(renderer: &display::Renderer, verbose: bool) -> Result<config::LoadedConfig> {
    let cfg = config::load()?;
    for w in &cfg.warnings {
        renderer.warn(&w.message);
    }
    if verbose {
        let group_desc = match &cfg.group {
            Some(g) if cfg.group_from_file => format!("{g} (from session.yml)"),
            Some(g) => format!("{g} (default)"),
            None => "none (showing all connections)".to_string(),
        };
        renderer.verbose(&format!("Active group: {group_desc}"));
        for l in &cfg.layers {
            renderer.verbose(&format!(
                "[{}] {} — {}",
                l.layer.label(),
                l.path.display(),
                l.connection_count
                    .map_or("not found".to_string(), |n| format!("{n} connections"))
            ));
        }
    }
    Ok(cfg)
}
