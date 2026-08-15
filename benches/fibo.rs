//! Time naive fibonacci, compiled.
//!
//! Two things are timed apart. Compiling is a plain function, measured in
//! process. Running is a binary being started and waited on, so every sample
//! carries the cost of starting a process, about a millisecond; that is a
//! floor rather than noise, and at the smaller sizes it is most of what gets
//! measured.
//!
//! The argument is patched straight into the XMIR, so no parser has to run per
//! size.

use eo2bin::{Program, Xmir};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The sizes to time. Naive fibonacci roughly doubles in cost with each step.
const SIZES: [u32; 5] = [18, 20, 25, 30, 32];

/// The objects `p4` leans on.
const LEANS: [&str; 7] = [
    "number",
    "bytes",
    "bool",
    "number/lt",
    "number/minus",
    "number/neg",
    "dataized",
];

fn main() {
    divan::main();
}

#[divan::bench(consts = SIZES)]
fn compile<const SIZE: u32>(bencher: divan::Bencher) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    bencher.bench(|| divan::black_box(object(&root, f64::from(SIZE))));
}

#[divan::bench(consts = SIZES, sample_count = 20)]
fn run<const SIZE: u32>(bencher: divan::Bencher) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = link(&root, f64::from(SIZE));
    bencher.bench(|| divan::black_box(Command::new(&out).output().unwrap()));
}

/// Compile fibonacci of one size into an object file.
fn object(root: &Path, size: f64) -> Vec<u8> {
    let fixtures = root.join("tests/fixtures");
    let text = fs::read_to_string(fixtures.join("p4.xmir")).unwrap();
    let mut documents = vec![Xmir::parse(&text.replace(&datum(6.0), &datum(size))).unwrap()];
    for each in LEANS {
        let text = fs::read_to_string(fixtures.join(format!("{each}.xmir"))).unwrap();
        documents.push(Xmir::parse(&text).unwrap());
    }
    Program::from(documents).compile("Φ.p4").unwrap()
}

/// Compile and link fibonacci of one size, and say where the binary landed.
fn link(root: &Path, size: f64) -> PathBuf {
    let out = std::env::temp_dir().join(format!("eo-bench-{size}"));
    fs::write(out.with_extension("o"), object(root, size)).unwrap();
    assert!(
        Command::new("cc")
            .arg("-o")
            .arg(&out)
            .arg(out.with_extension("o"))
            .arg(root.join("target/debug/libeo_runtime.a"))
            .status()
            .unwrap()
            .success(),
        "linking failed"
    );
    out
}

/// How a double is written in XMIR.
fn datum(value: f64) -> String {
    value
        .to_be_bytes()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<String>>()
        .join("-")
}
