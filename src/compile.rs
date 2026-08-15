//! Turning a resolved expression into machine code.
//!
//! This covers what the resolver pinned down completely and what needs no
//! object at run time: number literals, the atoms that map onto a single
//! instruction, and formations applied to arguments. An argument is built where
//! the call is written and handed to the void it fills, so no slot outlives the
//! expression being built. Everything else is refused rather than guessed at.

use crate::program::{Program, Resolver, Where, attribute, child};
use crate::xmir::Element;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Value, types};
use cranelift_codegen::{isa, settings};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::HashMap;

/// What every filled void holds.
type Env = HashMap<usize, Value>;

/// How deep an expression may nest before the compiler gives up, so that an
/// object applying itself cannot unfold forever.
const DEPTH: usize = 64;

/// The atoms that are one machine instruction, by the locator of the object
/// declaring them.
fn instruction(loc: &str) -> Option<Op> {
    match loc {
        "Φ.number.plus" => Some(Op::Add),
        "Φ.number.times" => Some(Op::Times),
        "Φ.number.div" => Some(Op::Div),
        _ => None,
    }
}

/// One arithmetic atom.
enum Op {
    Add,
    Times,
    Div,
}

impl Program {
    /// Compile one top-level object into an object file exporting `main`,
    /// which exits with the value of that object.
    pub fn compile(&self, locator: &str) -> Result<Vec<u8>, String> {
        let resolver = self.resolver();
        let object = self
            .documents
            .iter()
            .flat_map(|document| document.root().children.iter())
            .find(|object| attribute(object, "loc") == Some(locator))
            .ok_or_else(|| format!("no object at {locator}"))?;
        let body = child(object, "φ").ok_or_else(|| format!("{locator} has no φ"))?;
        let isa = isa::lookup(target_lexicon::Triple::host())
            .map_err(|e| e.to_string())?
            .finish(settings::Flags::new(settings::builder()))
            .map_err(|e| e.to_string())?;
        let frontend = isa.frontend_config();
        let mut module = ObjectModule::new(
            ObjectBuilder::new(isa, "eo", cranelift_module::default_libcall_names())
                .map_err(|e| e.to_string())?,
        );
        let mut context = module.make_context();
        context
            .func
            .signature
            .returns
            .push(AbiParam::new(types::I32));
        let mut shell = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut shell);
        let entry = builder.create_block();
        builder.switch_to_block(entry);
        builder.seal_block(entry);
        let mut env = Env::new();
        let value = emit(
            &resolver,
            &mut builder,
            body,
            Where::At(object),
            &mut env,
            0,
        )?;
        let code = builder.ins().fcvt_to_sint(types::I32, value);
        builder.ins().return_(&[code]);
        builder.finalize(frontend);
        let id = module
            .declare_function("main", Linkage::Export, &context.func.signature)
            .map_err(|e| e.to_string())?;
        module
            .define_function(id, &mut context)
            .map_err(|e| e.to_string())?;
        module.finish().emit().map_err(|e| e.to_string())
    }
}

/// Build the value of one expression.
///
/// `scope` is the formation the expression sits in, which `ξ` names, and `env`
/// holds the value handed to every void filled so far.
fn emit(
    resolver: &Resolver<'_>,
    builder: &mut FunctionBuilder,
    element: &Element,
    scope: Where<'_>,
    env: &mut Env,
    depth: usize,
) -> Result<Value, String> {
    if depth > DEPTH {
        return Err("the expression nests deeper than this compiles".to_string());
    }
    if let Some(text) = &element.text {
        return Ok(builder.ins().f64const(number(text)?));
    }
    let base = attribute(element, "base").ok_or("a formation where a value was wanted")?;
    if base == "∅" {
        return held(element, env);
    }
    if let Some(void) = void(base, scope) {
        return held(void, env);
    }
    let Where::At(target) = resolver.lands(Some(element), base, scope, 0) else {
        return Err(format!("{base} does not land anywhere known"));
    };
    if let Some(op) = attribute(target, "loc").and_then(instruction) {
        let left = receiver(resolver, builder, element, base, scope, env, depth)?;
        let right = emit(
            resolver,
            builder,
            argument(element, 0)?,
            scope,
            env,
            depth + 1,
        )?;
        return Ok(match op {
            Op::Add => builder.ins().fadd(left, right),
            Op::Times => builder.ins().fmul(left, right),
            Op::Div => builder.ins().fdiv(left, right),
        });
    }
    apply(resolver, builder, element, target, scope, env, depth)
}

