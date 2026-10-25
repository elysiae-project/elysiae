pub mod cache;
pub mod logging;
pub mod notifications;
pub mod progress;
pub mod runtime;
pub mod settings;
pub mod web;

// Test the exact parser used by build.rs; build-script tests are not run by Cargo.
#[cfg(test)]
#[path = "../../build.rs"]
mod build_checks;
