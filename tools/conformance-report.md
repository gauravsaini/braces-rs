# Conformance report — exact napi-wrapper checker

Checker: `tools/check-conformance.js` (run on the box against the built
binding). Semantics are EXACT — stricter than the Rust-core test and the
old `napi/smoke.js` claims:

- value fixtures: `JSON.stringify(got) === JSON.stringify(expected)`
- error fixtures: `` `${err.name}: ${err.message}` `` === corpus `error`
  string (the corpus stores errors as one `"Name: message"` string)
- every fixture is called with the pattern **as a string** (the faithful
  drop-in surface)

Corpus: `crates/braces-core/tests/conformance.json` (byte-identical to
`/mnt/sdb1/braces-rs/state/lane-a/conformance.json`, md5
`4203a86239f0ccdc01d4ca3f7360726f`). The corpus is never edited.

## Result (before fixes)

**1980 pass / 28 fail / 2008 total** — 1 REAL divergence, 27 corpus
representation artifacts.

## REAL divergence (fixed)

| # | fixture | expected | got |
|---|---------|----------|-----|
| 1299 | `expand("{01..20}", {transform:"double"})` | `TypeError: input.padStart is not a function` | same message **plus** suffix ` (transform returned a number for a padded range)` |

Fix: `crates/braces-core/src/error.rs` — `BracesError::PadStartOnNumber`
message is now byte-identical to the reference. (Committed as
`fix(core): exact padStart TypeError message`.)

## Corpus artifacts (27) — generator representation, not implementation bugs

The generator (`gen-conformance.js`) recorded some calls whose inputs
were **not strings**; it stored `String(input)` in the `pattern` field.
No string-arg implementation can reproduce those recordings — the string
`"{a,b},{1..3}"` is a different input than the array `["{a,b}","{1..3}"]`.
Each artifact below was verified against the generator source.

### Non-string inputs to parse/expand/compile (18) — `for (const bad of [null, undefined, 123, true, {}, ['x']])`

- #1966–1968 `pattern:"null"` — recorded `braces.parse/expand/compile(null)`; string call parses/expands the literal text `null`.
- #1969–1971 `pattern:"undefined"` — recorded the same with `undefined`.
- #1972–1974 `pattern:"123"` — recorded with the **number** 123 (V8 `Cannot create property 'queue'` / `compile → ""`).
- #1975–1977 `pattern:"true"` — recorded with the **boolean** true.
- #1978–1980 `pattern:"[object Object]"` — recorded with the **object** `{}` (`String({})` encoding).
- #1981–1983 `pattern:"x"` — recorded with the **array** `['x']` (`String(['x'])` encoding).

Note: the wrapper DOES reproduce the exact V8-worded errors when these
values are passed as actual non-strings (see DIVERGENCES.md §1) — the
artifact is only that the corpus cannot express that call.

### Non-string inputs to create (4)

- #1946–1947 `pattern:"0"` — recorded `braces.create(0)` (number); string call yields `["0"]`.
- #1948–1949 `pattern:"null"` — recorded `braces.create(null)`; string call yields `"null"` / `["null"]`.

### Array inputs to the default fn (4)

- #1962–1963 `pattern:"{a,b},{1..3}"` — recorded `braces(["{a,b}", "{1..3}"])` (two patterns, results concatenated); the joined string is one pattern and expands differently.
- #1964 `pattern:"{a,a},b{a,a}"` — recorded `braces(["{a,a}", "b{a,a}"], {expand:true, nodupes:true})`.
- #1965 `pattern:""` with `result: []` — recorded `braces([])` (empty array); the string call `braces("")` correctly yields `[""]` (and the sibling fixture recording `braces("")` passes).

### Result recorded as a length (1)

- #1988 `expand("{1..5000}", {rangeLimit:false})` — the generator recorded
  `braces.expand(...).length` (the number `5000`), not the array. The
  wrapper returns the full 5000-element array; verified equal in length
  and content by the Rust-core conformance test.

## Result (after fixes)

Re-run after the padStart fix + rebuild: **1981 pass / 27 fail**, every
remaining failure a documented corpus artifact above. The checker exits 0
(no REAL failures).