/// Hand an application its arguments and build the body of what it applies.
///
/// Every argument is built where the call is written, and only then bound, so
/// a void of the formation being applied cannot be mistaken for one of the
/// caller. A formation whose body is one of its own voids falls out of this on
/// its own: binding the argument and then building the body is the argument.
fn apply(
    resolver: &Resolver<'_>,
    builder: &mut FunctionBuilder,
    element: &Element,
    formation: &Element,
    scope: Where<'_>,
    env: &mut Env,
    depth: usize,
) -> Result<Value, String> {
    let voids: Vec<&Element> = formation
        .children
        .iter()
        .filter(|child| attribute(child, "base") == Some("∅"))
        .collect();
    let mut handed = Vec::new();
    for (slot, void) in voids.iter().enumerate() {
        let value = emit(
            resolver,
            builder,
            argument(element, slot)?,
            scope,
            env,
            depth + 1,
        )?;
        handed.push((address(void), value));
    }
    for (void, value) in handed {
        env.insert(void, value);
    }
    let body = child(formation, "φ").ok_or("an application of something with no φ")?;
    emit(
        resolver,
        builder,
        body,
        Where::At(formation),
        env,
        depth + 1,
    )
}

/// The object a dispatch is made on: the child carrying no `as` when the chain
/// starts with a dot, and whatever the chain names up to its last step
/// otherwise.
fn receiver(
    resolver: &Resolver<'_>,
    builder: &mut FunctionBuilder,
    element: &Element,
    base: &str,
    scope: Where<'_>,
    env: &mut Env,
    depth: usize,
) -> Result<Value, String> {
    if base.starts_with('.') {
        let given = element
            .children
            .iter()
            .find(|child| attribute(child, "as").is_none())
            .ok_or("a dispatch with no receiver")?;
        return emit(resolver, builder, given, scope, env, depth + 1);
    }
    let (head, _) = base
        .rsplit_once('.')
        .ok_or_else(|| format!("{base} dispatches on nothing"))?;
    match void(head, scope) {
        Some(void) => held(void, env),
        None => Err(format!("{head} is not a receiver this compiles")),
    }
}

/// The void a chain names, when it names one of the scope's own.
fn void<'a>(base: &str, scope: Where<'a>) -> Option<&'a Element> {
    let name = base.strip_prefix("ξ.")?;
    if name.contains('.') {
        return None;
    }
    let Where::At(formation) = scope else {
        return None;
    };
    let binding = child(formation, name)?;
    (attribute(binding, "base") == Some("∅")).then_some(binding)
}

/// What a void was handed.
fn held(void: &Element, env: &Env) -> Result<Value, String> {
    env.get(&address(void))
        .copied()
        .ok_or_else(|| "a void nothing was handed to".to_string())
}

/// A binding's identity.
fn address(element: &Element) -> usize {
    std::ptr::from_ref(element) as usize
}

/// The argument in one position.
fn argument(element: &Element, slot: usize) -> Result<&Element, String> {
    element
        .children
        .iter()
        .find(|child| attribute(child, "as") == Some(&format!("α{slot}")))
        .ok_or_else(|| format!("no argument α{slot}"))
}

/// Read a datum written as big-endian bytes, `40-00-00-00-00-00-00-00`.
fn number(text: &str) -> Result<f64, String> {
    let bytes: Result<Vec<u8>, String> = text
        .trim()
        .split('-')
        .map(|byte| u8::from_str_radix(byte, 16).map_err(|e| e.to_string()))
        .collect();
    let bytes = bytes?;
    let eight: [u8; 8] = bytes
        .try_into()
        .map_err(|_| format!("{text} is not eight bytes"))?;
    Ok(f64::from_be_bytes(eight))
}
