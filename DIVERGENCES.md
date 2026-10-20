# Divergences from micromatch/braces v3

braces-rs aims for byte-for-byte parity. Under the **exact** napi-wrapper
checker (`tools/check-conformance.js`: JSON value equality, error
name+message equality), 1981/2008 corpus fixtures pass; the 27 remaining
are corpus representation artifacts (§6), not implementation divergences.
The following are the only known intentional or engine-forced divergences.

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

- 1981 of 2008 corpus fixtures are byte-identical through the napi
  wrapper under **exact** comparison (value JSON equality, error
  name+message equality) — verified by `tools/check-conformance.js`
  (see `tools/conformance-report.md`). The remaining 27 are corpus
  representation artifacts (recorded inputs the generator's `String()`
  encoding cannot express; §6).
- Documented upstream quirks are reproduced faithfully, including:
  `{1..10..3}` with `rangeLimit: 5` NOT throwing (the `exceedsLimit`
  positional-argument quirk), `]` always compiled to `\]`, NBSP/ZWNBSP
  stripping, dollar braces kept literal, trailing-backslash `"undefined"`
  text, `expand('')` → `[]`, ranges with interior escapes not expanding,
  and regex-range's character-class merging in compile output.

## 6. Corpus representation artifacts (27)

The conformance generator recorded 27 calls with non-string inputs but
stored `String(input)` in the fixture's `pattern` field, so no string-arg
implementation can reproduce those recordings. The exact checker
(`tools/check-conformance.js`) classifies each of them with a reason;
`tools/conformance-report.md` lists all 27 individually:

- 18 × non-string inputs to parse/expand/compile — `null`, `undefined`,
  number `123`, boolean `true`, object `{}` (`"[object Object]"`), array
  `['x']` (`"x"`). (The wrapper reproduces the exact V8-worded errors
  when these values are passed as actual non-strings — §1 above.)
- 4 × non-string inputs to `create` — number `0`, `null`.
- 4 × array inputs to the default fn — `["{a,b}","{1..3}"]` (×2),
  `["{a,a}","b{a,a}"]` with dedupe, `[]`.
- 1 × `expand("{1..5000}", {rangeLimit:false})` — generator recorded
  `.length` (5000) instead of the array.
