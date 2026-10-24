//! Everything a Rust front end needs to talk to the fleet-board daemon, with no UI types.
pub mod schema;
pub mod board;
pub mod sse;
pub mod http;
pub mod stream;
pub mod config;
pub mod daemon_control;
pub mod close;
