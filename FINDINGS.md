# LANE A — braces v3.0.3 API study + conformance corpus

Source: micromatch/braces v3.0.3 (package.json), deps: fill-range ^7.1.1 → to-regex-range ^5.0.1.
Corpus: `conformance.json` (2008 fixtures), generator: `gen-conformance.js`.
Each record: {fn, pattern, options, result} or {fn, pattern, options, error:"Name: message"}.
`transform` options recorded as string tags ("double","alphaFoo","prefixN").

## 1. API surface

| Export                | Signature                                  | Returns                                                                                                                                                                                |
| --------------------- | ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `braces` (default fn) | `(patterns: string \| string[], options?)` | **array of strings** — `create()` per pattern, flattened; if `expand&&nodupes` dedupes via Set                                                                                         |
| `braces.expand`       | `(input: string \| AST, options?)`         | **array**; `noempty` → filter(Boolean); `nodupes` → Set dedupe                                                                                                                         |
| `braces.compile`      | `(input: string \| AST, options?)`         | **string** (NOT array — JSDoc saying "array" is wrong): regex-source                                                                                                                   |
| `braces.create`       | `(input: string, options?)`                | if `input==='' \|\| input.length<3` → `[input]`; else `expand` if `options.expand===true` else compile-string                                                                          |
| `braces.stringify`    | `(input: string \| AST-node, options?)`    | raw string reconstruction (used for invalid/dollar braces)                                                                                                                             |
| `braces.parse`        | `(input: string, options?)`                | AST `{type:'root', input, nodes[]}`; throws TypeError('Expected a string') on non-string; SyntaxError(`Input length (N), exceeds max characters (M)`) when len > min(maxLength, 10000) |

### Options (defaults shown; `undefined` = off)

