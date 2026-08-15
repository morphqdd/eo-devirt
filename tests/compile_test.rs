use eo_devirt::{Program, Xmir};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// The arithmetic slice: an expression the resolver pinned down completely,
/// turned into machine code and run. `p1` is `(2.plus 3).plus 4`.
#[test]
fn compiles_constant_arithmetic_into_a_binary_that_exits_with_the_result() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let documents: Vec<Xmir> = ["p1", "number", "bytes"]
        .iter()
        .map(|name| {
            let text = fs::read_to_string(fixtures.join(format!("{name}.xmir"))).unwrap();
            Xmir::parse(&text).unwrap()
        })
        .collect();
    let object = Program::from(documents).compile("Φ.p1").unwrap();
    let out = std::env::temp_dir().join("eo-devirt-p1");
    let unit = out.with_extension("o");
    fs::write(&unit, object).unwrap();
    let linked = Command::new("cc")
        .arg("-o")
        .arg(&out)
        .arg(&unit)
        .status()
        .unwrap();
    assert!(linked.success(), "linking failed");
    assert_eq!(Command::new(&out).status().unwrap().code(), Some(9));
}
