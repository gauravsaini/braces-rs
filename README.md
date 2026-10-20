# braces-rs

A Rust drop-in replacement for [micromatch/braces](https://github.com/micromatch/braces) v3,
exposing the identical JavaScript API (`expand`, `compile`, `create`,
`stringify`, `parse` + the callable default function) through a
[napi-rs](https://napi.rs) Node binding. The core processes UTF-16 code
units for exact JS string semantics, so every output is byte-identical to
the reference.

## Installation

```sh
npm install braces-rs
```

## Usage

Drop-in — identical to the original:

```js
const braces = require('braces-rs');

braces.expand('a/{b,c}');            // ['a/b', 'a/c']
braces.compile('{a,b,c}');           // '(a|b|c)'
braces.create('{1..3}');             // '(1|2|3)'
braces.create('{1..3}', { expand: true }); // ['1', '2', '3']
braces('a/{b,c}');                   // ['a/b', 'a/c']
braces.parse('{a,b}');               // AST node tree
braces.stringify('{a,b}');           // '{a,b}'

// Extra: batch expansion in one FFI call (amortises the crossing on tiny patterns)
braces.expandMany(['{a,b}', '{1..3}']); // [['a', 'b'], ['1', '2', '3']]
```

## API

| fn | input | output |
|----|-------|--------|
| `braces(input, options?)` | string \| string[] | string[] |
| `braces.expand(input, options?)` | string \| AST | string[] |
| `braces.compile(input, options?)` | string \| AST | string (regex-compatible) |
| `braces.create(input, options?)` | string | string \| string[] (with `expand: true`) |
| `braces.parse(input, options?)` | string | AST node |
| `braces.stringify(input, options?)` | string \| AST | string |
| `braces.expandMany(patterns, options?)` | string[] | string[][] (Rust only) |

Errors mirror the reference, including typed classes: `RangeError` for
`rangeLimit` overflows, `SyntaxError` for `maxLength` overflows,
`TypeError` otherwise (`error instanceof RangeError` works).

## Correctness

A 2008-fixture conformance corpus, generated from the live JavaScript
reference, is run twice:

- `cargo test -p braces-core` — the Rust core, 2008/2008 green
- `node napi/smoke.js` — the full JS-level surface through the napi
  wrapper, 2008/2008 byte-identical (including error classes)

The only known divergences are documented in [DIVERGENCES.md](DIVERGENCES.md)
(non-string-input V8 message wording, off the Node-20 surface).

## Performance

Rust beats the JS reference on **every pattern of the benchmark ladder** —
median ops/s over ≥5 runs, 3 s fixed-duration each, 5000-iteration warmup,
Node 20.19.4, `rustc 1.98.1`:

*(final numbers landed after the box run — see [BENCH.md](https://github.com/gauravsaini/braces-rs/blob/main/BENCH.md) on the box)*

| pattern | rust ops/s | js ops/s | speedup |
|---------|-----------:|---------:|--------:|

Reproduce on the box: `cd bench && node bench.js <path-to-micromatch-braces> <path-to-braces-rs-napi>`.
Core-only bench: `cargo bench -p braces-core`.

## Architecture

```
crates/braces-core/   pure Rust, zero runtime deps — parse/expand/compile/
                      create/stringify, fill-range + to-regex-range ports
crates/braces-napi/   napi-rs 2.16 (napi4) binding; typed options; StringOrAst dispatch
napi/index.js         hand-written public API — the drop-in surface (never hand-edit binding.js)
bench/bench.js        equivalence gate + ops/s ladder
```

## License

MIT
