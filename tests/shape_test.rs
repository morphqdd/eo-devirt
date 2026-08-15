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

/// A void has no value of its own, but every call site says what shape goes
/// into it. When they all agree, the body can be resolved through it.
#[test]
fn carries_the_shape_of_an_argument_into_the_body() {
    let xmir = Xmir::parse(concat!(
        "<object>",
        "<o loc=\"Φ.n\" name=\"n\"><o name=\"plus\"/></o>",
        "<o loc=\"Φ.a\" name=\"a\">",
        "<o name=\"twice\">",
        "<o base=\"∅\" name=\"x\"/>",
        "<o base=\"ξ.x.plus\" name=\"φ\"/>",
        "</o>",
        "<o base=\"ξ.twice\" name=\"φ\"><o as=\"α0\" base=\"Φ.n\"/></o>",
        "</o>",
        "</object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.dynamic(), 0);
    assert_eq!(report.unresolved(), 0);
}

/// Two call sites disagreeing leave the void open.
#[test]
fn leaves_a_void_open_when_call_sites_disagree() {
    let xmir = Xmir::parse(concat!(
        "<object>",
        "<o loc=\"Φ.n\" name=\"n\"><o name=\"plus\"/></o>",
        "<o loc=\"Φ.s\" name=\"s\"/>",
        "<o loc=\"Φ.a\" name=\"a\">",
        "<o name=\"twice\">",
        "<o base=\"∅\" name=\"x\"/>",
        "<o base=\"ξ.x.plus\" name=\"φ\"/>",
        "</o>",
        "<o base=\"ξ.twice\" name=\"φ\"><o as=\"α0\" base=\"Φ.n\"/></o>",
        "<o base=\"ξ.twice\" name=\"t\"><o as=\"α0\" base=\"Φ.s\"/></o>",
        "</o>",
        "</object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert!(report.dynamic() > 0);
}

/// A dispatch written with a leading dot takes its receiver from the child that
/// carries no `as`, and that child has a shape like any other expression.
#[test]
fn resolves_a_dispatch_on_the_receiver_it_is_handed() {
    let xmir = Xmir::parse(concat!(
        "<object>",
        "<o loc=\"Φ.n\" name=\"n\"><o name=\"plus\"/></o>",
        "<o loc=\"Φ.a\" name=\"a\">",
        "<o base=\".plus\" name=\"φ\"><o base=\"Φ.n\" loc=\"Φ.a.φ.ρ\"/></o>",
        "</o>",
        "</object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.dynamic(), 0);
    assert_eq!(report.unresolved(), 0);
}

/// Dispatching past an atom goes through what its native code produces, and the
/// `atom` attribute on the `λ` says what shape that is.
#[test]
fn follows_the_declared_result_of_an_atom() {
    let xmir = Xmir::parse(concat!(
        "<object>",
        "<o loc=\"Φ.bool\" name=\"bool\"><o name=\"if\"/></o>",
        "<o loc=\"Φ.gt\" name=\"gt\"><o atom=\"Φ.bool\" name=\"λ\"/></o>",
        "<o loc=\"Φ.a\" name=\"a\"><o base=\"Φ.gt.if\" name=\"φ\"/></o>",
        "</object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.resolved(), 2);
    assert_eq!(report.dynamic(), 0);
}

/// When a formation decorates something we cannot pin down, an attribute we do
/// not find might still be there. That is not knowing, not absence.
#[test]
fn calls_a_name_dynamic_when_the_decorator_cannot_be_followed() {
    let xmir = Xmir::parse(concat!(
        "<object>",
        "<o loc=\"Φ.a\" name=\"a\"><o base=\"∅\" name=\"φ\"/></o>",
        "<o loc=\"Φ.b\" name=\"b\"><o base=\"Φ.a.whatever\" name=\"φ\"/></o>",
        "</object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.unresolved(), 0);
    assert_eq!(report.dynamic(), 1);
}
