//! Resolution of the references an EO program makes.
//!
//! Every dispatch in XMIR is written as a chain in the `base` attribute:
//! `Φ.number.as-i64`, `ξ.x.plus`, `.if`. A chain starting at `Φ` begins at a
//! global object, one starting at `ξ` at the enclosing formation, and a leading
//! dot dispatches on a receiver computed at run time.

use crate::xmir::{Element, Xmir};
use std::collections::{HashMap, HashSet};

/// How far the resolver follows decorators before it gives up, so that a cycle
/// of objects decorating each other cannot spin forever.
const DEPTH: usize = 32;

/// How many times the shapes of voids are recomputed before the answer is
/// taken as settled. What one call site puts into a void can depend on what
/// another put into a void of its own, so one pass is not enough.
const ROUNDS: usize = 8;

/// A whole program: every XMIR document that makes it up.
pub struct Program {
    documents: Vec<Xmir>,
}

/// How much of the program's dispatch was pinned down.
pub struct Report {
    resolved: usize,
    unresolved: usize,
    dynamic: usize,
    missing: Vec<String>,
}

impl From<Vec<Xmir>> for Program {
    fn from(documents: Vec<Xmir>) -> Self {
        Self { documents }
    }
}

impl Program {
    /// Replace every dispatch whose body is safe to move with that body.
    ///
    /// Moving a body is only sound when nothing in it reads `ρ`: the `dot` rule
    /// binds `ρ` to whatever the dispatch was made on, so a body that reads it
    /// means something else once it sits somewhere else.
    pub fn inline(&self) -> Vec<Xmir> {
        let resolver = self.resolver();
        self.documents
            .iter()
            .map(|document| document.rebuilt(resolver.inlined(document.root(), Where::Nowhere)))
            .collect()
    }

    /// How many dispatches could be replaced by the body they land on.
    pub fn movable(&self) -> usize {
        let resolver = self.resolver();
        self.documents
            .iter()
            .map(|document| resolver.movables(document.root(), Where::Nowhere))
            .sum()
    }

    /// Resolve every reference the program makes and report on the outcome.
    pub fn resolve(&self) -> Report {
        let resolver = self.resolver();
        let mut report = Report {
            resolved: 0,
            unresolved: 0,
            dynamic: 0,
            missing: Vec::new(),
        };
        for document in &self.documents {
            resolver.count(document.root(), Where::Nowhere, &mut report);
        }
        report
    }

    /// Everything the program declares at the top level, plus the formation
    /// each object is declared in.
    fn resolver(&self) -> Resolver<'_> {
        let mut nests = HashMap::new();
        for document in &self.documents {
            for object in &document.root().children {
                nest(object, None, &mut nests);
            }
        }
        let mut resolver = Resolver {
            globals: self
                .documents
                .iter()
                .flat_map(|document| document.root().children.iter())
                .filter_map(|object| path(object).map(|path| (path, object)))
                .collect(),
            nests,
            shapes: HashMap::new(),
            open: HashSet::new(),
        };
        for _ in 0..ROUNDS {
            let mut shapes = HashMap::new();
            let mut open = HashSet::new();
            for document in &self.documents {
                resolver.learn(document.root(), Where::Nowhere, &mut shapes, &mut open);
            }
            let settled = signature(&shapes, &open) == signature(&resolver.shapes, &resolver.open);
            resolver.shapes = shapes;
            resolver.open = open;
            if settled {
                break;
            }
        }
        resolver
    }
}

impl Report {
    /// How many dispatch steps were pinned to a known object.
    pub fn resolved(&self) -> usize {
        self.resolved
    }

    /// How many named something the program does not declare at all.
    pub fn unresolved(&self) -> usize {
        self.unresolved
    }

    /// How many go through a value only known at run time, and so are left to
    /// the shape analysis.
    pub fn dynamic(&self) -> usize {
        self.dynamic
    }

    /// The names it could not find, in the order it met them.
    pub fn missing(&self) -> &[String] {
        &self.missing
    }
}

