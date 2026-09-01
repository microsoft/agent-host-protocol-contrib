//! A black-box conformance runner for the protocol slice in this repository.
//!
//! The runner deliberately uses raw JSON-RPC over WebSocket. It therefore
//! verifies observable wire behavior and can test another AHP implementation,
//! not only this sample host.

#![forbid(unsafe_code)]

mod runner;

pub use runner::{run_local_suite, run_suite, ConformanceError, OutputFormat, Report, RunOptions};
