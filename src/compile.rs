//! Turning a resolved program into machine code.
//!
//! Every formation that gets applied becomes a function taking its voids as
//! parameters, so an object that applies itself is a call rather than an
//! unfolding. Values are doubles throughout, a truth being 1.0 or 0.0.
//! Everything outside that is refused rather than guessed at.

use std::collections::HashMap;

use cranelift_codegen::{
    Context,
    ir::{AbiParam, BlockArg, InstBuilder, Value, condcodes::FloatCC, types},
    isa, settings,
    settings::Configurable as _,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::{
    program::{Program, Resolver, Where, attribute, child},
    xmir::Element,
};

/// What is known while one function is being built.
#[derive(Default)]
struct Env {
    /// What every filled void holds.
    filled: HashMap<usize, Val>,
    /// What every local already built came to, and the block it was built in.
    ///
    /// A local names one object however often it is mentioned, so building it
    /// again would make a second one, with its own slots and its own work to
    /// redo. The block is kept because CLIR is in SSA form: a value may only
    /// be used where its definition dominates, which the same block always
    /// does and another block may not, so a local first built inside one arm
    /// of a branch is built again in the other rather than borrowed across.
    built: HashMap<usize, (cranelift_codegen::ir::Block, Val)>,
}

impl Env {
    /// Start with nothing known.
    fn new() -> Self {
        Self::default()
    }

    /// Bind a void.
    fn insert(&mut self, void: usize, value: Val) {
        self.filled.insert(void, value);
    }

    /// What a void was bound to.
    fn get(&self, void: usize) -> Option<&Val> {
        self.filled.get(&void)
    }
}

/// One value a compiled program holds.
///
/// Numbers are unboxed doubles. Bytes are where they start and how many of
/// them there are, which is what a system call wants and what a literal can be
/// laid down as. Nothing here is allocated while the program runs.
#[derive(Clone, Copy)]
enum Val {
    /// A number, a truth being 1.0 or 0.0.
    Number(Value),
    /// Bytes, as where they start and how many of them there are.
    Bytes { at: Value, size: Value },
    /// An object, as where it is. Only what the resolver could not pin down
    /// takes this form, since an object costs a lookup where a number does not.
    Object(Value),
}

impl Val {
    /// The number this is, when it is one.
    fn number(self) -> Result<Value, String> {
        match self {
            Self::Number(value) => Ok(value),
            Self::Bytes { .. } => Err("bytes where a number was wanted".to_string()),
            Self::Object(_) => Err("an object where a number was wanted".to_string()),
        }
    }
}

/// What an expression will turn out to be, worked out before any code is
/// built, because a function's shape has to be settled before its body can
/// call itself.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A number, which travels unboxed.
    Number,
    /// An object, which travels as a pointer.
    Object,
}

/// Where in the program one expression is being built.
#[derive(Clone, Copy)]
struct Frame<'a> {
    /// The formation the expression sits in, which `ξ` names.
    scope: Where<'a>,
    /// How many expressions deep we already are.
    depth: usize,
}

impl<'a> Frame<'a> {
    /// The frame for the body of a formation being entered.
    const fn within(self, formation: &'a Element) -> Self {
        Self {
            scope: Where::At(formation),
            depth: self.depth + 1,
        }
    }

