//! Turning a resolved expression into machine code.
//!
//! This covers the arithmetic slice: expressions the resolver pinned down
//! completely, built out of number literals and the atoms that map onto a
//! single instruction. There is no object graph and no laziness yet, because a
//! constant expression needs neither. Everything else is refused rather than
//! guessed at.

use crate::program::{Program, Resolver, Where, attribute, child};
use crate::xmir::Element;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Value, types};
use cranelift_codegen::{isa, settings};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

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
        let value = emit(&resolver, &mut builder, body, Where::At(object))?;
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
fn emit(
    resolver: &Resolver<'_>,
    builder: &mut FunctionBuilder,
    element: &Element,
    scope: Where<'_>,
) -> Result<Value, String> {
    if let Some(text) = &element.text {
        return Ok(builder.ins().f64const(number(text)?));
    }
    let base = attribute(element, "base").ok_or("a formation where a value was wanted")?;
    let Where::At(target) = resolver.lands(Some(element), base, scope, 0) else {
        return Err(format!("{base} does not land anywhere known"));
    };
    if let Some(op) = attribute(target, "loc").and_then(instruction) {
        let left = emit(resolver, builder, receiver(element)?, scope)?;
        let right = emit(resolver, builder, argument(element, 0)?, scope)?;
        return Ok(match op {
            Op::Add => builder.ins().fadd(left, right),
            Op::Times => builder.ins().fmul(left, right),
            Op::Div => builder.ins().fdiv(left, right),
        });
    }
    match wrapper(target) {
        Some(slot) => emit(resolver, builder, argument(element, slot)?, scope),
        None => Err(format!("{base} is not part of the arithmetic slice")),
    }
}

/// Which argument a formation hands straight back, when all it does is wrap
/// one: its `φ` is one of its own voids, so applying it is the argument.
fn wrapper(formation: &Element) -> Option<usize> {
    let voids: Vec<&Element> = formation
        .children
        .iter()
        .filter(|child| attribute(child, "base") == Some("∅"))
        .collect();
    let body = child(formation, "φ")?;
    let wanted = match attribute(body, "base") {
        Some("∅") => body,
        Some(base) => {
            let name = base.strip_prefix("ξ.")?;
            child(formation, name)?
        }
        None => return None,
    };
    voids.iter().position(|void| std::ptr::eq(*void, wanted))
}

/// The object a dispatch is made on: the one child carrying no `as`.
fn receiver(element: &Element) -> Result<&Element, String> {
    element
        .children
        .iter()
        .find(|child| attribute(child, "as").is_none())
        .ok_or_else(|| "a dispatch with no receiver".to_string())
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
