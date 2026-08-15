# eo-devirt

An ahead-of-time compiler for EO. It reads XMIR, works out where each attribute
dispatch lands, and emits native code, spending what it worked out on direct
instructions instead of a lookup at run time.

## Compiling

`(2.plus 3).plus 4` in EO, through XMIR and the resolver, into an object file
that links with `cc` and exits with 9:

```
main:
  movabs $0x4000000000000000,%rsi   # 2.0
  movq   %rsi,%xmm4
  addsd  0x55(%rip),%xmm4           # 3.0
  addsd  0x5d(%rip),%xmm4           # 4.0
  cvttsd2si %xmm4,%eax
```

This is the arithmetic slice and nothing more: expressions the resolver pinned
down completely, made of number literals and the atoms that are one
instruction. No object graph, no laziness, no allocation, because a constant
expression needs none of them. Everything else is refused with a message rather
than guessed at.

The oracle is `eoc dataize`, which runs the same source through the Java
runtime. Comparing against it is the only check a binary can have, the format
no longer being the same on both ends:

```text
$ eoc dataize p1
[0x40220000-00000000-] = 9.0
```

The binary writes `9.0`, which agrees. It writes rather than exits with a code,
so the comparison holds for any number rather than only small whole ones.

## The runtime

`runtime/` is what a compiled program leans on while it runs, linked into the
binary as a static library. The compiler emits calls into it by name, so
everything there is `extern "C"` and unmangled.

It reaches the operating system through libc rather than raw syscalls. Windows
has no stable syscall numbering and needs the DLL route regardless, so the
second path would have to exist anyway, and libc is one path for both.

So far it holds one function, which writes out a dataized number.

## Where the dispatch goes

Measured over the 171 XMIR files of `eo-runtime`, 11997 dispatch steps in all:

| | steps | |
|---|---|---|
| resolved | 9057 (75.5%) | pinned to an object the program declares |
| dynamic | 2860 (23.8%) | goes through a value that exists only at run time |
| unresolved | 80 (0.7%) | named something the program does not declare |

Reproduce with `cargo run --example resolve <dir-with-xmir>`.

Three quarters of all dispatch is pinned down. The dynamic quarter is what a
run-time lookup has to carry.

What counts as what:

- A step onto `ρ` resolves to the formation the body is declared in. The `dot`
  rule binds `ρ` to the whole formation that held the attribute, and the `stay`
  rule refuses to rebind an `ρ` that is already bound, so the shape of `ρ` is
  fixed lexically no matter who does the dispatching. Only a top-level object
  has none, its `ρ` being `Φ` itself. This one rule accounts for most of what
  is resolved.
- A step through a void is dynamic, though finding the void itself is not: the
  binding is exactly where the program says it is, only its value is not.
- A step past an atom follows the shape its native code produces, which the
  `atom` attribute on its `λ` declares.
- A leading-dot dispatch takes its receiver from the child carrying no `as`,
  and that receiver has a shape like any other expression.
- A void takes the shape every call site puts into it, when they all agree.
- Not finding a name on a formation that hides native code, or that decorates
  something we could not follow, is dynamic rather than unresolved: it is not
  knowing, not absence. Getting this distinction wrong is what a wrong shape
  looks like, so the unresolved count doubles as an alarm.

## Inlining does not pay, and `ρ` is why

XMIR has no node meaning "call this body directly", so the only way to spend a
resolved dispatch is to move the body to the call site. That is sound only when
nothing in the body reads `ρ`, since `ρ` is bound to whatever the dispatch was
made on and means something else once the body sits elsewhere.

Over `eo-runtime`, **3 of 9776 dispatch sites qualify**. The bodies that resolve
almost always read `ρ`, hold a void, or hide native code behind a `λ`. Allowing
arguments at the call site does not help: the count stays at 3.

The three that do qualify make the program bigger, not smaller: +113 dispatch
nodes and +0.7% of text, because a moved body brings its own dispatches with it.

So inlining is not the lever. Everything routes back to `ρ`, which is also the
largest dynamic group. Proving what `ρ` is at a given site is the next thing
worth building, and it is shape analysis.

## Known approximation

The `dot` rule contextualizes a dispatched body against the formation *without*
the binding being dispatched, so a `ξ` self-reference inside that body does not
see the attribute it was reached by. This resolver contextualizes against the
whole formation, so it will resolve such a self-reference where the calculus
collapses it. Narrow, but it is a place where the numbers above read slightly
high.

## Leftovers

The 0.7% that is neither is not yet explained. The obvious guess, that these are
dispatches past atoms, is wrong: the rule for that case fires zero times on this
corpus and moved none of them.

## The round-trip guarantee

Losing information silently is the one failure the reader must not have, so it
refuses everything it does not model: mixed content, CDATA, processing
instructions, comments inside the tree, unknown entities. What it accepts, it
models in full, and the test asserts that parsing the printed form gives back an
equal document.

Known limitation: attribute values are kept exactly as written, escapes and all.
Round-tripping is unaffected, since they are written back the same way, but
whoever compares `base` values has to unescape them first.

Run it against a whole parsed code base:

```bash
XMIR_CORPUS=/path/to/.eoc/1-parse cargo test
```

To see the canonical form of one file:

```bash
cargo run --example canon file.xmir
```

## License

MIT
