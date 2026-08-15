//! Report how many dispatches can be replaced by the body they land on.
//!
//! Usage: `cargo run --example inline <dir-with-xmir>`

use eo2bin::{Program, Xmir};
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let dir = std::env::args().nth(1).expect("usage: inline <dir>");
    let mut paths = Vec::new();
    collect(Path::new(&dir), &mut paths);
    let documents: Vec<Xmir> = paths
        .iter()
        .map(|path| Xmir::parse(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    let before: usize = documents.iter().map(|d| dispatches(&d.print())).sum();
    let grew: usize = documents.iter().map(|d| d.print().len()).sum();
    let program = Program::from(documents);
    println!("movable sites:  {}", program.movable());
    let done = program.inline();
    let after: usize = done.iter().map(|d| dispatches(&d.print())).sum();
    let size: usize = done.iter().map(|d| d.print().len()).sum();
    println!("dispatch sites: {before} -> {after}");
    println!(
        "change:         {:+} ({:+.1}%)",
        after as i64 - before as i64,
        100.0 * (after as f64 - before as f64) / before as f64
    );
    println!(
        "size:           {grew} -> {size} ({:+.1}%)",
        100.0 * (size as f64 - grew as f64) / grew as f64
    );
}

fn dispatches(text: &str) -> usize {
    text.matches("base=\"").count()
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
