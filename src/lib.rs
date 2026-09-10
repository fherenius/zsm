//! Pure, host-testable logic shared by the `zsm` plugin binary.
//!
//! Anything here is free of Zellij dependencies and is covered by
//! `cargo test --lib --target <host-triple>`.

pub mod config;
pub mod list;
pub mod naming;
pub mod session_name;
pub mod session_usage;
pub mod text;
