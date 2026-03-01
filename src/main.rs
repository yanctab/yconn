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

    match cli.command {
        Commands::List => commands::list::run(),
        Commands::Connect { name } => commands::connect::run(&name),
        Commands::Show { name } => commands::show::run(&name),
        Commands::Add => commands::add::run(),
        Commands::Edit { name } => commands::edit::run(&name),
        Commands::Remove { name } => commands::remove::run(&name),
        Commands::Init => commands::init::run(),
        Commands::Config => commands::config::run(),
        Commands::Group { subcommand } => match subcommand {
            GroupCommands::List => commands::group::list(),
            GroupCommands::Use { name } => commands::group::use_group(&name),
            GroupCommands::Clear => commands::group::clear(),
            GroupCommands::Current => commands::group::current(),
        },
    }
}