/// What one step of a chain cost us.
enum Score {
    /// The name was found where the program says it is.
    Resolved,
    /// The name sits on a value that only exists at run time.
    Dynamic,
    /// The name is nowhere to be found.
    Unresolved,
}

/// Where a chain has got to.
#[derive(Clone, Copy)]
enum Where<'a> {
    /// On an object the program declares.
    At(&'a Element),
    /// On something whose value only exists at run time: a void, or a receiver
    /// the program computes. Name resolution stops here; shape analysis picks
    /// it up.
    Runtime,
    /// On a name the program does not declare at all.
    Nowhere,
}

/// The objects a program declares at the top level, which is what `Φ` names.
///
/// A top-level object carries its full path in `loc`, so an object declared in
/// a package is known as `string.regex`, not just `regex`.
///
/// What every call site put into a void.
enum Shape<'a> {
    /// Every call site agreed on this one.
    One(&'a Element),
    /// They did not agree.
    Many,
}

/// `nests` maps every object to the formation that declares it, which is what
/// its `ρ` will be bound to. `shapes` says what goes into each void, and `open`
/// holds the names dispatched by applications we could not resolve: such an
/// application can fill the voids of any body bound under that name, so those
/// voids stay open however well the visible call sites agree.
struct Resolver<'a> {
    globals: Vec<(String, &'a Element)>,
    nests: HashMap<usize, &'a Element>,
    shapes: HashMap<usize, Shape<'a>>,
    open: HashSet<String>,
}

impl<'a> Resolver<'a> {
    /// Walk the tree, resolving the `base` of every element that has one.
    ///
    /// `scope` is the formation whose body we are inside, which `ξ` names.
    fn count(&self, element: &'a Element, scope: Where<'a>, report: &mut Report) {
        let inner = match attribute(element, "base") {
            Some(base) => {
                self.walk(element, base, scope, report);
                scope
            }
            None => Where::At(element),
        };
        for child in &element.children {
            self.count(child, inner, report);
        }
    }

    /// Follow a chain one step at a time, scoring each step by whether it
    /// landed on an object we know.
    ///
    /// A chain that starts with a bare dot dispatches on a receiver computed at
    /// run time, which no amount of name resolution can pin down, so every one
    /// of its steps counts as dynamic.
    fn walk(&self, element: &'a Element, base: &str, scope: Where<'a>, report: &mut Report) {
        let mut steps = base.split('.');
        let mut here = match steps.next() {
            Some("Φ") => {
                let rest: Vec<&str> = steps.collect();
                return self.enter(&rest, report);
            }
            Some("ξ") => scope,
            Some("∅") => Where::Runtime,
            _ => self.handed(element, scope, 0),
        };
        for name in steps.filter(|step| !step.is_empty()) {
            let (score, landing) = self.step(here, name, 0);
            tally(score, name, report);
            here = landing;
        }
    }