- `maxLength` (10000, capped by MAX_LENGTH=10000) — parse only
- `expand` (bool) — routes main/create to expand instead of compile
- `nodupes` — dedupe expand results; also applies in main fn when expand:true
- `noempty` — drop empty strings from expand results
- `rangeLimit` (1000; `false` disables) — expand only; throws RangeError('expanded array length exceeds range limit...') when `(max-min)/step >= limit`. NB: `{1..1000}` is OK (999<1000); `{1..1001}` throws
- `transform(fn)` — per-item mapping in range fill: numeric values get numbers, alpha ranges get **char codes**
- `keepEscaping` (bool) — parse keeps the backslash on escaped chars (`a\{b,c}d` → `a\{b,c}d` instead of `a{b,c}d`)
- `keepQuotes` (bool) — parse keeps quote chars around brace groups
- `escapeInvalid` (bool) — compile/stringify prefixes invalid-brace `{`/`}` with `\`
- `step` (number) — fill-range fallback step when pattern has no explicit step: `{1..9},{step:3}` → 1,4,7
- `quantifiers` — **DEAD OPTION**: documented in README, zero code references; runtime-verified: identical output with/without
- `capture`, `wrap`, `shorthand`, `strictZeros`, `relaxZeros` — passed through to to-regex-range on the compile path. compile() forces `{wrap:false, toRegex:true, strictZeros:true}` then spreads user options (so user can override). `strictZeros:true` also selects strict zero-padding mode in to-regex-range

## 2. Tricky semantics (each verified at runtime)

1. **`]` is always escaped**: parse converts every literal `]` to `\]`. `expand('a]b')` → `['a\\]b']`; `compile(']')` → `'\\]'`. Rust port must do this too.
2. **Dollar braces are literal, always**: `expand('${a,b}')` → `['${a,b}']`; `compile('${1..3}')` → `'${1..3}'`. (bash parameter-expansion guard; `dollar` flag on brace node)
3. **Single-item braces are literal**: `expand('{a}')` → `['{a}']`; `expand('{}')` → `['{}']`; `expand('{,}')` → `['','']` (two empties!); `compile('{,}')` → `'(|)'`.
4. **Unbalanced braces pass through literally**: `expand('a{b}}c')` → `['a{b}}c']`; `expand('{a,b')` → `['{a,b']`; stray `}` alone is plain text. `compile('{a,b')` → `'{a,b'` (with `escapeInvalid` → `'\\{a,b'`).
5. **Multi-char / invalid ranges fall back to literal**: `expand('{aa..bb}')` → `['{aa..bb}']`; `expand('{1..2..3..4}')` → literal; `expand('{a..e..x}')` → literal (fillLetters invalidStep → [] → stringify(node) fallback); `expand('{1..5..x}')` → literal. **No error thrown** — silently literal.
6. **Numeric range semantics (fill-range)**: step is `Math.max(Math.abs(step),1)` — `{0..100..0}` expands 0..100 step 1; `{1..5..-1}` → 1..5 (sign ignored); `{10..1..2}` → 10,8,6,4,2. Descending auto-detected from endpoints. `{+1..5}` → 1..5 ('+' dropped). `{-0..5}` → `-0..5`. Padding: any endpoint or step with leading zeros → pad ALL to max length, and the sign char counts: `{-05..05}` → `-05..-01,000..005`.
7. **Alpha ranges use UTF-16 charCodeAt**: `{a..z}` works; `{a..9}` descends through ALL punctuation (97→57); `'{α..ω}'` → 25 items (single UTF-16 units OK); endpoints >1 UTF-16 unit (`'😀'`) → invalid → literal passthrough. Multi-char latin endpoints invalid: `{ab..cd}` → literal.
8. **Commas + ranges mixing**: `{a,1..3,b}` is a LIST (commas win): `['a','1','2','3','b']` — a range never coexists with commas in one brace; when a comma appears after a `..` started, parser stringifies the partial range as text.
9. **Quotes eat commas**: `{"a,b"}` → `['{a,b}']`? No — `expand('{"a,b"}')` → `['{a,b}']` wait, corpus: quoted content kept as single text, quotes stripped → expand('{`foo,bar`}') → ['foo,bar']; with keepQuotes:true → ['{`foo,bar`}']. Unpaired quote → consumed as regular char.
10. **Brackets hide braces**: `a/[{b,c}]/d` → whole span one text token (braces inside `[]` never expand); but `a/{a,b,[{c,d}]}/e` keeps `[{c,d}]` as a text element. Escapes inside `[]` are respected for scanning.
11. **Parens** form `paren` nodes that are transparent: `*(a|{b|c,d})` → `['*(a|b|c)','*(a|d)']`. Unmatched paren → literal text.
12. **NBSP and ZWNBSP are stripped** by the parser (both inside and outside braces): `expand('{a b,c}')` (NBSP) → `['ab','c']`.
13. **Compile range wrapping**: numeric range alone `{1..10..2}` → `(1|3|5|7|9)` (parens added by compile since range-length>1); `{a..z}` → `([a-z])`; `{a..b}` → `([a-b])`; single-value `{5..5}` → bare `5`. Negative ranges: `{-10..10}` → `(-[1-9]|-?10|[0-9])` (boundary value gets `-?` from collatePatterns), `{-10..-1}` → `(-[1-9]|-10)`. Padded ranges: `{001..300}` → `(00[1-9]|0[1-9][0-9]|[12][0-9]{2}|300)`; `{01..03}` → `(0[1-3])`. Huge ranges work: `{1..9007199254740991}` → compact alternation (see corpus).
14. **Step>1 compile numeric uses toSequence**: `{2..10..2}` → `(2|4|6|8|10)`; negatives padded to maxLen via toMaxLen.
15. **`transform` receives (value,index)**; numeric → number, alpha → char code. Applied pre-padding.
16. **expand('') → ['']** (create short-circuit); `braces('')` → `['']`.
17. **`compile` has a stray `console.log('node.isClose', ...)`** at lib/compile.js:17 on the `node.isClose` branch. I could not trigger it with a battery of unbalanced patterns (`a{b}}c`, `{a}}`, `{a,{b}c`, `a}b{c,d}`, `{a,b`, `a{b}}c`) — appears near-dead; do NOT replicate the side effect in Rust. Noted for API parity: outputs unaffected.
18. **Errors**: non-string to parse → TypeError('Expected a string'); to expand directly → TypeErrors like "Cannot create property 'queue' on number '123'" (expand skips parse for non-strings — quirk worth replicating or documenting); oversize → SyntaxError('Input length (N), exceeds max characters (M)') with exact wording; rangeLimit → RangeError('expanded array length exceeds range limit. Use options.rangeLimit to increase or disable the limit.') with exact wording.

## 3. Perf-critical hot paths (for the Rust builder)

1. **parse()** — single pass over input, char-class dispatch; every call starts here. Must be a tight byte scanner; avoid per-char allocations (merge consecutive text — JS does this; note AST node-per-char is NOT what it does).
2. **fill-range numeric fill (expand)** — the `while (a<=b)` loop with pad+transform; `padStart` per item is the inner cost.
3. **to-regex-range (compile)** — `splitToRanges` digit-group splitting + string assembly; has an internal cache (toRegexRange.cache) — Rust port should add one too.
4. **expand cartesian product** (`append` in expand.js) — recursion-heavy; benchmark drivers use multi-group patterns like `{a,b}{c,d}{e,f}`.
5. **Benchmark targets** (bench/index.js uses `{1..100}` style ranges + nested sets): beat `braces.expand` on `{1..100}` (~291B), nested-range expansion, and `braces()` compile of `{1..1000000}` (Rust should be ~µs vs JS ms).

## 4. Open questions

- `expand()` on non-string input throws V8-specific TypeErrors ("Cannot create property 'queue' on number '123'") — does the Rust port replicate exact error strings or map to napi errors? Recommend: document as divergence, throw napi Error with TypeError semantics.
- `compile()` JSDoc says it returns Array — reality is string. napi binding should return string; note the doc bug.
- `quantifiers` dead in v3.0.3 (works in micromatch v4's braces 3.0.3? no — same lib). Do not implement unless requested.
- to-regex-range's huge-range algorithm uses Number math with abs() — for ranges > Number.MAX_SAFE_INTEGER the JS output may be subtly wrong; corpus records actual bytes so Rust port can match the recorded behaviour.
- `step` option only applies when the pattern range lacks an explicit step — and interacts with `options.step` inside `exceedsLimit(...args, options.step, rangeLimit)`; a pattern step overrides. Edge: explicit step + rangeLimit uses the pattern step (verified in code).
- Zero-width `\uFEFF`/`\u00A0` stripping: applies to ALL text including inside quoted strings? Probes: inside `[]` yes (whole span). Inside `"..."` quotes — parser handles quotes before the strip check? Strip check is before quote handling in the loop, so quotes containing NBSP... `expand('{"a b"}')` → corpus? Not included; minor edge, recorded in generator as out-of-scope.
