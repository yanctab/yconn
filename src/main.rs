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

use cli::{
    Cli, Commands, ConnectionCommands, GroupCommands, SshConfigArgs, SshConfigCommands,
    UserCommands,
};

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
        Commands::Connections { subcommand } => match subcommand {
            ConnectionCommands::Show { name, dump } => {
                let cfg = load_and_warn(&renderer, verbose)?;
                if dump {
                    commands::show::run_dump(&cfg, &renderer)
                } else {
                    commands::show::run(
                        &cfg,
                        &renderer,
                        &name.expect("name is required when --dump is not set"),
                    )
                }
            }
            ConnectionCommands::Add { layer } => commands::add::run(layer),
            ConnectionCommands::Edit { name, layer } => {
                let cfg = load_and_warn(&renderer, verbose)?;
                commands::edit::run(&cfg, &name, layer)
            }
            ConnectionCommands::Remove { name, layer } => {
                let cfg = load_and_warn(&renderer, verbose)?;
                commands::remove::run(&cfg, &renderer, &name, layer)
            }
            ConnectionCommands::Init { location } => commands::init::run(location),
        },
        Commands::Config => {
            let cfg = load_and_warn(&renderer, verbose)?;
            commands::config::run(&cfg, &renderer)
        }
        Commands::Groups { subcommand } => match subcommand {
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
        Commands::Connect {
            name,
            user_overrides,
        } => {
            let cfg = load_and_warn(&renderer, verbose)?;
            let overrides = parse_user_overrides(&user_overrides)?;
            commands::connect::run(&cfg, &renderer, &name, verbose, &overrides)
        }
        Commands::SshConfig(SshConfigArgs { subcommand }) => {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
            match subcommand {
                SshConfigCommands::Install {
                    dry_run,
                    user_overrides,
                    skip_user,
                } => {
                    let cfg = load_and_warn(&renderer, verbose)?;
                    let overrides = parse_user_overrides(&user_overrides)?;
                    commands::ssh_config::run_install(
                        &cfg, &renderer, dry_run, &home, &overrides, skip_user,
                    )
                }
                SshConfigCommands::Print {
                    user_overrides,
                    skip_user,
                } => {
                    let cfg = load_and_warn(&renderer, verbose)?;
                    let overrides = parse_user_overrides(&user_overrides)?;
                    commands::ssh_config::run_print(&cfg, &renderer, &home, &overrides, skip_user)
                }
                SshConfigCommands::Uninstall => commands::ssh_config::run_uninstall(&home),
                SshConfigCommands::Disable => commands::ssh_config::run_disable(&home),
                SshConfigCommands::Enable => commands::ssh_config::run_enable(&home),
            }
        }
        Commands::Users { subcommand } => match subcommand {
            UserCommands::Show => {
                let cfg = load_and_warn(&renderer, verbose)?;
                commands::user::show(&cfg, &renderer)
            }
            UserCommands::Add { layer, user_pairs } => commands::user::add(layer, user_pairs),
            UserCommands::Edit { key, layer } => {
                let cfg = load_and_warn(&renderer, verbose)?;
                commands::user::edit(&cfg, &key, layer)
            }
        },
    }
}

/// Parse `--user key:value` CLI strings into a `HashMap<String, String>`.
///
/// Each element must contain exactly one `:` separator. If any element is
/// malformed, returns an error with a clear message.
fn parse_user_overrides(
    overrides: &[String],
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut map = std::collections::HashMap::new();
    for s in overrides {
        match s.split_once(':') {
            Some((key, value)) => {
                map.insert(key.to_string(), value.to_string());
            }
            None => {
                anyhow::bail!("--user value '{}' is invalid: expected format KEY:VALUE", s);
            }
        }
    }
    Ok(map)
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
