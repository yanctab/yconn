// src/connect/mod.rs
// SSH argument construction and process invocation.
//
// Takes a resolved connection entry and builds the SSH invocation arguments.
// Executes SSH by replacing the current process so terminal behaviour works
// correctly. For auth: password, the native SSH password prompt is used —
// no password is ever passed programmatically.

use anyhow::Result;

#[allow(dead_code)]
pub fn run() -> Result<()> {
    todo!()
}
