//! Embedded Python adapter for an `FftApp` lease.
//!
//! The adapter receives a scoped lease for each run and drives the app through
//! it. Products may share one adapter-lifetime lease across sequential runs;
//! the adapter never mints leases. It does not own an app implementation or
//! construct a runtime.
//!
//! The optional `python` feature builds the CPython binding (PyO3 with a
//! linked libpython, not an extension module). Keep it opt-in: products decide
//! when to ship the interpreter.

#![allow(unused_crate_dependencies)] // core deps are used by the optional engine

#[cfg(feature = "python")]
pub mod python;
#[cfg(feature = "python")]
mod views;

#[cfg(feature = "python")]
pub use python::PythonScript;

#[cfg(test)]
mod tests;