    /// The frame one expression further in.
    const fn inner(self) -> Self {
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

/// What everything the runtime reads is aligned to, a shape being read as
/// words rather than bytes.
const WORD: u64 = 8;

/// How many arguments a system call may be handed, which the runtime agrees
/// on.
const ARGUMENTS: usize = 4;

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
            .find_map(|object| at(object, locator))
            .ok_or_else(|| format!("no object at {locator}"))?;
        let body = child(object, "φ").ok_or_else(|| format!("{locator} has no φ"))?;
        let mut flags = settings::builder();
        flags.set("is_pic", "true").map_err(|e| e.to_string())?;
        let isa = isa::lookup(target_lexicon::Triple::host())
            .map_err(|e| e.to_string())?
            .finish(settings::Flags::new(flags))
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
            spelt: 0,
            interned: HashMap::new(),
            attributes: Vec::new(),
            shapes: HashMap::new(),
            frontend,
        };
        unit.entry(body, object)?;
        while !unit.pending.is_empty() || !unit.attributes.is_empty() {
            while let Some(formation) = unit.pending.pop() {
                unit.body(formation)?;
            }
            while let Some((binding, formation)) = unit.attributes.pop() {
                unit.body_of(binding, formation)?;
            }
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
    spelt: usize,
    interned: HashMap<String, u64>,
    attributes: Vec<(&'a Element, &'a Element)>,
    shapes: HashMap<usize, cranelift_module::DataId>,
    frontend: cranelift_codegen::isa::TargetFrontendConfig,
}

impl<'a> Unit<'a> {
    /// Build `main`, which evaluates one expression, writes it out through the
    /// runtime and exits.
    fn entry(&mut self, body: &'a Element, object: &'a Element) -> Result<(), String> {
        let mut writing = self.module.make_signature();
        writing.params.push(AbiParam::new(types::F64));
        let printer = self
            .module
            .declare_function("eo_print", Linkage::Import, &writing)
            .map_err(|e| e.to_string())?;
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
        let shown = self.unboxed(&mut builder, value)?;
        let callee = self.module.declare_func_in_func(printer, builder.func);
        builder.ins().call(callee, &[shown]);
        let fine = builder.ins().iconst(types::I32, 0);
        builder.ins().return_(&[fine]);
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
        context.func.signature = self.signature(formation)?;
        let boxing = self.gives(formation)? == types::I64;
        let mut shell = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut shell);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);
        let mut env = Env::new();
        for (slot, void) in voids.iter().enumerate() {
            env.insert(
                address(void),
                Val::Number(builder.block_params(entry)[slot]),
            );
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
        let handed = if boxing {
            self.boxed(&mut builder, value)?
        } else {
            value.number()?
        };
        builder.ins().return_(&[handed]);
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
        builder: &mut FunctionBuilder<'_>,
        element: &'a Element,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Val, String> {
        if frame.depth > DEPTH {
            return Err("the expression nests deeper than this compiles".to_string());
        }
        if let Some(text) = &element.text {
            return Ok(Val::Number(builder.ins().f64const(number(text)?)));
        }
        let base = attribute(element, "base").ok_or("a formation where a value was wanted")?;
        if base == "∅" {
            return held(element, env);
        }
        if let Some(void) = void(base, frame.scope) {
            return held(void, env);
        }
        if let Some(local) = local(base, frame.scope) {
            return self.built(builder, local, frame, env);
        }
        if base.rsplit('.').next() == Some("if") {
            if !self.truth(element, base, frame)? {
                return Err(format!(
                    "{base} asks something that is not a truth to choose"
                ));
            }
            return self.branch(builder, element, base, frame, env);
        }
        if let Where::At(target) = self.resolver.lands(Some(element), base, frame.scope, 0) {
            if let Some(op) = attribute(target, "loc").and_then(instruction) {
                let left = self
                    .receiver(builder, element, base, frame, env)?
                    .number()?;
                let right = self
                    .emit(builder, argument(element, 0)?, frame.inner(), env)?
                    .number()?;
                return Ok(Val::Number(Self::operate(builder, &op, left, right)));
            }
            if plain(target) {
                return self.object(builder, target);
            }
            if attribute(target, "loc") == Some("Φ.string") {
                return self.letters(builder, element);
            }
            if attribute(target, "loc") == Some("Φ.posix") {
                return self.syscall(builder, element, frame, env);
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
        builder: &mut FunctionBuilder<'_>,
        element: &'a Element,
        base: &str,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Val, String> {
        let wanted = base
            .rsplit('.')
            .next()
            .ok_or_else(|| format!("{base} names nothing"))?;
        let got = self.receiver(builder, element, base, frame, env)?;
        if wanted == "as-bytes" || wanted == "code" {
            return Ok(got);
        }
        if let Val::Bytes { size, .. } = got {
            if wanted == "size" {
                return Ok(Val::Number(builder.ins().fcvt_from_sint(types::F64, size)));
            }
            return Err(format!("bytes have no {wanted}"));
        }
        let numeric = self
            .resolver
            .lands(None, &format!("Φ.number.{wanted}"), Where::Nowhere, 0);
        let Where::At(target) = numeric else {
            return match got {
                Val::Object(object) => self.lookup(builder, object, wanted),
                _ => Err(format!("{base} does not land anywhere known")),
            };
        };
        let value = self.unboxed(builder, got)?;
        if let Some(op) = attribute(target, "loc").and_then(instruction) {
            let right = self.emit(builder, argument(element, 0)?, frame.inner(), env)?;
            let right = self.unboxed(builder, right)?;
            return Ok(Val::Number(Self::operate(builder, &op, value, right)));
        }
        self.call(builder, element, target, &[value], frame, env)
    }

    /// Build one instruction.
    fn operate(builder: &mut FunctionBuilder<'_>, op: &Op, left: Value, right: Value) -> Value {
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

    /// Whether what a dispatch is made on is a truth.
    ///
    /// `if` is compiled as a branch, which is right only for `bool`: `true`
    /// and `false` are the same object differing in which argument they hand
    /// back, so dispatching on one is a choice between two expressions. On
    /// anything else the name means whatever that object says it means, and a
    /// branch would quietly answer something else, so it is refused instead.
    fn truth(&self, element: &'a Element, base: &str, frame: Frame<'a>) -> Result<bool, String> {
        let landed = if base.starts_with('.') {
            let given = element
                .children
                .iter()
                .find(|child| attribute(child, "as").is_none())
                .ok_or("a dispatch with no receiver")?;
            self.resolver.lands(
                Some(given),
                attribute(given, "base").unwrap_or_default(),
                frame.scope,
                0,
            )
        } else {
            let (head, _) = base
                .rsplit_once('.')
                .ok_or_else(|| format!("{base} dispatches on nothing"))?;
            self.resolver.lands(None, head, frame.scope, 0)
        };
        if let Where::At(target) = landed {
            return Ok(self.bool(target, 0));
        }
        // The receiver went through a value, so the name at the end of its
        // chain is an attribute of `number`, which is where to ask instead.
        let chain = if base.starts_with('.') {
            element
                .children
                .iter()
                .find(|child| attribute(child, "as").is_none())
                .and_then(|given| attribute(given, "base"))
                .unwrap_or_default()
        } else {
            base.rsplit_once('.').map_or("", |(head, _)| head)
        };
        let Some(last) = chain.rsplit('.').next() else {
            return Ok(false);
        };
        match self
            .resolver
            .lands(None, &format!("Φ.number.{last}"), Where::Nowhere, 0)
        {
            Where::At(target) => Ok(self.bool(target, 0)),
            _ => Ok(false),
        }
    }

    /// Whether a formation is a truth, or stands for one.
    fn bool(&self, formation: &'a Element, depth: usize) -> bool {
        if depth > DEPTH {
            return false;
        }
        if matches!(
            attribute(formation, "loc"),
            Some("Φ.bool" | "Φ.true" | "Φ.false")
        ) {
            return true;
        }
        if let Some(lambda) = child(formation, "λ") {
            return attribute(lambda, "atom") == Some("Φ.bool");
        }
        match child(formation, "φ") {
            Some(body) => match self.resolver.lands(
                Some(body),
                attribute(body, "base").unwrap_or_default(),
                Where::At(formation),
                0,
            ) {
                Where::At(next) => !std::ptr::eq(next, formation) && self.bool(next, depth + 1),
                _ => false,
            },
            None => false,
        }
    }

    /// What an expression will turn out to be, worked out without building
    /// anything.
    ///
    /// A function's shape has to be settled before its body is built, since
    /// the body may call the function it is the body of. So this answers the
    /// same question `emit` answers, one step ahead of it and on the tree
    /// alone.
    fn kind(&self, element: &'a Element, frame: Frame<'a>) -> Result<Kind, String> {
        if frame.depth > DEPTH {
            return Ok(Kind::Number);
        }
        if element.text.is_some() {
            return Ok(Kind::Number);
        }
        let Some(base) = attribute(element, "base") else {
            return Ok(Kind::Object);
        };
        if base == "∅" || void(base, frame.scope).is_some() {
            return Ok(Kind::Number);
        }
        if let Some(local) = local(base, frame.scope) {
            return self.kind(local, frame.inner());
        }
        if base.rsplit('.').next() == Some("if") {
            let taken = self.kind(argument(element, 0)?, frame.inner())?;
            let other = self.kind(argument(element, 1)?, frame.inner())?;
            return Ok(if taken == Kind::Object || other == Kind::Object {
                Kind::Object
            } else {
                Kind::Number
            });
        }
        let Where::At(target) = self.resolver.lands(Some(element), base, frame.scope, 0) else {
            return Ok(Kind::Number);
        };
        if plain(target) {
            return Ok(Kind::Object);
        }
        if attribute(target, "loc").and_then(instruction).is_some()
            || attribute(target, "loc") == Some("Φ.posix")
            || attribute(target, "loc") == Some("Φ.string")
        {
            return Ok(Kind::Number);
        }
        if attribute(target, "loc").is_some_and(passes) {
            return self.kind(argument(element, 0)?, frame.inner());
        }
        match child(target, "φ") {
            Some(body) => self.kind(body, frame.within(target)),
            None => Ok(Kind::Number),
        }
    }

    /// What kind of value the function standing for a formation hands back.
    fn gives(&self, formation: &'a Element) -> Result<types::Type, String> {
        let body = child(formation, "φ").ok_or("a formation with no φ was applied")?;
        Ok(
            match self.kind(
                body,
                Frame {
                    scope: Where::At(formation),
                    depth: 0,
                },
            )? {
                Kind::Object => types::I64,
                Kind::Number => types::F64,
            },
        )
    }

    /// Build a local, or hand back what it came to when it was built already
    /// in the block being built now.
    fn built(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        local: &'a Element,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Val, String> {
        let here = builder
            .current_block()
            .ok_or("a local built outside any block")?;
        if let Some((block, value)) = env.built.get(&address(local))
            && *block == here
        {
            return Ok(*value);
        }
        let value = self.emit(builder, local, frame.inner(), env)?;
        env.built.insert(address(local), (here, value));
        Ok(value)
    }

    /// Build an object: room of its shape, and nothing put in it.
    ///
    /// The slots stay empty. An attribute is a body that runs the first time
    /// something asks for it, so making an object costs one allocation and
    /// nothing else, however much work its attributes would be.
    fn object(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        formation: &'a Element,
    ) -> Result<Val, String> {
        let shape = self.shape(formation)?;
        let word = self.module.declare_data_in_func(shape, builder.func);
        let at = builder.ins().symbol_value(types::I64, word);
        let made = self.calling("eo_make", &[types::I64], Some(types::I64))?;
        let callee = self.module.declare_func_in_func(made, builder.func);
        let call = builder.ins().call(callee, &[at]);
        Ok(Val::Object(builder.inst_results(call)[0]))
    }

    /// Lay down the shape of a formation: how many attributes, then the number
    /// each of their names was interned to, then the body of each.
    fn shape(&mut self, formation: &'a Element) -> Result<cranelift_module::DataId, String> {
        if let Some(id) = self.shapes.get(&address(formation)) {
            return Ok(*id);
        }
        let count = formation.children.len();
        let mut bodies = Vec::new();
        for binding in &formation.children {
            bodies.push(self.attribute(binding, formation)?);
        }
        let mut words = (count as u64).to_ne_bytes().to_vec();
        for binding in &formation.children {
            let name = attribute(binding, "name").ok_or("an attribute with no name")?;
            let number = self.intern(name);
            words.extend_from_slice(&number.to_ne_bytes());
        }
        words.extend(std::iter::repeat_n(0u8, count * WORD as usize));
        let id = self
            .module
            .declare_data(
                &format!("eo_shape_{}", self.spelt),
                Linkage::Local,
                false,
                false,
            )
            .map_err(|e| e.to_string())?;
        self.spelt += 1;
        let mut description = cranelift_module::DataDescription::new();
        description.set_align(WORD);
        description.define(words.into_boxed_slice());
        for (slot, body) in bodies.iter().enumerate() {
            let reference = self.module.declare_func_in_data(*body, &mut description);
            let at = (1 + count + slot) * WORD as usize;
            description.write_function_addr(at as u32, reference);
        }
        self.module
            .define_data(id, &description)
            .map_err(|e| e.to_string())?;
        self.shapes.insert(address(formation), id);
        Ok(id)
    }

    /// Declare the function standing for one attribute, and queue its body.
    ///
    /// It takes the object it was dispatched from and hands back an object,
    /// which is the shape every attribute has, whatever it holds.
    fn attribute(
        &mut self,
        binding: &'a Element,
        formation: &'a Element,
    ) -> Result<FuncId, String> {
        if let Some(id) = self.signed.get(&address(binding)) {
            return Ok(*id);
        }
        let signature = self.calling_shape(&[types::I64], Some(types::I64));
        let id = self
            .module
            .declare_function(
                &name(binding, self.signed.len()),
                Linkage::Local,
                &signature,
            )
            .map_err(|e| e.to_string())?;
        self.signed.insert(address(binding), id);
        self.attributes.push((binding, formation));
        Ok(id)
    }

    /// Build the function standing for one attribute.
    fn body_of(&mut self, binding: &'a Element, formation: &'a Element) -> Result<(), String> {
        let id = *self
            .signed
            .get(&address(binding))
            .ok_or("an attribute compiled before it was declared")?;
        let mut context = self.module.make_context();
        context.func.signature = self.calling_shape(&[types::I64], Some(types::I64));
        let mut shell = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut shell);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);
        let mut env = Env::new();
        let value = self.emit(
            &mut builder,
            binding,
            Frame {
                scope: Where::At(formation),
                depth: 0,
            },
            &mut env,
        )?;
        let handed = self.boxed(&mut builder, value)?;
        builder.ins().return_(&[handed]);
        builder.finalize(self.frontend);
        self.module
            .define_function(id, &mut context)
            .map_err(|e| e.to_string())
    }

    /// The shape of a call into or out of the runtime.
    fn calling_shape(
        &self,
        takes: &[types::Type],
        gives: Option<types::Type>,
    ) -> cranelift_codegen::ir::Signature {
        let mut signature = self.module.make_signature();
        for each in takes {
            signature.params.push(AbiParam::new(*each));
        }
        if let Some(gives) = gives {
            signature.returns.push(AbiParam::new(gives));
        }
        signature
    }

    /// The number a name is known by, the same everywhere in one program.
    ///
    /// `φ` gets zero, which the runtime holds back for it: an object that does
    /// not hold a name carries on through its decorator, and the runtime has
    /// to know which slot that is without being told a name.
    fn intern(&mut self, name: &str) -> u64 {
        if name == "φ" {
            return 0;
        }
        let next = self.interned.len() as u64 + 1;
        *self.interned.entry(name.to_string()).or_insert(next)
    }

    /// Put a value into the form an object slot holds.
    fn boxed(&mut self, builder: &mut FunctionBuilder<'_>, value: Val) -> Result<Value, String> {
        match value {
            Val::Object(at) => Ok(at),
            Val::Number(number) => {
                let wrap = self.calling("eo_number", &[types::F64], Some(types::I64))?;
                let callee = self.module.declare_func_in_func(wrap, builder.func);
                let call = builder.ins().call(callee, &[number]);
                Ok(builder.inst_results(call)[0])
            }
            Val::Bytes { .. } => Err("bytes where an object was wanted".to_string()),
        }
    }

    /// Take a number back out of an object.
    fn unboxed(&mut self, builder: &mut FunctionBuilder<'_>, value: Val) -> Result<Value, String> {
        match value {
            Val::Object(at) => {
                let read = self.calling("eo_as_number", &[types::I64], Some(types::F64))?;
                let callee = self.module.declare_func_in_func(read, builder.func);
                let call = builder.ins().call(callee, &[at]);
                Ok(builder.inst_results(call)[0])
            }
            other => other.number(),
        }
    }

    /// Look a name up on an object while the program runs.
    fn lookup(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        object: Value,
        name: &str,
    ) -> Result<Val, String> {
        let number = self.intern(name);
        let named = builder.ins().iconst(types::I64, number as i64);
        let find = self.calling("eo_dispatch", &[types::I64, types::I64], Some(types::I64))?;
        let callee = self.module.declare_func_in_func(find, builder.func);
        let call = builder.ins().call(callee, &[object, named]);
        Ok(Val::Object(builder.inst_results(call)[0]))
    }

    /// Declare one of the runtime's own functions.
    fn calling(
        &mut self,
        name: &str,
        takes: &[types::Type],
        gives: Option<types::Type>,
    ) -> Result<FuncId, String> {
        let signature = self.calling_shape(takes, gives);
        self.module
            .declare_function(name, Linkage::Import, &signature)
            .map_err(|e| e.to_string())
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
        builder: &mut FunctionBuilder<'_>,
        element: &'a Element,
        base: &str,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Val, String> {
        let asked = self
            .receiver(builder, element, base, frame, env)?
            .number()?;
        let zero = builder.ins().f64const(0.0);
        let flag = builder.ins().fcmp(FloatCC::NotEqual, asked, zero);
        let boxing = self.kind(argument(element, 0)?, frame.inner())? == Kind::Object
            || self.kind(argument(element, 1)?, frame.inner())? == Kind::Object;
        let (yes, no, join) = (
            builder.create_block(),
            builder.create_block(),
            builder.create_block(),
        );
        builder.append_block_param(join, if boxing { types::I64 } else { types::F64 });
        builder.ins().brif(flag, yes, &[], no, &[]);
        for (slot, block) in [yes, no].into_iter().enumerate() {
            builder.switch_to_block(block);
            builder.seal_block(block);
            let arm = self.emit(builder, argument(element, slot)?, frame.inner(), env)?;
            let carried = if boxing {
                self.boxed(builder, arm)?
            } else {
                arm.number()?
            };
            builder.ins().jump(join, &[BlockArg::Value(carried)]);
        }
        builder.seal_block(join);
        builder.switch_to_block(join);
        let landed = builder.block_params(join)[0];
        Ok(if boxing {
            Val::Object(landed)
        } else {
            Val::Number(landed)
        })
    }

    /// Build a system call.
    ///
    /// The name is a literal at every call site the runtime library has, so it
    /// is read here and laid down as a string for the runtime to match, rather
    /// than being built and read back while the program runs. The arguments
    /// arrive as a tuple, which is a chain of `tail`, `head` and `length`, so
    /// walking the tails and taking the heads gives them back in order.
    fn syscall(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        element: &'a Element,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Val, String> {
        let named = text(argument(element, 0)?).ok_or("a system call with no name to read")?;
        let mut handed = Vec::new();
        self.gather(builder, argument(element, 1)?, frame, env, &mut handed)?;
        if handed.len() > ARGUMENTS {
            return Err(format!("{named} is handed more arguments than this passes"));
        }
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(types::I64));
        signature.params.push(AbiParam::new(types::I64));
        for _ in 0..ARGUMENTS {
            signature.params.push(AbiParam::new(types::I64));
        }
        signature.returns.push(AbiParam::new(types::F64));
        let id = self
            .module
            .declare_function("eo_posix", Linkage::Import, &signature)
            .map_err(|e| e.to_string())?;
        let spelling = self.spell(&named)?;
        let word = self.module.declare_data_in_func(spelling, builder.func);
        let mut passed = vec![
            builder.ins().symbol_value(types::I64, word),
            builder.ins().iconst(types::I64, handed.len() as i64),
        ];
        let nothing = builder.ins().iconst(types::I64, 0);
        for slot in 0..ARGUMENTS {
            passed.push(match handed.get(slot) {
                Some(Val::Number(value)) => builder.ins().fcvt_to_sint(types::I64, *value),
                Some(Val::Bytes { at, .. }) => *at,
                Some(Val::Object(_)) => {
                    return Err("an object where a system call wanted a number".to_string());
                }
                None => nothing,
            });
        }
        let callee = self.module.declare_func_in_func(id, builder.func);
        let call = builder.ins().call(callee, &passed);
        Ok(Val::Number(builder.inst_results(call)[0]))
    }

    /// Build a string literal, which is bytes laid down once and pointed at.
    fn letters(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        element: &'a Element,
    ) -> Result<Val, String> {
        let raw = raw(argument(element, 0)?).ok_or("a string with nothing to read")?;
        let size = raw.len() as i64;
        let id = self.lay(raw)?;
        let word = self.module.declare_data_in_func(id, builder.func);
        Ok(Val::Bytes {
            at: builder.ins().symbol_value(types::I64, word),
            size: builder.ins().iconst(types::I64, size),
        })
    }

    /// Lay a name down where the runtime can read it, ended as C expects.
    fn spell(&mut self, named: &str) -> Result<cranelift_module::DataId, String> {
        let mut letters = named.as_bytes().to_vec();
        letters.push(0);
        self.lay(letters)
    }

    /// Lay bytes down in the object file.
    fn lay(&mut self, letters: Vec<u8>) -> Result<cranelift_module::DataId, String> {
        let id = self
            .module
            .declare_data(
                &format!("eo_letters_{}", self.spelt),
                Linkage::Local,
                false,
                false,
            )
            .map_err(|e| e.to_string())?;
        self.spelt += 1;
        let mut description = cranelift_module::DataDescription::new();
        description.set_align(WORD);
        description.define(letters.into_boxed_slice());
        self.module
            .define_data(id, &description)
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Walk a tuple, building each of its items in turn.
    fn gather(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        element: &'a Element,
        frame: Frame<'a>,
        env: &mut Env,
        into: &mut Vec<Val>,
    ) -> Result<(), String> {
        let base = attribute(element, "base").unwrap_or_default();
        if base.ends_with("tuple.empty") {
            return Ok(());
        }
        if !base.ends_with("tuple") {
            return Err(format!("{base} is not a tuple of arguments"));
        }
        self.gather(builder, argument(element, 0)?, frame, env, into)?;
        let item = self.emit(builder, argument(element, 1)?, frame.inner(), env)?;
        into.push(item);
        Ok(())
    }

    /// Build a call to the function standing for a formation.
    fn call(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        element: &'a Element,
        formation: &'a Element,
        prefix: &[Value],
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Val, String> {
        let mut handed = prefix.to_vec();
        let wanted = voids(formation).len().saturating_sub(prefix.len());
        for slot in 0..wanted {
            let Ok(arg) = argument(element, slot) else {
                return Err(format!(
                    "{} is handed {slot} of the {wanted} it takes, which is an object with a void left in it",
                    attribute(formation, "loc").unwrap_or("something")
                ));
            };
            handed.push(self.emit(builder, arg, frame.inner(), env)?.number()?);
        }
        let id = self.declare(formation)?;
        let callee = self.module.declare_func_in_func(id, builder.func);
        let call = builder.ins().call(callee, &handed);
        let given = builder.inst_results(call)[0];
        Ok(if self.gives(formation)? == types::I64 {
            Val::Object(given)
        } else {
            Val::Number(given)
        })
    }

    /// The function standing for a formation, declared and queued for a body
    /// the first time it is asked for.
    fn declare(&mut self, formation: &'a Element) -> Result<FuncId, String> {
        if let Some(id) = self.signed.get(&address(formation)) {
            return Ok(*id);
        }
        let signature = self.signature(formation)?;
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
        builder: &mut FunctionBuilder<'_>,
        element: &'a Element,
        base: &str,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Val, String> {
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
        self.along(builder, head, frame, env)
    }

    /// The value a chain names, walked from its start.
    ///
    /// A chain of more than one step has to be taken a step at a time: the
    /// start is a void, a local, or an object the program declares, and each
    /// step after it is a dispatch on whatever the step before came to.
    fn along(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        base: &str,
        frame: Frame<'a>,
        env: &mut Env,
    ) -> Result<Val, String> {
        if let Some(void) = void(base, frame.scope) {
            return held(void, env);
        }
        if let Some(local) = local(base, frame.scope) {
            return self.built(builder, local, frame, env);
        }
        if base.starts_with('Φ')
            && let Where::At(target) = self.resolver.lands(None, base, Where::Nowhere, 0)
        {
            return self.emit(builder, target, frame.within(target), env);
        }
        let (head, last) = base
            .rsplit_once('.')
            .ok_or_else(|| format!("{base} is not a receiver this compiles"))?;
        match self.along(builder, head, frame, env)? {
            Val::Object(object) => self.lookup(builder, object, last),
            _ => Err(format!("{head} is not an object to ask for {last}")),
        }
    }

    /// A function of so many doubles, handing back what its body will be.
    fn signature(
        &self,
        formation: &'a Element,
    ) -> Result<cranelift_codegen::ir::Signature, String> {
        let mut signature = self.module.make_signature();
        for _ in 0..voids(formation).len() {
            signature.params.push(AbiParam::new(types::F64));
        }
        signature
            .returns
            .push(AbiParam::new(self.gives(formation)?));
        Ok(signature)
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

/// The datum an expression wraps, however many wrappers deep, read as text.
fn text(element: &Element) -> Option<String> {
    String::from_utf8(raw(element)?).ok()
}

/// The datum an expression wraps, however many wrappers deep.
///
/// A single byte is written with a dash after it, `78-`, and empty bytes as
/// `--`, so the pieces between dashes can be empty and are passed over.
fn raw(element: &Element) -> Option<Vec<u8>> {
    if let Some(hex) = &element.text {
        return hex
            .trim()
            .split('-')
            .filter(|byte| !byte.is_empty())
            .map(|byte| u8::from_str_radix(byte, 16))
            .collect::<Result<Vec<u8>, _>>()
            .ok();
    }
    element
        .children
        .iter()
        .find(|child| attribute(child, "as") == Some("α0"))
        .and_then(raw)
}

/// The object one locator names, however deep it sits.
fn at<'a>(element: &'a Element, locator: &str) -> Option<&'a Element> {
    if attribute(element, "loc") == Some(locator) {
        return Some(element);
    }
    element.children.iter().find_map(|child| at(child, locator))
}

/// Whether a formation is one to hold rather than to apply: it declares no
/// voids to fill, so there is nothing to hand it and all it is is what it
/// holds. A `φ` is one of those attributes like any other, and the object it
/// decorates is asked only for names this one does not have.
fn plain(element: &Element) -> bool {
    attribute(element, "base").is_none()
        && !element.children.is_empty()
        && voids(element).is_empty()
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
fn held(void: &Element, env: &Env) -> Result<Val, String> {
    env.get(address(void))
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
