use eo_devirt::{Program, Xmir};

/// `ρ` is bound by the `dot` rule to the formation that held the attribute, and
/// the `stay` rule refuses to rebind it, so its shape is the formation the body
/// is declared in -- whoever does the dispatching.
#[test]
fn resolves_the_parent_to_the_formation_that_declares_the_body() {
    let xmir = Xmir::parse(concat!(
        "<object>",
        "<o loc=\"Φ.a\" name=\"a\">",
        "<o name=\"k\"/>",
        "<o name=\"m\"><o base=\"ξ.ρ.k\" name=\"φ\"/></o>",
        "</o>",
        "<o loc=\"Φ.b\" name=\"b\"><o base=\"Φ.a.m\" name=\"φ\"/></o>",
        "</object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.dynamic(), 0);
    assert_eq!(report.unresolved(), 0);
}

/// A top-level object is declared in no formation: its `ρ` is `Φ` itself, which
/// this does not model yet.
#[test]
fn leaves_the_parent_open_for_a_top_level_object() {
    let xmir =
        Xmir::parse("<object><o loc=\"Φ.a\" name=\"a\"><o base=\"ξ.ρ\" name=\"φ\"/></o></object>")
            .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.dynamic(), 1);
}
