// src/config/mod.rs
// Layer loading, upward walk, merge logic.
//
// Loads each config layer (project, user, system) in priority order,
// performs the upward directory walk for project config, merges layers
// into a flat connection map with source tracking, and retains shadowed
// entries for `--all` display. Surfaces the resolved `docker` block if present.

use anyhow::Result;

#[allow(dead_code)]
pub fn load() -> Result<()> {
    todo!()
}
