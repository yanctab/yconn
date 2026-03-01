// src/group/mod.rs
// Active group resolution, session.yml read/write.
//
// Reads and writes ~/.config/yconn/session.yml. Resolves the active group
// name (defaulting to "connections" when the file is absent). Scans all
// layer directories to discover which groups have config files, used by
// `yconn group list`.

use anyhow::Result;

#[allow(dead_code)]
pub fn run() -> Result<()> {
    todo!()
}
