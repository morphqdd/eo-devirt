use eo_devirt::{Program, Xmir};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// The arithmetic slice: an expression the resolver pinned down completely,
/// turned into machine code and run. `p1` is `(2.plus 3).plus 4`.
///
/// The 9 is not arithmetic done here. It is what the Java runtime answers for
/// the same source:
///
/// ```text
/// $ eoc dataize p1
/// [0x40220000-00000000-] = 9.0
/// ```
///
/// Comparing through the exit code only works while the answer is a small
/// whole number, an exit code being one byte. Anything else needs the value
/// printed, which needs the runtime this does not have yet.
#[test]
fn compiles_constant_arithmetic_into_a_binary_that_exits_with_the_result() {
    assert_eq!(run("p1"), 9);
}

/// Compile one fixture together with the runtime objects it leans on, link it
/// and report the exit code.
fn run(name: &str) -> i32 {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let documents: Vec<Xmir> = [
        name,
        "number",
        "bytes",
        "bool",
        "number/lt",
        "number/minus",
        "number/neg",
        "dataized",
    ]
    .iter()
    .map(|each| {
        let text = fs::read_to_string(fixtures.join(format!("{each}.xmir"))).unwrap();
        Xmir::parse(&text).unwrap()
    })
    .collect();
    let object = Program::from(documents)
        .compile(&format!("Φ.{name}"))
        .unwrap();
    let out = std::env::temp_dir().join(format!("eo-devirt-{name}"));
    let unit = out.with_extension("o");
    fs::write(&unit, object).unwrap();
    let linked = Command::new("cc")
        .arg("-o")
        .arg(&out)
        .arg(&unit)
        .status()
        .unwrap();
    assert!(linked.success(), "linking failed");
    Command::new(&out).status().unwrap().code().unwrap()
}

/// A formation applied to a literal. `p2` declares `twice(x) = x.plus x` and
/// applies it to 21, so the argument has to reach the body.
///
/// The 42 comes from the Java runtime:
///
/// ```text
/// $ eoc dataize p2
/// [0x40450000-00000000-] = 42.0
/// ```
#[test]
fn compiles_a_formation_applied_to_a_literal() {
    assert_eq!(run("p2"), 42);
}

/// Recursion and branching. `p4` is `fibo 6` where `fibo` calls itself twice
/// and picks a branch with `if`, so the formation has to become a real
/// function and the branches must not both be evaluated.
///
/// The 8 comes from the Java runtime:
///
/// ```text
/// $ eoc dataize p4
/// [0x40200000-00000000-] = 8.0
/// ```
#[test]
fn compiles_a_recursive_object() {
    assert_eq!(run("p4"), 8);
}
