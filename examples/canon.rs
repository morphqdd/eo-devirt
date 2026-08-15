#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "a tool run by hand reports to whoever ran it, and stops on anything it cannot do"
)]

//! Print the canonical form of an XMIR file, for debugging round-trip failures.

use eo2bin::Xmir;

fn main() {
    let path = std::env::args().nth(1).expect("usage: canon <file.xmir>");
    let text = std::fs::read_to_string(&path).unwrap();
    print!("{}", Xmir::parse(&text).unwrap().print());
}
