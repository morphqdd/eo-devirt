use eo2bin::{Program, Xmir};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// An expression the resolver pinned down completely, turned into machine code
/// and run. `p1` is `(2.plus 3).plus 4`.
///
/// The expected value is what the Java runtime answers for the same source:
///
/// ```text
/// $ eoc dataize p1
/// [0x40220000-00000000-] = 9.0
/// ```
#[test]
fn compiles_constant_arithmetic() {
    assert_eq!(run("p1").1, "9.0");
}

/// A formation applied to a literal. `p2` declares `twice(x) = x.plus x` and
/// applies it to 21, so the argument has to reach the body.
///
/// ```text
/// $ eoc dataize p2
/// [0x40450000-00000000-] = 42.0
/// ```
#[test]
fn compiles_a_formation_applied_to_a_literal() {
    assert_eq!(run("p2").1, "42.0");
}

/// Recursion and branching. `p4` is `fibo 6`, where `fibo` calls itself twice
/// and picks a branch with `if`, so the formation has to become a real
/// function and the branches must not both be evaluated.
///
/// ```text
/// $ eoc dataize p4
/// [0x40200000-00000000-] = 8.0
/// ```
#[test]
fn compiles_a_recursive_object() {
    assert_eq!(run("p4").1, "8.0");
}

/// A result an exit code cannot carry. `p5` is `2.div 4`, and the answer only
/// arrives because the binary writes it out through the runtime.
///
/// ```text
/// $ eoc dataize p5
/// [0x3FE00000-00000000-] = 0.5
/// ```
#[test]
fn writes_out_a_result_that_is_not_a_whole_number() {
    assert_eq!(run("p5").1, "0.5");
}

/// A system call. `p6` closes a descriptor that was never open, which every
/// POSIX answers with -1, so the value is the same on any machine.
///
/// ```text
/// $ eoc dataize p6
/// [0xBFF00000-00000000-] = -1.0
/// ```
#[test]
fn makes_a_system_call() {
    assert_eq!(run("p6").1, "-1.0");
}

/// Bytes reaching the operating system. `p7` writes "hi" to the standard
/// output and answers with how many bytes went. The two stay apart: what the
/// program writes is its own, and what it dataizes to is the runtime speaking
/// about it.
///
/// ```text
/// $ eoc dataize p7
/// hi
/// [0x40000000-00000000-] = 2.0
/// ```
#[test]
fn writes_bytes_to_the_standard_output() {
    assert_eq!(run("p7"), ("hi".to_string(), "2.0".to_string()));
}

/// Zero is a value like any other. `p8` dataizes to it, and it is reported
/// the same way every other value is, while the program itself writes nothing.
///
/// ```text
/// $ eoc dataize p8
/// [0x00000000-00000000-] = 0.0
/// ```
#[test]
fn reports_a_result_of_zero() {
    assert_eq!(run("p8"), (String::new(), "0.0".to_string()));
}

/// An object picked while the program runs. `p12` declares two unrelated
/// objects, chooses between them on a condition, and asks the winner for an
/// attribute. Which one it is cannot be known until it runs, so the name has
/// to be looked up then.
///
/// ```text
/// $ eoc dataize p12
/// [0x3FF00000-00000000-] = 1.0
/// ```
#[test]
fn dispatches_on_an_object_chosen_while_running() {
    assert_eq!(run("p12").1, "1.0");
}

/// An attribute nothing asks for is never computed. `p13` holds one that
/// writes a letter when it runs, and asks the object for a different one, so
/// the letter must not appear.
///
/// ```text
/// $ eoc dataize p13
/// [0x3FF00000-00000000-] = 1.0
/// ```
///
/// with nothing on the output.
#[test]
fn leaves_an_attribute_alone_until_it_is_asked_for() {
    assert_eq!(run("p13"), (String::new(), "1.0".to_string()));
}

/// Compile one fixture together with the runtime objects it leans on, link it
/// against the runtime library, run it, and hand back what the program wrote
/// and what it dataized to, which the runtime reports separately.
fn run(name: &str) -> (String, String) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("tests/fixtures");
    let documents: Vec<Xmir> = [
        name,
        "number",
        "bytes",
        "bool",
        "number/lt",
        "number/minus",
        "number/neg",
        "dataized",
        "string",
        "tuple",
        "posix",
        "true",
        "false",
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
    let out = std::env::temp_dir().join(format!("eo2bin-{name}"));
    let unit = out.with_extension("o");
    fs::write(&unit, object).unwrap();
    let linked = Command::new("cc")
        .arg("-o")
        .arg(&out)
        .arg(&unit)
        .arg(library(&root))
        .status()
        .unwrap();
    assert!(linked.success(), "linking failed");
    let done = Command::new(&out).output().unwrap();
    assert!(done.status.success(), "{} exited badly", out.display());
    let written = String::from_utf8(done.stdout).unwrap();
    let reported = String::from_utf8(done.stderr).unwrap();
    (written.trim().to_string(), reported.trim().to_string())
}

/// Build the runtime and say where its library landed.
fn library(root: &Path) -> PathBuf {
    let built = Command::new(env!("CARGO"))
        .arg("build")
        .arg("--package")
        .arg("eo-runtime")
        .current_dir(root)
        .status()
        .unwrap();
    assert!(built.success(), "the runtime failed to build");
    root.join("target/debug/libeo_runtime.a")
}
