# eo-devirt

A devirtualizer for EO programs: it reads XMIR, resolves the attribute
dispatches it can resolve statically, and writes XMIR back out.

Keeping the same format on both ends is the point. A transformed program can be
fed back into the normal EO build and compared against the original, so every
transformation has an oracle.

## Status

Stage 1 of 5: the XMIR codec.

- [x] read and write XMIR without losing anything
- [ ] resolve `Φ.a.b` and `ξ.a` references into a graph
- [ ] inline the dispatches that turn out to be monomorphic
- [ ] shape analysis for the rest
- [ ] a native backend

## Why bother

Counted over the 171 XMIR files of `eo-runtime`, of 5231 dispatch steps:

| | steps | |
|---|---|---|
| first step from `Φ`, a global object | 2039 | static |
| first step from `ξ`, a lexical binding | 1248 | static |
| tails after `Φ`, the object is known | 223 | static |
| **resolvable with no analysis at all** | **3510 (67%)** | |
| first step on `ρ`, a computed receiver | 506 | needs shape analysis |
| tails after `ξ`, going through a value | 1215 | needs shape analysis |

Two thirds of all dispatch comes off with plain name resolution. That number is
a lower bound: part of the remaining third resolves under analysis.

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
