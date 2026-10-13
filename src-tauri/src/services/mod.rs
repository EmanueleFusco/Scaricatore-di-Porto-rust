//! Backend services (spec §3). Pure, unit-testable logic lives here.
//!
//! `process.rs` (spec §4) is the only module allowed to spawn a child process;
//! the rest of M0 (tools manifest, queue, yt-dlp) arrives in the next steps.

pub mod paths;
pub mod process;
