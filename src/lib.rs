//! Pure, host-testable logic shared by the `zsm` plugin binary.
//!
//! The plugin binary itself links `zellij-tile`, whose host imports are
//! undefined off `wasm32-wasip1`, so it cannot be linked into a test harness.
//! Anything here is free of Zellij dependencies and is covered by
//! `cargo test --lib --target <host-triple>`.

pub mod list;
pub mod session_name;
pub mod text;
