# eo-devirt

A devirtualizer for EO programs: it reads XMIR, resolves the attribute
dispatches it can resolve statically, and writes XMIR back out.

Keeping the same format on both ends is the point. A transformed program can be
fed back into the normal EO build and compared against the original, so every
transformation has an oracle.

## Status

Stage 2 of 5: name resolution.

- [x] read and write XMIR without losing anything
- [x] resolve `Φ.a.b` and `ξ.a` references, following decorators and packages
- [ ] inline the dispatches that turn out to be monomorphic
- [ ] shape analysis for the rest
- [ ] a native backend

## Where the dispatch goes

Measured over the 171 XMIR files of `eo-runtime`, 11997 dispatch steps in all:

| | steps | |
|---|---|---|
| resolved | 7303 (60.9%) | pinned to an object the program declares |
| dynamic | 4567 (38.1%) | goes through a value that exists only at run time |
| unresolved | 127 (1.1%) | named something not found |

Reproduce with `cargo run --example resolve <dir-with-xmir>`.

Three fifths of all dispatch comes off with plain name resolution, before any
analysis of shapes. The dynamic share is what stage 4 has to attack.

What counts as what:

- A step onto `ρ` is dynamic. `ρ` is the object a formation was dispatched
  from, bound by the `dot` rule at reduction time. It is usually the enclosing
  formation, but nothing in the program guarantees that, so it is not guessed
  at here. This is the single largest dynamic group.
- A step through a void is dynamic, though finding the void itself is not: the
  binding is exactly where the program says it is, only its value is not.
- A step past an atom is dynamic. The `atom` attribute on a `λ` binding does
  declare the shape of the result, which stage 4 should be able to use.

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
