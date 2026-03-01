// src/docker/mod.rs
// Container detection, mount resolution, docker invocation.
//
// Handles all Docker-related logic: container detection, building the mount
// list from discovered config file paths and the binary's own path,
// constructing the `docker run` command, and replacing the current process
// with Docker. Completely separate from `connect` — these are two different
// execution paths.

use anyhow::Result;

pub fn run() -> Result<()> {
    todo!()
}
