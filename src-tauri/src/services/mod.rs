//! Backend services (spec §3). Pure, unit-testable logic lives here.
//!
//! `process.rs` (spec §4) is the only module allowed to spawn a child process;
//! `download.rs` is the only module allowed to reach the network.

pub mod manifest;
pub mod paths;
pub mod process;
