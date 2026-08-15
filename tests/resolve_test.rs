use eo_devirt::{Program, Xmir};

#[test]
fn counts_a_step_onto_the_result_of_an_atom_as_dynamic() {
    let xmir = Xmir::parse(concat!(
        "<object>",
        "<o loc=\"Φ.a\" name=\"a\"><o atom=\"Φ.bool\" name=\"λ\"/></o>",
        "<o loc=\"Φ.b\" name=\"b\"><o base=\"Φ.a.if\" name=\"φ\"/></o>",
        "</object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.dynamic(), 1);
    assert_eq!(report.unresolved(), 0);
}

#[test]
fn resolves_an_object_declared_inside_a_package() {
    let library =
        Xmir::parse("<object><o loc=\"Φ.p.a\" name=\"a\"><o name=\"m\"/></o></object>").unwrap();
    let user = Xmir::parse(
        "<object><o loc=\"Φ.b\" name=\"b\"><o base=\"Φ.p.a.m\" name=\"φ\"/></o></object>",
    )
    .unwrap();
    let report = Program::from(vec![library, user]).resolve();
    assert_eq!(report.resolved(), 3);
    assert_eq!(report.unresolved(), 0);
}

#[test]
fn counts_a_step_onto_the_parent_as_dynamic() {
    let xmir = Xmir::parse(concat!(
        "<object><o name=\"a\"><o name=\"b\">",
        "<o base=\"ξ.ρ\" name=\"φ\"/>",
        "</o></o></object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.dynamic(), 1);
    assert_eq!(report.unresolved(), 0);
}

#[test]
fn names_what_it_could_not_resolve() {
    let xmir =
        Xmir::parse("<object><o name=\"b\"><o base=\"Φ.nope\" name=\"φ\"/></o></object>").unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.missing(), &["nope".to_string()]);
}

#[test]
fn counts_a_step_taken_through_a_void_as_dynamic() {
    let xmir = Xmir::parse(concat!(
        "<object><o name=\"b\">",
        "<o base=\"∅\" name=\"x\"/>",
        "<o base=\"ξ.x.plus\" name=\"φ\"/>",
        "</o></object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.resolved(), 1);
    assert_eq!(report.dynamic(), 1);
    assert_eq!(report.unresolved(), 0);
}

#[test]
fn follows_the_decorator_when_the_attribute_is_absent() {
    let xmir = Xmir::parse(concat!(
        "<object>",
        "<o name=\"a\"><o name=\"m\"/></o>",
        "<o name=\"b\"><o base=\"Φ.a\" name=\"φ\"/></o>",
        "<o name=\"c\"><o base=\"Φ.b.m\" name=\"φ\"/></o>",
        "</object>"
    ))
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.resolved(), 3);
    assert_eq!(report.unresolved(), 0);
}

#[test]
fn counts_a_dispatch_on_a_computed_receiver_as_dynamic() {
    let xmir =
        Xmir::parse("<object><o name=\"b\"><o base=\".if\" name=\"φ\"/></o></object>").unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.dynamic(), 1);
    assert_eq!(report.resolved(), 0);
    assert_eq!(report.unresolved(), 0);
}

#[test]
fn resolves_every_step_of_a_chain_through_a_global_object() {
    let xmir = Xmir::parse(
        "<object><o name=\"a\"><o name=\"m\"/></o><o name=\"b\"><o base=\"Φ.a.m\" name=\"φ\"/></o></object>",
    )
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.resolved(), 2);
    assert_eq!(report.unresolved(), 0);
}

#[test]
fn resolves_a_reference_to_a_sibling_attribute() {
    let xmir = Xmir::parse(
        "<object><o name=\"b\"><o name=\"x\"/><o base=\"ξ.x\" name=\"φ\"/></o></object>",
    )
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.resolved(), 1);
    assert_eq!(report.unresolved(), 0);
}

#[test]
fn resolves_a_reference_to_a_top_level_object() {
    let xmir = Xmir::parse(
        "<object><o name=\"a\"/><o name=\"b\"><o base=\"Φ.a\" name=\"φ\"/></o></object>",
    )
    .unwrap();
    let report = Program::from(vec![xmir]).resolve();
    assert_eq!(report.resolved(), 1);
    assert_eq!(report.unresolved(), 0);
}
