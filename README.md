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

Rust beats the JS reference on **every pattern of the required benchmark
ladder** — median ops/s over 5 runs, 3 s fixed-duration each,
5000-iteration warmup, Node 20.19.4, `rustc 1.98.1`, i7-7700:

| pattern | rust ops/s | js ops/s | speedup |
|---------|-----------:|---------:|--------:|
| literal (no braces) | 2,221,028 | 653,044 | 3.40x |
| small list `{a,b,c}` | 522,894 | 278,241 | 1.88x |
| padded range `{001..300}` | 73,077 | 44,894 | 1.63x |
| stepped range `{0..100..10}` | 757,981 | 475,742 | 1.59x |
| alpha range `{a..z}` | 827,131 | 415,747 | 1.99x |
| nested `{a,{b..d},e}` | 364,009 | 234,752 | 1.55x |
| big range `{1..1000}` | 23,160 | 16,547 | 1.40x |
| compile `{001..300}` | 1,092,179 | 691,848 | 1.58x |
| compile `{1..10..2}` | 1,047,351 | 677,674 | 1.55x |
| expandMany batch-100 | 1,697 | 1,444 | 1.18x |

Full methodology, machine info, and two extra patterns that do not beat
the reference (default-expand tie at 0.99x; a 3000-output multi-group
case at 0.63x) are documented honestly in [BENCH.md](BENCH.md).

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
