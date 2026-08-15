# eo-devirt

A devirtualizer for EO programs: it reads XMIR, resolves the attribute
dispatches it can resolve statically, and writes XMIR back out.

Keeping the same format on both ends is the point. A transformed program can be
fed back into the normal EO build and compared against the original, so every
transformation has an oracle.

## Status

Stage 4 of 5: shapes.

- [x] read and write XMIR without losing anything
- [x] resolve `Φ.a.b` and `ξ.a` references, following decorators and packages
- [x] inline the dispatches whose body is safe to move -- 3 of 9776
- [x] work out the shape of `ρ`
- [ ] a native backend

## Where the dispatch goes

Measured over the 171 XMIR files of `eo-runtime`, 11997 dispatch steps in all:

| | steps | |
|---|---|---|
| resolved | 8789 (73.3%) | pinned to an object the program declares |
| dynamic | 3017 (25.1%) | goes through a value that exists only at run time |
| unresolved | 191 (1.6%) | named something not found |

Reproduce with `cargo run --example resolve <dir-with-xmir>`.

Three fifths of all dispatch comes off with plain name resolution, before any
analysis of shapes. The dynamic share is what stage 4 has to attack.

What counts as what:

- A step onto `ρ` resolves to the formation the body is declared in. The `dot`
  rule binds `ρ` to the whole formation that held the attribute, and the `stay`
  rule refuses to rebind an `ρ` that is already bound, so the shape of `ρ` is
  fixed lexically no matter who does the dispatching. Only a top-level object
  has none, its `ρ` being `Φ` itself. This is what took resolution from 60.9%
  to 73.3%.
- A step through a void is dynamic, though finding the void itself is not: the
  binding is exactly where the program says it is, only its value is not.
- A step past an atom is dynamic. The `atom` attribute on a `λ` binding does
  declare the shape of the result, which stage 4 should be able to use.

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

The 1.1% that is neither is not yet explained. The obvious guess, that these are
dispatches past atoms, is wrong: the rule for that case fires zero times on this
corpus and moved none of them.

## The round-trip guarantee

Losing information silently is the one failure this stage must not have, so the
reader refuses everything it does not model: mixed content, CDATA, processing
instructions, comments inside the tree, unknown entities. What it accepts, it
models in full, and the test asserts that parsing the printed form gives back an
equal document.

Known limitation: attribute values are kept exactly as written, escapes and all.
Round-tripping is unaffected, since they are written back the same way, but
whoever starts comparing `base` values in stage 2 has to unescape them first.

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
