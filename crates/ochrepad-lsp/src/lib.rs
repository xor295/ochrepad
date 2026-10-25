//! The language server client (PLAN_completion L1): servers' processes,
//! the base protocol, document sync and requests. No egui; the app polls
//! each [`Client`] once a frame.

pub mod client;
#[cfg(test)]
mod client_tests;
pub mod config;
pub mod framing;
pub mod message;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod transport;
pub mod uri;

pub use client::{Client, Event, State, Tag};
pub use lsp_types;
