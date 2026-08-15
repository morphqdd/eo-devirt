//! Every XMIR file survives being parsed and printed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test says what it means by failing loudly, and there is nobody to hand an error to"
)]

use std::{fs, path::PathBuf};

use eo2bin::Xmir;

/// Every XMIR file must survive parse-print-parse unchanged. The parser refuses
/// anything it cannot model, so an unchanged second parse means the first one
/// dropped nothing.
#[test]
fn round_trips_every_fixture() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    assert!(
        round_trip(&dir) > 0,
        "no fixtures found in {}",
        dir.display()
    );
}

/// The same check over a whole parsed code base, pointed at by `XMIR_CORPUS`.
/// Skipped when the variable is unset, since the corpus is not part of the repo.
#[test]
fn round_trips_the_whole_corpus() {
    let Ok(dir) = std::env::var("XMIR_CORPUS") else {
        return;
    };
    let dir = PathBuf::from(dir);
    assert!(round_trip(&dir) > 0, "no XMIR found in {}", dir.display());
}

/// Round-trip every `.xmir` under a directory, returning how many were checked.
fn round_trip(dir: &PathBuf) -> usize {
    let mut checked = 0;
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            checked += round_trip(&path);
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "xmir") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let once = Xmir::parse(&text)
            .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
        let twice = Xmir::parse(&once.print())
            .unwrap_or_else(|e| panic!("{} failed to re-parse: {e}", path.display()));
        assert!(once == twice, "{} changed on the way back", path.display());
        checked += 1;
    }
    checked
}
