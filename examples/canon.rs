//! Print the canonical form of an XMIR file, for debugging round-trip failures.

use eo2bin::Xmir;

fn main() {
    let path = std::env::args().nth(1).expect("usage: canon <file.xmir>");
    let text = std::fs::read_to_string(&path).unwrap();
    print!("{}", Xmir::parse(&text).unwrap().print());
}