    /// The receiver an application hands to a leading-dot dispatch: the one
    /// child that carries no `as`, every argument having one.
    fn handed(&self, element: &'a Element, scope: Where<'a>, depth: usize) -> Where<'a> {
        if depth > DEPTH {
            return Where::Runtime;
        }
        match element
            .children
            .iter()
            .find(|child| attribute(child, "as").is_none())
        {
            Some(receiver) => self.value(receiver, scope, depth + 1),
            None => Where::Runtime,
        }
    }

    /// Walk the program, noting what each call site puts into a void.
    fn learn(
        &self,
        element: &'a Element,
        scope: Where<'a>,
        shapes: &mut HashMap<usize, Shape<'a>>,
        open: &mut HashSet<String>,
    ) {
        let inner = match attribute(element, "base") {
            Some(base) => {
                self.hand(element, base, scope, shapes, open);
                scope
            }
            None => Where::At(element),
        };
        for child in &element.children {
            self.learn(child, inner, shapes, open);
        }
    }

    /// Hand the arguments of one application to the voids they fill.
    fn hand(
        &self,
        element: &'a Element,
        base: &str,
        scope: Where<'a>,
        shapes: &mut HashMap<usize, Shape<'a>>,
        open: &mut HashSet<String>,
    ) {
        let args: Vec<&'a Element> = element
            .children
            .iter()
            .filter(|child| attribute(child, "as").is_some())
            .collect();
        if args.is_empty() {
            return;
        }
        let Where::At(formation) = self.target(base, scope, 0) else {
            if let Some(last) = base.rsplit('.').find(|step| !step.is_empty()) {
                open.insert(last.to_string());
            }
            return;
        };
        let voids: Vec<&'a Element> = formation
            .children
            .iter()
            .filter(|child| attribute(child, "base") == Some("∅"))
            .collect();
        for arg in args {
            let slot = attribute(arg, "as").and_then(|slot| match slot.strip_prefix('α') {
                Some(index) => index
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| voids.get(i).copied()),
                None => voids.iter().copied().find(|void| named(void, slot)),
            });
            let Some(slot) = slot else { continue };
            let seen = match self.value(arg, scope, 0) {
                Where::At(shape) => Shape::One(shape),
                _ => Shape::Many,
            };
            let key = slot as *const Element as usize;
            match (shapes.get(&key), &seen) {
                (None, _) => {
                    shapes.insert(key, seen);
                }
                (Some(Shape::One(known)), Shape::One(shape)) if std::ptr::eq(*known, *shape) => {}
                _ => {
                    shapes.insert(key, Shape::Many);
                }
            }
        }
    }

    /// What a void holds, when every call site agreed and nothing could have
    /// filled it from a place we could not see.
    fn filled(&self, void: &'a Element) -> Where<'a> {
        let hidden = self
            .nests
            .get(&(void as *const Element as usize))
            .and_then(|formation| attribute(formation, "name"))
            .is_some_and(|name| self.open.contains(name));
        if hidden {
            return Where::Runtime;
        }
        match self.shapes.get(&(void as *const Element as usize)) {
            Some(Shape::One(shape)) => Where::At(shape),
            _ => Where::Runtime,
        }
    }

    /// Rewrite a subtree, replacing every dispatch that can safely be replaced
    /// by the body it lands on.
    fn inlined(&self, element: &'a Element, scope: Where<'a>) -> Element {
        let inner = match attribute(element, "base") {
            Some(_) => scope,
            None => Where::At(element),
        };
        if let Some(body) = self.movable(element, scope) {
            return Element {
                tag: element.tag.clone(),
                attributes: element
                    .attributes
                    .iter()
                    .filter(|(key, _)| key != "base")
                    .cloned()
                    .collect(),
                text: element.text.clone(),
                children: body.children.clone(),
            };
        }
        Element {
            tag: element.tag.clone(),
            attributes: element.attributes.clone(),
            text: element.text.clone(),
            children: element
                .children
                .iter()
                .map(|child| self.inlined(child, inner))
                .collect(),
        }
    }

    /// Count the dispatches in a subtree that could be moved.
    fn movables(&self, element: &'a Element, scope: Where<'a>) -> usize {
        let inner = match attribute(element, "base") {
            Some(_) => scope,
            None => Where::At(element),
        };
        usize::from(self.movable(element, scope).is_some())
            + element
                .children
                .iter()
                .map(|child| self.movables(child, inner))
                .sum::<usize>()
    }

    /// The body a dispatch lands on, when moving it would keep the meaning.
    fn movable(&self, element: &'a Element, scope: Where<'a>) -> Option<&'a Element> {
        if !element.children.is_empty() {
            return None;
        }
        let base = attribute(element, "base")?;
        let Where::At(body) = self.target(base, scope, 0) else {
            return None;
        };
        if attribute(body, "base").is_some() || !settled(body) {
            return None;
        }
        Some(body)
    }

    /// Start a chain at `Φ`, taking as many steps as the longest global path
    /// that matches, then carrying on inside whatever that object is.
    fn enter(&self, steps: &[&str], report: &mut Report) {
        let Some(first) = steps.first() else { return };
        let taken = self.longest(steps);
        let mut here = match taken {
            0 => {
                tally(Score::Unresolved, first, report);
                Where::Nowhere
            }
            taken => {
                for name in &steps[..taken] {
                    tally(Score::Resolved, name, report);
                }
                self.global(&steps[..taken].join("."))
            }
        };
        let rest = if taken == 0 { 1 } else { taken };
        for name in &steps[rest..] {
            let (score, landing) = self.step(here, name, 0);
            tally(score, name, report);
            here = landing;
        }
    }

    /// How many leading steps name a global object, preferring the longest
    /// match so that `string.regex` wins over `string`.
    fn longest(&self, steps: &[&str]) -> usize {
        (1..=steps.len())
            .rev()
            .find(|taken| matches!(self.global(&steps[..*taken].join(".")), Where::At(_)))
            .unwrap_or(0)
    }

    /// Take one step of a chain from wherever the last one landed.
    ///
    /// Finding the name and landing somewhere useful are two different things:
    /// `ξ.x` on a void finds `x` exactly where the program declares it, yet
    /// lands on a value that will not exist until the program runs.
    ///
    /// `ρ` is the object a formation was dispatched from, which the `dot` rule
    /// of the calculus binds at reduction time. It usually turns out to be the
    /// enclosing formation, but nothing in the program says it must be, so it
    /// is left to the shape analysis rather than guessed at here.
    ///
    /// A formation holding a `λ` is an atom: what it offers beyond its own
    /// bindings lives in the result its native code produces. The `atom`
    /// attribute on that `λ` even declares the shape of that result, which the
    /// shape analysis will be able to use; until then the step is dynamic.
    fn step(&self, here: Where<'a>, name: &str, depth: usize) -> (Score, Where<'a>) {
        if name == "ρ" {
            let Where::At(body) = here else {
                return (Score::Dynamic, Where::Runtime);
            };
            return match self.nests.get(&(body as *const Element as usize)) {
                Some(nest) => (Score::Resolved, Where::At(nest)),
                None => (Score::Dynamic, Where::Runtime),
            };
        }
        match here {
            Where::At(formation) => match self.attribute(formation, name, depth) {
                Some(binding) => (Score::Resolved, self.value(binding, here, depth)),
                None if self.opaque(formation, depth) => (Score::Dynamic, Where::Runtime),
                None => (Score::Unresolved, Where::Nowhere),
            },
            Where::Runtime => (Score::Dynamic, Where::Runtime),
            Where::Nowhere => (Score::Unresolved, Where::Nowhere),
        }
    }

    /// Whether a formation might still offer names we cannot see: it hides
    /// native code behind a `λ`, or it decorates something we could not follow.
    /// Not finding a name on such a formation is not knowing, not absence.
    fn opaque(&self, formation: &'a Element, depth: usize) -> bool {
        if child(formation, "λ").is_some() {
            return true;
        }
        match child(formation, "φ") {
            Some(decorator) => !matches!(
                self.value(decorator, Where::At(formation), depth + 1),
                Where::At(_)
            ),
            None => false,
        }
    }

    /// Where a chain lands, without scoring anything on the way.
    fn target(&self, base: &str, scope: Where<'a>, depth: usize) -> Where<'a> {
        self.lands(None, base, scope, depth)
    }

    /// Where a chain lands, knowing which element carries it so that a
    /// leading-dot dispatch can see its receiver.
    fn lands(
        &self,
        element: Option<&'a Element>,
        base: &str,
        scope: Where<'a>,
        depth: usize,
    ) -> Where<'a> {
        let mut steps = base.split('.');
        let mut here = match steps.next() {
            Some("Φ") => {
                let rest: Vec<&str> = steps.collect();
                let taken = self.longest(&rest);
                if taken == 0 {
                    return Where::Nowhere;
                }
                let mut here = self.global(&rest[..taken].join("."));
                for name in &rest[taken..] {
                    here = self.step(here, name, depth + 1).1;
                }
                return here;
            }
            Some("ξ") => scope,
            Some("∅") => Where::Runtime,
            _ => match element {
                Some(element) => self.handed(element, scope, depth),
                None => return Where::Runtime,
            },
        };
        for name in steps.filter(|step| !step.is_empty()) {
            here = self.step(here, name, depth + 1).1;
        }
        here
    }

    /// The binding a formation offers under this name.
    ///
    /// When the formation does not hold the name, the decorator is followed,
    /// and failing that the result of the formation's own native code, whose
    /// shape the `atom` attribute on its `λ` declares.
    fn attribute(&self, formation: &'a Element, name: &str, depth: usize) -> Option<&'a Element> {
        if depth > DEPTH {
            return None;
        }
        if let Some(found) = child(formation, name) {
            return Some(found);
        }
        if let Some(decorator) = child(formation, "φ")
            && let Where::At(decorated) = self.value(decorator, Where::At(formation), depth + 1)
            && let Some(found) = self.attribute(decorated, name, depth + 1)
        {
            return Some(found);
        }
        let kind = attribute(child(formation, "λ")?, "atom")?;
        match self.lands(None, kind, Where::Nowhere, depth + 1) {
            Where::At(result) => self.attribute(result, name, depth + 1),
            _ => None,
        }
    }

    /// The object a binding stands for: the formation it is, or wherever its
    /// own chain lands.
    fn value(&self, binding: &'a Element, scope: Where<'a>, depth: usize) -> Where<'a> {
        if depth > DEPTH {
            return Where::Runtime;
        }
        match attribute(binding, "base") {
            Some("∅") => self.filled(binding),
            Some(base) => self.lands(Some(binding), base, scope, depth + 1),
            None => Where::At(binding),
        }
    }

    /// The top-level object under this full path.
    fn global(&self, path: &str) -> Where<'a> {
        match self
            .globals
            .iter()
            .find(|(known, _)| known == path)
            .map(|(_, object)| *object)
        {
            Some(found) => Where::At(found),
            None => Where::Nowhere,
        }
    }
}

