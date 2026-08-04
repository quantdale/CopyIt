//! In-process user simulation: drives the real CopyIt UI through complete user
//! journeys headlessly (and, via the `sim` feature, in the real window). Compiled
//! only under `cfg(any(test, feature = "sim"))`.

#[cfg(feature = "sim")]
pub mod cli;
pub mod harness;
pub mod journey;
pub mod persona;
pub mod report;
