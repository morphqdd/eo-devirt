//! A devirtualizer for EO programs, working on XMIR in and XMIR out.

mod compile;
mod program;
mod xmir;

pub use program::{Program, Report};
pub use xmir::Xmir;