/// Record what a step cost, keeping the name of anything not found.
fn tally(score: Score, name: &str, report: &mut Report) {
    match score {
        Score::Resolved => report.resolved += 1,
        Score::Dynamic => report.dynamic += 1,
        Score::Unresolved => {
            report.unresolved += 1;
            report.missing.push(name.to_string());
        }
    }
}

/// A cheap stand-in for the whole table, to tell when it stopped changing.
fn signature(shapes: &HashMap<usize, Shape<'_>>, open: &HashSet<String>) -> (usize, usize, usize) {
    (
        shapes.len(),
        shapes
            .values()
            .filter(|shape| matches!(shape, Shape::Many))
            .count(),
        open.len(),
    )
}

/// Note, for every object, the formation it is declared in. An application is
/// not a formation, so what sits under it belongs to the formation above.
fn nest<'a>(
    element: &'a Element,
    formation: Option<&'a Element>,
    nests: &mut HashMap<usize, &'a Element>,
) {
    if let Some(formation) = formation {
        nests.insert(element as *const Element as usize, formation);
    }
    let inner = match attribute(element, "base") {
        Some(_) => formation,
        None => Some(element),
    };
    for child in &element.children {
        nest(child, inner, nests);
    }
}

/// Whether a body carries its meaning with it: no `ρ` to be rebound, no void
/// left dangling, and no native code hiding behind a `λ`.
fn settled(body: &Element) -> bool {
    if child(body, "λ").is_some() {
        return false;
    }
    let mine = match attribute(body, "base") {
        Some("∅") => false,
        Some(base) => !base.split('.').any(|step| step == "ρ"),
        None => true,
    };
    mine && body.children.iter().all(settled)
}

/// The full path a top-level object is known by: whatever `loc` says, minus the
/// leading `Φ.`, falling back to the plain name when there is no locator.
fn path(object: &Element) -> Option<String> {
    match attribute(object, "loc") {
        Some(loc) => loc.strip_prefix("Φ.").map(str::to_string),
        None => attribute(object, "name").map(str::to_string),
    }
}

/// The attribute a formation binds under this name.
fn child<'a>(formation: &'a Element, name: &str) -> Option<&'a Element> {
    formation.children.iter().find(|child| named(child, name))
}

/// Whether an element is bound under this name.
fn named(element: &Element, name: &str) -> bool {
    attribute(element, "name") == Some(name)
}

/// The value of a named attribute, if the element carries it.
fn attribute<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}
