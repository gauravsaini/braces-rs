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
reference, is run three ways:

- `cargo test --release -p braces-core` — the Rust core (conformance
  fixtures, multi-group regression fixtures, unit tests), all green
- `node napi/test-multigroup.js` — the multi-group pattern and 16
  variants byte-identical to the reference (71 assertions)
- `node tools/check-conformance.js` — the **exact** JS-level gate:
  1981/2008 fixtures byte-identical through the napi wrapper (value JSON
  equality, error name+message equality); the 27 remaining are corpus
  representation artifacts (generator recorded non-string inputs as
  `String()` text or logged result lengths), each documented in
  [tools/conformance-report.md](tools/conformance-report.md)

The only known divergences are documented in [DIVERGENCES.md](DIVERGENCES.md)
(non-string-input V8 message wording, off the Node-20 surface).

## Testing

Run the whole gate (on the box, where the binding is built):

```sh
# Rust core: conformance + multi-group + unit tests
cargo test --release -p braces-core
# exact napi-wrapper conformance (exit 0 = zero real divergences)
node tools/check-conformance.js
# multi-group byte-identity vs the JS reference
BRACES_REF=<path-to-micromatch-braces> node napi/test-multigroup.js
# or the package's own test script (from napi/)
cd napi && npm test
```

## Performance

Rust beats the JS reference on **every pattern of the required benchmark
ladder** — median ops/s over 5 runs, 3 s fixed-duration each,
5000-iteration warmup, Node 20.19.4, `rustc 1.98.1`, i7-7700:

| pattern | rust ops/s | js ops/s | speedup |
|---------|-----------:|---------:|--------:|
| literal (no braces) | 2,292,774 | 642,937 | 3.57x |
| small list `{a,b,c}` | 496,559 | 255,356 | 1.94x |
| padded range `{001..300}` | 73,713 | 44,879 | 1.64x |
| stepped range `{0..100..10}` | 749,035 | 472,424 | 1.59x |
| alpha range `{a..z}` | 832,353 | 428,622 | 1.94x |
| nested `{a,{b..d},e}` | 365,310 | 227,424 | 1.61x |
| big range `{1..1000}` | 23,429 | 16,615 | 1.41x |
| compile `{001..300}` | 1,082,637 | 673,952 | 1.61x |
| compile `{1..10..2}` | 1,070,797 | 675,425 | 1.59x |
| expandMany batch-100 | 1,674 | 1,374 | 1.22x |

Full methodology, machine info, and two extra patterns that do not beat
the reference (default-expand borderline at 0.99x–1.06x across runs; a 3000-output
multi-group case at 0.66x) are documented honestly in [BENCH.md](BENCH.md).

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
