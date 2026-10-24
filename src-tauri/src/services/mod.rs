//! Backend services (spec §3). Pure, unit-testable logic lives here.
//!
//! `process.rs` (spec §4) is the only module allowed to spawn a child process;
//! `download.rs` is the only module allowed to reach the network.

pub mod diagnostics;
pub mod download;
pub mod encoders;
pub mod ffmpeg;
pub mod inspect;
pub mod job;
pub mod jsruntime;
pub mod logging;
pub mod manifest;
pub mod models;
pub mod notify;
pub mod paths;
pub mod process;
pub mod queue;
pub mod secrets;
pub mod tools;
pub mod transcribe;
pub mod ytdlp;
