//! A devirtualizer for EO programs, working on XMIR in and XMIR out.

mod program;
mod xmir;

pub use program::{Program, Report};
pub use xmir::Xmir;
