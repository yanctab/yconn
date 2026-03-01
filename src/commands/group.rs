// src/commands/group.rs
// Handlers for `yconn group` subcommands:
//   list    — show all groups found across all layers
//   use     — set the active group (persisted to ~/.config/yconn/session.yml)
//   clear   — remove active_group from session.yml, revert to default
//   current — print the active group name and resolved config file paths

use anyhow::Result;

pub fn list() -> Result<()> {
    todo!()
}

pub fn use_group(_name: &str) -> Result<()> {
    todo!()
}

pub fn clear() -> Result<()> {
    todo!()
}

pub fn current() -> Result<()> {
    todo!()
}
