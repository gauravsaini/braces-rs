# Divergences from micromatch/braces v3

braces-rs aims for byte-for-byte parity. Everything in the 2008-fixture
conformance corpus passes — in the Rust core tests and in the JS-level
smoke test through the napi wrapper (`napi/smoke.js`, 2008/2008). The
following are the only known intentional or engine-forced divergences.

## 1. Non-string-input error messages (V8-internal strings)

The reference crashes inside V8 with engine-specific messages for
non-string inputs, e.g.:

- `expand(null)` → `TypeError: Cannot set properties of null (setting 'queue')`
- `expand(123)` → `TypeError: Cannot create property 'queue' on number '123'`
- `compile(null)` → `TypeError: Cannot read properties of null (reading 'invalid')`
- `create(null)` → `TypeError: Cannot read properties of null (reading 'length')`
- `expand({})` / `expand(['x'])` → `TypeError: Cannot read properties of undefined (reading 'length')`

**Status:** the hand-written wrapper (`napi/index.js`) reproduces these
messages with the current V8 wording (Node ≥17), so callers on Node 20 see
byte-identical errors. The Rust core itself reports a stable
`TypeError: braces: expected a string or AST, got <kind>` for such inputs;
only the wrapper-level surface guarantees the V8 wording. If a future V8
changes its wording, the wrapper messages will still read as the Node-20
phrasing while the real reference (on that newer Node) would differ.

`parse(non-string)` is not a divergence: the reference itself throws
`TypeError: Expected a string`, and so does braces-rs.

## 2. `transform` calling convention on the napi fast path

In the reference, `options.transform` is a JS function called as
`transform(value, index, options)` and its return value is used verbatim
(a function returning a number inserts the number into the string).

braces-rs supports both:

- **JS function** — called back across the FFI boundary. Faithful,
  including the case where the function returns a non-string (the
  reference then crashes with `input.padStart is not a function` when
  padding applies; braces-rs reproduces that error).
- **String tag** (`'double' | 'alphaFoo' | 'prefixN'`) — a native fast
  path used by the conformance corpus; not part of the upstream API.

## 3. Error construction cost

The reference allocates a V8 error object with a stack trace for every
thrown error. braces-rs throws real typed JS errors too (RangeError /
SyntaxError / TypeError, reconstructed in the wrapper), so `instanceof`
and `.name` behave identically.

## 4. Bulk-result transport (internal, not observable)

Multi-string results (`expand`, `braces()`, `expandAst`, `expandMany`)
cross the napi boundary packed into a single string —
`[count: u32 as 2 chars][end-offsets: count × u32 as 2 chars][data]` —
which the wrapper unpacks with `String.prototype.substring`. This is a
pure transport optimisation (per-string N-API creation costs ~134ns,
more than the reference's per-element budget): the public API returns
plain `string[]` / `string[][]` exactly as the reference does.

## 5. What is NOT diverged

- All 2008 corpus fixtures: outputs are byte-identical, including error
  class + message for RangeError (rangeLimit), SyntaxError (maxLength),
  and TypeError (parse) cases.
- Documented upstream quirks are reproduced faithfully, including:
  `{1..10..3}` with `rangeLimit: 5` NOT throwing (the `exceedsLimit`
  positional-argument quirk), `]` always compiled to `\]`, NBSP/ZWNBSP
  stripping, dollar braces kept literal, trailing-backslash `"undefined"`
  text, `expand('')` → `[]`, ranges with interior escapes not expanding,
  and regex-range's character-class merging in compile output.
