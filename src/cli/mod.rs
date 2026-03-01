// src/cli/mod.rs
// Entry point, command definitions, flag parsing.
//
// Parses commands and flags, delegates entirely to other modules.
// No business logic lives here.

use clap::{Parser, Subcommand};

/// yconn — SSH connection manager
#[derive(Debug, Parser)]
#[command(name = "yconn", version, about)]
pub struct Cli {
    /// Target a specific config layer for add / edit / remove
    #[arg(long, global = true, value_name = "LAYER")]
    pub layer: Option<String>,

    /// Include shadowed entries in list output
    #[arg(long, global = true)]
    pub all: bool,

    /// Disable coloured output
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Print config loading decisions, merge resolution, and full Docker invocation
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// List all connections across all layers
    List,

    /// Connect to a named host
    Connect {
        /// Name of the connection to open
        name: String,
    },

    /// Show the resolved config for a connection (no secrets printed)
    Show {
        /// Name of the connection to inspect
        name: String,
    },

    /// Interactive wizard to add a connection to a chosen layer
    Add,

    /// Open the connection's source config file in $EDITOR
    Edit {
        /// Name of the connection to edit
        name: String,
    },

    /// Remove a connection (prompts for layer if ambiguous)
    Remove {
        /// Name of the connection to remove
        name: String,
    },

    /// Scaffold a <group>.yaml in .yconn/ in the current directory
    Init,

    /// Show which config files are active, their paths, and Docker status
    Config,

    /// Manage the active connection group
    Group {
        #[command(subcommand)]
        subcommand: GroupCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum GroupCommands {
    /// Show all groups found across all layers
    List,

    /// Set the active group (persisted to ~/.config/yconn/session.yml)
    Use {
        /// Group name to activate
        name: String,
    },

    /// Remove active_group from session.yml, reverting to default (connections)
    Clear,

    /// Print the active group name and resolved config file paths
    Current,
}
