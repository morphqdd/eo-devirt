//! Turning a resolved program into machine code.
//!
//! Every formation that gets applied becomes a function taking its voids as
//! parameters, so an object that applies itself is a call rather than an
//! unfolding. Values are doubles throughout, a truth being 1.0 or 0.0.
//! Everything outside that is refused rather than guessed at.

use crate::program::{Program, Resolver, Where, attribute, child};
use crate::xmir::Element;
use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{AbiParam, BlockArg, InstBuilder, Value, types};
use cranelift_codegen::{Context, isa, settings};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::HashMap;

/// What every filled void holds.
type Env = HashMap<usize, Value>;

/// Where in the program one expression is being built.
#[derive(Clone, Copy)]
struct Frame<'a> {
    /// The formation the expression sits in, which `ξ` names.
    scope: Where<'a>,
    /// How many expressions deep we already are.
    depth: usize,
}

impl<'a> Frame<'a> {
    /// The frame one expression further in.
    fn inner(self) -> Self {
        Self {
            scope: self.scope,
            depth: self.depth + 1,
        }
    }
}

/// How deep one expression may nest before the compiler gives up.
const DEPTH: usize = 64;

/// The atoms that are one machine instruction, by the locator of the object
/// declaring them.
fn instruction(loc: &str) -> Option<Op> {
    match loc {
        "Φ.number.plus" => Some(Op::Add),
        "Φ.number.times" => Some(Op::Times),
        "Φ.number.div" => Some(Op::Div),
        "Φ.number.gt" => Some(Op::Above),
        _ => None,
    }
}

/// The objects that hand a value back unchanged.
///
/// A number and its bytes are one and the same while values are unboxed, so
/// converting between them, or dataizing something already evaluated, is
/// nothing at all. Each is named rather than inferred, since being a no-op is
/// a property of this value model and not of the object.
fn passes(loc: &str) -> bool {
    matches!(loc, "Φ.dataized")
}

/// One atom that is a single instruction.
enum Op {
    Add,
    Times,
    Div,
    Above,
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
        let mut unit = Unit {
            resolver,
            module: ObjectModule::new(
                ObjectBuilder::new(isa, "eo", cranelift_module::default_libcall_names())
                    .map_err(|e| e.to_string())?,
            ),
            signed: HashMap::new(),
            pending: Vec::new(),
            frontend,
        };
        unit.entry(body, object)?;
        while let Some(formation) = unit.pending.pop() {
            unit.body(formation)?;
        }
        unit.module.finish().emit().map_err(|e| e.to_string())
    }
}

/// One object file being built, and what is known about the program going into
/// it.
struct Unit<'a> {
    resolver: Resolver<'a>,
    module: ObjectModule,
    signed: HashMap<usize, FuncId>,
    pending: Vec<&'a Element>,
    frontend: cranelift_codegen::isa::TargetFrontendConfig,
}

impl<'a> Unit<'a> {
    /// Build `main`, which evaluates one expression and exits with it.
    fn entry(&mut self, body: &'a Element, object: &'a Element) -> Result<(), String> {
        let mut context = self.module.make_context();
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
        let value = self.emit(
            &mut builder,
            body,
            Frame {
                scope: Where::At(object),
                depth: 0,
            },
            &mut env,
        )?;
        let code = builder.ins().fcvt_to_sint(types::I32, value);
        builder.ins().return_(&[code]);
        builder.finalize(self.frontend);
        self.define("main", Linkage::Export, &mut context)?;
        Ok(())
    }

