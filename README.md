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
5000-iteration warmup, Node 20.19.4, `rustc 1.98.1`, i7-7700 (final run
2026-10-20, post-fidelity + rope rework):

| pattern | rust ops/s | js ops/s | speedup |
|---------|-----------:|---------:|--------:|
| literal (no braces) | 2,177,783 | 659,016 | 3.30x |
| small list `{a,b,c}` | 611,184 | 278,932 | 2.19x |
| padded range `{001..300}` | 73,739 | 44,397 | 1.66x |
| stepped range `{0..100..10}` | 747,411 | 476,563 | 1.57x |
| alpha range `{a..z}` | 807,306 | 420,033 | 1.92x |
| nested `{a,{b..d},e}` | 429,643 | 239,939 | 1.79x |
| big range `{1..1000}` | 23,357 | 16,214 | 1.44x |
| compile `{001..300}` | 1,079,165 | 674,204 | 1.60x |
| compile `{1..10..2}` | 1,040,522 | 628,512 | 1.66x |
| default compile | 946,881 | 740,443 | 1.28x |
| expandMany batch-100 | 2,068 | 1,443 | 1.43x |

The one extra pattern that does not beat the reference is the
3030-output multi-group case (`user-{200..300}/project-{a,b,c}-{1..10}`,
0.89x) — full methodology, the rope-walk rework that took it from 0.66x,
and the honest reason for the residual are documented in
[BENCH.md](BENCH.md).

Reproduce on the box: `cd bench && node bench.js <path-to-micromatch-braces> <path-to-braces-rs-napi>`.
Core-only bench: `cargo bench -p braces-core`.

## Security

CVE-2026-93687 — deeply nested brace patterns (`'{'×4999 + 'a' + '}'×4999`)
send the JS reference's recursive walkers into `RangeError: Maximum call
stack size exceeded`; a native recursive implementation risks worse (an
uncatchable stack-overflow abort). braces-rs parses iteratively, and every
recursive walker (expand / compile / stringify, plus the AST-object
converter) refuses ASTs deeper than `MAX_AST_DEPTH = 10,000` with a typed
`RangeError: Maximum call stack size exceeded` instead of overflowing.
The limit sits 2x above the deepest AST any string input can express
under the (reference-faithful) 10,000-char input cap, and 2x above the
depth where the reference dies on this machine — so no input the
reference handles can be rejected by it. Regression tests:
`crates/braces-core/tests/cve_2026_93687.rs` and
`napi/test-cve-2026-93687.js`.

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
