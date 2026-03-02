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
    let renderer = display::Renderer::new(!cli.no_color);

    match cli.command {
        Commands::List => {
            let cfg = load_and_warn(&renderer, verbose)?;
            commands::list::run(&cfg, &renderer, all)
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
            GroupCommands::Use { name } => commands::group::use_group(&name),
            GroupCommands::Clear => commands::group::clear(),
        },
        Commands::Connect { name } => {
            let cfg = load_and_warn(&renderer, verbose)?;
            commands::connect::run(&cfg, &renderer, &name, verbose)
        }
        Commands::Add => commands::add::run(),
        Commands::Edit { name } => commands::edit::run(&name),
        Commands::Remove { name } => commands::remove::run(&name),
        Commands::Init => commands::init::run(),
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
        renderer.verbose(&format!(
            "Active group: {} ({})",
            cfg.group,
            if cfg.group_from_file {
                "from session.yml"
            } else {
                "default"
            }
        ));
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