    /// Build the function standing for one formation.
    fn body(&mut self, formation: &'a Element) -> Result<(), String> {
        let id = *self
            .signed
            .get(&address(formation))
            .ok_or("a formation compiled before it was declared")?;
        let voids = voids(formation);
        let mut context = self.module.make_context();
        context.func.signature = self.signature(voids.len());
        let mut shell = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut shell);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);
        let mut env = Env::new();
        for (slot, void) in voids.iter().enumerate() {
            env.insert(address(void), builder.block_params(entry)[slot]);
        }
        let body = child(formation, "φ").ok_or("a formation with no φ was applied")?;
        let value = self.emit(
            &mut builder,
            body,
            Frame {
                scope: Where::At(formation),
                depth: 0,
            },
            &mut env,
        )?;
        builder.ins().return_(&[value]);
        builder.finalize(self.frontend);
        self.module
            .define_function(id, &mut context)
            .map_err(|e| e.to_string())
    }

    /// Build the value of one expression.
    ///
    /// `scope` is the formation the expression sits in, which `ξ` names, and
    /// `env` holds the value bound to every void filled so far.
    fn emit(
        &mut self,
        builder: &mut FunctionBuilder,
        element: &'a Element,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Value, String> {
        if frame.depth > DEPTH {
            return Err("the expression nests deeper than this compiles".to_string());
        }
        if let Some(text) = &element.text {
            return Ok(builder.ins().f64const(number(text)?));
        }
        let base = attribute(element, "base").ok_or("a formation where a value was wanted")?;
        if base == "∅" {
            return held(element, env);
        }
        if let Some(void) = void(base, frame.scope) {
            return held(void, env);
        }
        if let Some(local) = local(base, frame.scope) {
            return self.emit(builder, local, frame.inner(), env);
        }
        if base.rsplit('.').next() == Some("if") {
            return self.branch(builder, element, base, frame, env);
        }
        if let Where::At(target) = self.resolver.lands(Some(element), base, frame.scope, 0) {
            if let Some(op) = attribute(target, "loc").and_then(instruction) {
                let left = self.receiver(builder, element, base, frame, env)?;
                let right = self.emit(builder, argument(element, 0)?, frame.inner(), env)?;
                return Ok(self.operate(builder, op, left, right));
            }
            if attribute(target, "loc").is_some_and(passes) {
                return self.emit(builder, argument(element, 0)?, frame.inner(), env);
            }
            return self.call(builder, element, target, &[], frame, env);
        }
        self.upon(builder, element, base, frame, env)
    }

    /// Build a dispatch made on a value rather than on an object the program
    /// declares.
    ///
    /// Every value here is a number, so the attribute is looked for on
    /// `Φ.number`, and the value it was dispatched on becomes the first
    /// argument. That is where the two shapes of number attribute meet: an
    /// atom like `plus` takes the value as its `ρ`, while an object like
    /// `minus` declares it as a void of its own, and both read the same way
    /// from the call site.
    fn upon(
        &mut self,
        builder: &mut FunctionBuilder,
        element: &'a Element,
        base: &str,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Value, String> {
        let wanted = base
            .rsplit('.')
            .next()
            .ok_or_else(|| format!("{base} names nothing"))?;
        if wanted == "as-bytes" {
            return self.receiver(builder, element, base, frame, env);
        }
        let Where::At(target) =
            self.resolver
                .lands(None, &format!("Φ.number.{wanted}"), Where::Nowhere, 0)
        else {
            return Err(format!("{base} does not land anywhere known"));
        };
        let value = self.receiver(builder, element, base, frame, env)?;
        if wanted == "as-bytes" {
            return Ok(value);
        }
        if let Some(op) = attribute(target, "loc").and_then(instruction) {
            let right = self.emit(builder, argument(element, 0)?, frame.inner(), env)?;
            return Ok(self.operate(builder, op, value, right));
        }
        self.call(builder, element, target, &[value], frame, env)
    }

    /// Build one instruction.
    fn operate(&self, builder: &mut FunctionBuilder, op: Op, left: Value, right: Value) -> Value {
        match op {
            Op::Add => builder.ins().fadd(left, right),
            Op::Times => builder.ins().fmul(left, right),
            Op::Div => builder.ins().fdiv(left, right),
            Op::Above => {
                let flag = builder.ins().fcmp(FloatCC::GreaterThan, left, right);
                let yes = builder.ins().f64const(1.0);
                let no = builder.ins().f64const(0.0);
                builder.ins().select(flag, yes, no)
            }
        }
    }

    /// Build a two-way branch.
    ///
    /// `if` is not an atom: `true` and `false` are both a `bool` holding a
    /// two-argument formation, differing only in which argument it hands back.
    /// Dispatching on one is therefore a choice between two expressions, and
    /// compiling it as a branch is what keeps the arm not taken unevaluated,
    /// which recursion depends on.
    fn branch(
        &mut self,
        builder: &mut FunctionBuilder,
        element: &'a Element,
        base: &str,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Value, String> {
        let asked = self.receiver(builder, element, base, frame, env)?;
        let zero = builder.ins().f64const(0.0);
        let flag = builder.ins().fcmp(FloatCC::NotEqual, asked, zero);
        let (yes, no, join) = (
            builder.create_block(),
            builder.create_block(),
            builder.create_block(),
        );
        builder.append_block_param(join, types::F64);
        builder.ins().brif(flag, yes, &[], no, &[]);
        builder.switch_to_block(yes);
        builder.seal_block(yes);
        let taken = self.emit(builder, argument(element, 0)?, frame.inner(), env)?;
        builder.ins().jump(join, &[BlockArg::Value(taken)]);
        builder.switch_to_block(no);
        builder.seal_block(no);
        let other = self.emit(builder, argument(element, 1)?, frame.inner(), env)?;
        builder.ins().jump(join, &[BlockArg::Value(other)]);
        builder.seal_block(join);
        builder.switch_to_block(join);
        Ok(builder.block_params(join)[0])
    }

    /// Build a call to the function standing for a formation.
    fn call(
        &mut self,
        builder: &mut FunctionBuilder,
        element: &'a Element,
        formation: &'a Element,
        prefix: &[Value],
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Value, String> {
        let mut handed = prefix.to_vec();
        for slot in 0..voids(formation).len().saturating_sub(prefix.len()) {
            handed.push(self.emit(builder, argument(element, slot)?, frame.inner(), env)?);
        }
        let id = self.declare(formation)?;
        let callee = self.module.declare_func_in_func(id, builder.func);
        let call = builder.ins().call(callee, &handed);
        Ok(builder.inst_results(call)[0])
    }

    /// The function standing for a formation, declared and queued for a body
    /// the first time it is asked for.
    fn declare(&mut self, formation: &'a Element) -> Result<FuncId, String> {
        if let Some(id) = self.signed.get(&address(formation)) {
            return Ok(*id);
        }
        let signature = self.signature(voids(formation).len());
        let id = self
            .module
            .declare_function(
                &name(formation, self.signed.len()),
                Linkage::Local,
                &signature,
            )
            .map_err(|e| e.to_string())?;
        self.signed.insert(address(formation), id);
        self.pending.push(formation);
        Ok(id)
    }

    /// The object a dispatch is made on: the child carrying no `as` when the
    /// chain starts with a dot, and whatever the chain names up to its last
    /// step otherwise.
    fn receiver(
        &mut self,
        builder: &mut FunctionBuilder,
        element: &'a Element,
        base: &str,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Value, String> {
        if base.starts_with('.') {
            let given = element
                .children
                .iter()
                .find(|child| attribute(child, "as").is_none())
                .ok_or("a dispatch with no receiver")?;
            return self.emit(builder, given, frame.inner(), env);
        }
        let (head, _) = base
            .rsplit_once('.')
            .ok_or_else(|| format!("{base} dispatches on nothing"))?;
        match void(head, frame.scope) {
            Some(void) => held(void, env),
            None => Err(format!("{head} is not a receiver this compiles")),
        }
    }

    /// A function of so many doubles, returning one.
    fn signature(&self, arity: usize) -> cranelift_codegen::ir::Signature {
        let mut signature = self.module.make_signature();
        for _ in 0..arity {
            signature.params.push(AbiParam::new(types::F64));
        }
        signature.returns.push(AbiParam::new(types::F64));
        signature
    }

    /// Declare and define a function under a name of its own.
    fn define(
        &mut self,
        name: &str,
        linkage: Linkage,
        context: &mut Context,
    ) -> Result<(), String> {
        let id = self
            .module
            .declare_function(name, linkage, &context.func.signature)
            .map_err(|e| e.to_string())?;
        self.module
            .define_function(id, context)
            .map_err(|e| e.to_string())
    }
}

/// The voids a formation declares, in the order arguments fill them.
fn voids(formation: &Element) -> Vec<&Element> {
    formation
        .children
        .iter()
        .filter(|child| attribute(child, "base") == Some("∅"))
        .collect()
}

/// A name for the function standing for a formation, taken from its locator so
/// that a disassembly reads like the program.
fn name(formation: &Element, seen: usize) -> String {
    let loc = attribute(formation, "loc").unwrap_or("object");
    let plain: String = loc
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{plain}_{seen}")
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

/// The binding a chain names, when it names one the scope declares itself and
/// that binding is an expression: not a void, which holds a value already, and
/// not a formation, which is something to apply rather than to build. The
/// parser leaves these behind whenever it desugars.
fn local<'a>(base: &str, scope: Where<'a>) -> Option<&'a Element> {
    let name = base.strip_prefix("ξ.")?;
    if name.contains('.') {
        return None;
    }
    let Where::At(formation) = scope else {
        return None;
    };
    let binding = child(formation, name)?;
    match attribute(binding, "base") {
        Some("∅") | None => None,
        Some(_) => Some(binding),
    }
}

/// What a void was bound to.
fn held(void: &Element, env: &Env) -> Result<Value, String> {
    env.get(&address(void))
        .copied()
        .ok_or_else(|| "a void nothing was bound to".to_string())
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
