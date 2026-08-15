#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "a tool run by hand reports to whoever ran it, and stops on anything it cannot do"
)]

//! Report how much of a program's dispatch resolves statically.
//!
//! Usage: `cargo run --example resolve <dir-with-xmir>`

use std::{
    fs,
    path::{Path, PathBuf},
};

use eo2bin::{Program, Xmir};

fn main() {
    let dir = std::env::args().nth(1).expect("usage: resolve <dir>");
    let mut paths = Vec::new();
    collect(Path::new(&dir), &mut paths);
    let documents: Vec<Xmir> = paths
        .iter()
        .map(|path| {
            let text = fs::read_to_string(path).unwrap();
            Xmir::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect();
    println!("documents: {}", documents.len());
    let report = Program::from(documents).resolve();
    let total = report.resolved() + report.dynamic() + report.unresolved();
    println!("steps:      {total}");
    println!(
        "resolved:   {} ({:.1}%)",
        report.resolved(),
        100.0 * report.resolved() as f64 / total as f64
    );
    println!(
        "dynamic:    {} ({:.1}%)",
        report.dynamic(),
        100.0 * report.dynamic() as f64 / total as f64
    );
    println!(
        "unresolved: {} ({:.1}%)",
        report.unresolved(),
        100.0 * report.unresolved() as f64 / total as f64
    );
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for name in report.missing() {
        *counts.entry(name.as_str()).or_default() += 1;
    }
    let mut top: Vec<(&str, usize)> = counts.into_iter().collect();
    top.sort_by_key(|(_, times)| std::cmp::Reverse(*times));
    println!("\ntop names not found:");
    for (name, times) in top.iter().take(15) {
        println!("  {times:5}  {name}");
    }
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
