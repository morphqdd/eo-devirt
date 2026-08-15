//! Moving a body to where it is dispatched.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test says what it means by failing loudly, and there is nobody to hand an error to"
)]

use eo2bin::{Program, Xmir};

/// A dispatch that lands on a body which never mentions `ρ` can be replaced by
/// that body: there is nothing in it whose meaning depends on where it was
/// dispatched from.
#[test]
fn inlines_a_dispatch_onto_a_body_that_never_mentions_the_parent() {
    let source = concat!(
        "<object>\n",
        "  <o loc=\"Φ.a\" name=\"a\">\n",
        "    <o name=\"m\">\n",
        "      <o name=\"k\"/>\n",
        "    </o>\n",
        "  </o>\n",
        "  <o loc=\"Φ.b\" name=\"b\">\n",
        "    <o base=\"Φ.a.m\" name=\"φ\"/>\n",
        "  </o>\n",
        "</object>\n"
    );
    let wanted = concat!(
        "<object>\n",
        "  <o loc=\"Φ.a\" name=\"a\">\n",
        "    <o name=\"m\">\n",
        "      <o name=\"k\"/>\n",
        "    </o>\n",
        "  </o>\n",
        "  <o loc=\"Φ.b\" name=\"b\">\n",
        "    <o name=\"φ\">\n",
        "      <o name=\"k\"/>\n",
        "    </o>\n",
        "  </o>\n",
        "</object>\n"
    );
    let done = Program::from(vec![Xmir::parse(source).unwrap()]).inline();
    assert_eq!(done[0].print(), wanted);
}

/// A body that reads `ρ` means something different once moved, so it stays put.
#[test]
fn leaves_a_dispatch_alone_when_the_body_mentions_the_parent() {
    let source = concat!(
        "<object>\n",
        "  <o loc=\"Φ.a\" name=\"a\">\n",
        "    <o name=\"m\">\n",
        "      <o base=\"ξ.ρ\" name=\"k\"/>\n",
        "    </o>\n",
        "  </o>\n",
        "  <o loc=\"Φ.b\" name=\"b\">\n",
        "    <o base=\"Φ.a.m\" name=\"φ\"/>\n",
        "  </o>\n",
        "</object>\n"
    );
    let done = Program::from(vec![Xmir::parse(source).unwrap()]).inline();
    assert_eq!(done[0].print(), source);
}

/// How many dispatches qualify, which is not the same as how much smaller the
/// program gets: a moved body brings its own dispatches along.
#[test]
fn counts_the_dispatches_it_can_move() {
    let source = concat!(
        "<object>",
        "<o loc=\"Φ.a\" name=\"a\"><o name=\"m\"><o name=\"k\"/></o></o>",
        "<o loc=\"Φ.b\" name=\"b\"><o base=\"Φ.a.m\" name=\"φ\"/></o>",
        "<o loc=\"Φ.c\" name=\"c\"><o base=\"ξ.ρ\" name=\"φ\"/></o>",
        "</object>"
    );
    let program = Program::from(vec![Xmir::parse(source).unwrap()]);
    assert_eq!(program.movable(), 1);
}
