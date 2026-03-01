// src/security/mod.rs
// Permission checks, credential field detection.
//
// Validates file permissions on config files and key files. Detects credential
// fields in git-trackable config layers. Warns if `docker` block appears in
// user-level config. All warnings are non-blocking.

use anyhow::Result;

pub fn run() -> Result<()> {
    todo!()
}
