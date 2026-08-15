#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "a tool run by hand reports to whoever ran it, and stops on anything it cannot do"
)]

//! Try to compile every test written in a parsed EO code base.
//!
//! The `.eo` sources carry their own tests, written as `++>` bindings, which
//! the parser leaves as attributes whose name starts with a plus. Each is an
//! expression that should dataize to true, so each is a small program to
//! compile. What fails, and with what message, says what is missing.
//!
//! Usage: `cargo run --release --example corpus -- <dir-with-xmir>`

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use eo2bin::{Program, Xmir};

fn main() {
    let dir = std::env::args().nth(1).expect("usage: corpus <dir>");
    let mut paths = Vec::new();
    collect(Path::new(&dir), &mut paths);
    let documents: Vec<Xmir> = paths
        .iter()
        .map(|path| Xmir::parse(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    let tests: Vec<String> = documents
        .iter()
        .flat_map(|document| named(&document.print()))
        .collect();
    println!("documents: {}, tests: {}", documents.len(), tests.len());
    let program = Program::from(documents);
    let mut done = 0;
    let mut why: BTreeMap<String, usize> = BTreeMap::new();
    for test in &tests {
        match program.compile(test) {
            Ok(_) => done += 1,
            Err(e) => *why.entry(shorten(&e)).or_default() += 1,
        }
    }
    println!(
        "compiled: {done} of {} ({:.1}%)",
        tests.len(),
        100.0 * f64::from(u32::try_from(done).unwrap())
            / f64::from(u32::try_from(tests.len()).unwrap())
    );
    let mut ranked: Vec<(&String, &usize)> = why.iter().collect();
    ranked.sort_by_key(|(_, times)| std::cmp::Reverse(**times));
    println!("\nwhat stopped it:");
    for (reason, times) in ranked.iter().take(15) {
        println!("  {times:5}  {reason}");
    }
}

/// The locators of every test an XMIR document holds.
fn named(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.split("loc=\"").nth(1))
        .filter_map(|rest| rest.split('"').next())
        .filter(|loc| {
            loc.rsplit('.')
                .next()
                .is_some_and(|last| last.starts_with('+'))
        })
        .map(str::to_string)
        .collect()
}

/// One message stands for every one like it, so they can be counted.
fn shorten(why: &str) -> String {
    why.split_whitespace()
        .map(|word| {
            if word.contains('.') || word.contains('α') {
                "..."
            } else {
                word
            }
        })
        .collect::<Vec<&str>>()
        .join(" ")
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "xmir") {
            out.push(path);
        }
    }
}
