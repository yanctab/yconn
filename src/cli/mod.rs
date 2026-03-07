// src/cli/mod.rs
// Entry point, command definitions, flag parsing.
//
// Parses commands and flags, delegates entirely to other modules.
// No business logic lives here.

use clap::{Parser, Subcommand, ValueEnum};

/// Which config layer to target for add / edit / remove.
///
/// - `system`  → `/etc/yconn/connections.yaml`
/// - `user`    → `~/.config/yconn/connections.yaml` (default when omitted)
/// - `project` → `.yconn/connections.yaml` in the current directory tree
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LayerArg {
    /// System-wide layer: `/etc/yconn/connections.yaml`
    System,
    /// Per-user layer: `~/.config/yconn/connections.yaml` (default)
    User,
    /// Project layer: `.yconn/connections.yaml` in the current directory tree
    Project,
}

/// yconn — SSH connection manager
#[derive(Debug, Parser)]
#[command(name = "yconn", version, about)]
pub struct Cli {
    /// Include shadowed entries in list output
    #[arg(long, global = true)]
    pub all: bool,

    /// Print config loading decisions, merge resolution, and full Docker invocation
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// Where `yconn init` places the scaffolded config file.
///
/// - `yconn` (default): `.yconn/connections.yaml` — recommended, git-trackable
/// - `dotfile`:         `.connections.yaml` in the current directory
/// - `plain`:           `connections.yaml` in the current directory (may clash with other tools)
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InitLocation {
    /// Scaffold `.yconn/connections.yaml` (default, backward compatible)
    Yconn,
    /// Scaffold `.connections.yaml` in the current directory
    Dotfile,
    /// Scaffold `connections.yaml` in the current directory
    Plain,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// List all connections across all layers
    List {
        /// Filter output to connections whose group field equals NAME
        #[arg(long, value_name = "NAME")]
        group: Option<String>,
    },

    /// Connect to a named host
    Connect {
        /// Name of the connection to open
        name: String,
        /// Override or add a users: map entry for this invocation (repeatable).
        /// Format: key:value. Use --user user:<name> to override ${user} expansion.
        #[arg(long = "user", value_name = "KEY:VALUE", action = clap::ArgAction::Append)]
        user_overrides: Vec<String>,
    },

    /// Show the resolved config for a connection (no secrets printed)
    Show {
        /// Name of the connection to inspect. Required unless --dump is set.
        #[arg(required_unless_present = "dump", conflicts_with = "dump")]
        name: Option<String>,
        /// Print the fully merged connections: and users: maps as YAML.
        /// Mutually exclusive with providing a connection name.
        #[arg(long, conflicts_with = "name")]
        dump: bool,
    },

    /// Interactive wizard to add a connection to a chosen layer
    Add {
        /// Target a specific config layer
        #[arg(long, value_name = "LAYER")]
        layer: Option<LayerArg>,
    },

    /// Open the connection's source config file in $EDITOR
    Edit {
        /// Name of the connection to edit
        name: String,
        /// Target a specific config layer
        #[arg(long, value_name = "LAYER")]
        layer: Option<LayerArg>,
    },

    /// Remove a connection (prompts for layer if ambiguous)
    Remove {
        /// Name of the connection to remove
        name: String,
        /// Target a specific config layer
        #[arg(long, value_name = "LAYER")]
        layer: Option<LayerArg>,
    },

    /// Scaffold a connections.yaml in the current directory
    Init {
        /// Where to place the scaffolded config file.
        ///
        /// yconn  → .yconn/connections.yaml (default, git-trackable, recommended)
        ///
        /// dotfile → .connections.yaml in cwd
        ///
        /// plain   → connections.yaml in cwd (may clash with other tools)
        #[arg(long, value_enum, default_value = "yconn")]
        location: InitLocation,
    },

    /// Show which config files are active, their paths, and Docker status
    Config,

    /// Manage the active connection group
    Group {
        #[command(subcommand)]
        subcommand: GroupCommands,
    },

    /// Write Host blocks to ~/.ssh/yconn-connections and update ~/.ssh/config
    SshConfig {
        /// Print generated config to stdout without writing any files
        #[arg(long)]
        dry_run: bool,
        /// Override or add a users: map entry for this invocation (repeatable).
        /// Format: key:value. Mutually exclusive with --skip-user.
        #[arg(long = "user", value_name = "KEY:VALUE", action = clap::ArgAction::Append, conflicts_with = "skip_user")]
        user_overrides: Vec<String>,
        /// Omit User lines from all generated Host blocks.
        /// Mutually exclusive with --user.
        #[arg(long, conflicts_with = "user_overrides")]
        skip_user: bool,
    },

    /// Manage user key/value entries in the users: config section
    Users {
        #[command(subcommand)]
        subcommand: UserCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum UserCommands {
    /// List all user entries across all layers (with shadowing info)
    Show,

    /// Interactive wizard to add a user entry to a chosen layer
    Add {
        /// Target a specific config layer
        #[arg(long, value_name = "LAYER")]
        layer: Option<LayerArg>,
    },

    /// Open the source config file for a named user entry in $EDITOR
    Edit {
        /// Key of the user entry to edit
        key: String,
        /// Target a specific config layer
        #[arg(long, value_name = "LAYER")]
        layer: Option<LayerArg>,
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
