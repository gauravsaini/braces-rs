# braces-rs — Performance Learnings from the user's Rust projects

Lane B output. Primary source: **ajv-napi** (the user's Rust + napi-rs JSON-schema
validator, a drop-in Ajv replacement that beats the JS original on realistic
workloads). Secondary source: **sense-engine** (the user's Rust on-device visual
engine), via the standing memory record in `~/MEMORY.md`.

Every technique below cites the file/line in ajv-napi it was extracted from.
Resolved dependency versions come from ajv-napi's committed `Cargo.lock`
(Cargo.toml ranges are loose — the lockfile is the truth).

**Stack versions (from ajv-napi `Cargo.lock`):**

| Crate / tool | Cargo.toml asks | Lockfile resolved |
|---|---|---|
| `napi` (features `napi4`, `serde-json`) | `2.12` (`Cargo.toml:10`) | **2.16.17** |
| `napi-derive` | `2.12` (`Cargo.toml:11`) | **2.16.13** |
| `napi-build` (build-dep) | `2.0` (`Cargo.toml:32`) | **2.3.1** |
| `@napi-rs/cli` (devDep) | `^2.18.0` (`package.json:50`) | — |
| `serde_json` | `1.0` (`Cargo.toml:13`) | 1.x |
| `simd-json` | `0.13` (`Cargo.toml:15`) | 0.13.11 |
| `mimalloc-safe` | `0.1` (`Cargo.toml:19,23,27`) | 0.1.56 |

Use these exact napi-rs versions for braces-rs. They are known-good together;
do not jump to napi v3 blind.

---

## The napi boundary

### 1. Per-call FFI cost is fixed — small inputs LOSE unless you amortise

- **Technique:** Treat every JS→Rust call as having a fixed crossing cost.
  Design the API so one crossing does as much work as possible: expose a
  batch entry point (`expandMany(patterns: Vec<String>) -> Vec<Vec<String>>`)
  alongside the single-pattern `expand`, and keep the single-pattern path
  absolutely minimal (typed args, no options-object deep conversion on the
  common path).
- **Why (with numbers):** ajv-napi's own published results show the napi
  version **losing** to pure JS on small payloads — Simple schema (39 bytes):
  ~2.40M ops/s vs Ajv's ~6.11M ops/s (**−61%**); Complex (489 bytes): **−9%** —
  and only winning once the payload reaches ~10KB (**+10–11%**)
  (`README.md`, "Performance" table). Brace patterns are *tiny* (tens of
  bytes) — braces-rs lives exactly in the regime where ajv-napi lost.
  The win for braces must come from (a) a much cheaper per-call path than
  ajv-napi's serde conversion, and (b) batch amortisation.
- **Applies in braces-rs:** napi boundary design (function signatures),
  benchmark design (must include the realistic single-small-pattern case,
  not just bulk cases).
- **Avoid:** Claiming victory on bulk benchmarks only. ajv-napi's README
  publishes its losses next to its wins — copy that honesty standard.

### 2. Byte inputs cross the boundary at ~zero cost; JS values do not

- **Technique:** Where input can arrive as bytes, take a napi `Buffer`
  (derefs to `&[u8]`, no copy) and parse in Rust — ajv-napi's
  `validate_buffer(env, buffer: Buffer)` at `src/lib.rs:305` feeds the slice
  straight into `simd_json::serde::from_slice` (`src/lib.rs:311`).
  For braces-rs the primary input is a JS string, so the analogue is:
  accept `String` (napi-rs converts via `JsString` UTF-8) and immediately
  work on `pattern.as_bytes()` — never re-encode, never go through
  `serde_json::Value`.
- **Why:** ajv-napi has two input paths and the measured difference between
  them *is* the product: the `serde_json::Value` path (`validate`,
  `src/lib.rs:300`) pays a full deep JS→serde conversion per call, while the
  Buffer path skips V8 object materialisation entirely (architecture
  diagram, `README.md:18-46`).
- **Applies in braces-rs:** all napi entry points — pattern in, options in.
- **Avoid:** Taking `serde_json::Value` for the options object. braces v3
  options are a handful of scalars (booleans / small ints). Use typed
  parameters (`Option<bool>`, `Option<u32>`, `Option<String>`) or a
  `#[napi(object)]` struct so napi-rs maps fields directly. Reserve
  `serde_json::Value` (enabled by the `serde-json` feature, `Cargo.toml:10`)
  for genuinely free-form data — braces has none.

### 3. Result marshalling: one flat typed return, no DOM, no empty-work allocs

- **Technique:** Return plain typed collections and let napi-rs convert them
  in one pass: ajv-napi returns `Option<Vec<AjvError>>` where `AjvError` is a
  `#[napi(object)]` struct (`src/lib.rs:290-297`, return at `src/lib.rs:300`).
  The `Option` matters: the happy path returns `None` (→ JS `null`) instead
  of allocating/marshalling an empty array (`validate_impl`,
  `src/lib.rs:344-364`).
- **Why:** Every returned element costs one V8 object/string creation; that
  cost is unavoidable per result string, but everything beyond it (nested
  vectors, serde DOM, wrapper objects) is pure overhead on top of a result
  set that can be thousands of expansions.
- **Applies in braces-rs:** `expand` returns `Vec<String>` directly — the
  direct analogue of `Vec<AjvError>`. Do not wrap results in a struct, do
  not return `serde_json::Value`, do not return `Vec<Vec<String>>` from the
  single-pattern API.
- **Avoid:** Building a JS-side post-processing step that re-walks the
  result array (see packaging, #13 — the wrapper must be a thin reshaping
  layer, not a second implementation).

### 4. Split the cheap check from the expensive detail path

- **Technique:** Two-tier API: cheap boolean/validity check first, full
  detail only on demand. In ajv-napi, `validate_impl` calls
  `validator.is_valid(data)` first and only runs the expensive
  `iter_errors()` collection when validation fails — the code says why
  (`src/lib.rs:346-348`: "Fast path: is_valid() is cheaper than iter_errors()
  for valid data (no path tracking or error collection overhead)"). The
  boolean-only `isValidBuffer` is exposed as its own entry point
  (`src/lib.rs:319`).
- **Why (with numbers):** `README.md:191` — the boolean fast path performs
  "up to ~50% faster on small payloads" than the full path.
- **Applies in braces-rs:** Inside the core expander — (a) a no-braces fast
  path: if a memchr scan finds no unescaped `{`, return the input as the
  single result without building any parser state; (b) a counting/validation
  pre-pass only where it is cheaper than the work it saves; (c) expose
  cheap predicates (e.g. "does this pattern contain braces / is it a range")
  as separate napi functions rather than forcing a full expansion.
- **Avoid:** One monolithic code path that pays full machinery cost for the
  trivial majority of patterns (most real-world patterns have 0–1 brace
  groups).

### 5. Errors: plain strings, formatted lazily, off the hot path

- **Technique:** Map internal errors with `Error::from_reason(...)` at the
  boundary only (`src/lib.rs:312,326` — `format!("Invalid JSON: {}", e)`;
  compile path `src/lib.rs:~276` — `.map_err(|e| Error::from_reason(e.to_string()))`).
  Formatting cost is paid only when an error actually occurs.
- **Applies in braces-rs:** napi boundary error mapping.
- **Avoid:** Two things. (a) Pre-formatting error strings speculatively in
  the core. (b) Throwing where the JS reference doesn't: micromatch/braces
  treats most malformed patterns (unbalanced braces etc.) as literal text
  rather than errors — a drop-in must reproduce that, so the Rust core
  should model "not a brace expression" as a normal outcome, not an `Err`.

### 6. NEVER call back into JS from the hot path

- **Technique:** Keep the core 100% callback-free. ajv-napi supports JS
  custom keywords/formats only as an explicit opt-in, because "calling from
  Rust into V8 has a performance cost compared to native validation"
  (`README.md:123`). It even skips the callback plumbing at runtime when no
  callbacks are registered: `has_custom_callbacks` is computed once at
  compile time (`src/lib.rs:278`) and `with_env` only installs the env when
  it's true (`src/lib.rs:332-342`).
- **Applies in braces-rs:** The whole core. braces v3 options are pure data
  — there is no legitimate reason for a JS callback inside expansion. If a
  future option ever needs one, gate it exactly like `has_custom_callbacks`:
  detect once, branch once, zero cost when unused.
- **Avoid (anti-pattern, do not copy):** ajv-napi's callback bridge itself —
  a thread-local raw `napi_env` smuggled into validation
  (`CURRENT_ENV`, `src/lib.rs:20-42`) plus `unsafe impl Send/Sync` on a
  wrapper around `Ref<()>` (`JsCallback`, `src/lib.rs:36-42`). It works but
  is the most fragile code in the repo. braces-rs does not need it; do not
  import the pattern.

---

## Hot-loop string building (the core expand engine)

### 7. Swap the global allocator to mimalloc

- **Technique:** Set mimalloc as the global allocator, with ajv-napi's
  exact per-target cfg matrix:

  ```rust
  #[cfg(not(any(target_arch = "arm", target_os = "freebsd", target_family = "wasm")))]
  #[global_allocator]
  static ALLOC: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;
  ```
  (`src/lib.rs:14-17`), with `mimalloc-safe` features per target:
  `skip_collect_on_exit` everywhere; `+ local_dynamic_tls` on Linux x64 and
  ARM64; `+ no_opt_arch` on Linux ARM64 (`Cargo.toml:18-27`).
- **Why:** Brace expansion is an allocation storm — one input pattern
  becomes tens to thousands of small `String`s. Allocator behaviour
  dominates this workload class; ajv-napi's README lists mimalloc as one of
  its four headline perf mechanisms (`README.md`, "Why Rust + NAPI?").
- **Applies in braces-rs:** crate root of the napi crate (and the core
  benchmark binary).
- **Avoid:** Forgetting the cfg exclusions (wasm / 32-bit arm / FreeBSD) —
  copying the `static ALLOC` line without its cfg gate breaks those builds.

### 8. Reuse a thread-local scratch buffer; clear, don't drop

- **Technique:** Keep a `thread_local!` scratch `Vec` that is cleared and
  refilled per call instead of allocating per call:

  ```rust
  thread_local! {
      static PARSE_BUF: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(256 * 1024));
  }
  // per call: buf.clear(); buf.extend_from_slice(&buffer);
  ```
  (`src/lib.rs:20-22`, used at `src/lib.rs:307-313` and `321-327`).
- **Why:** Node calls a napi module from (potentially) several threads;
  a thread-local avoids both locks and repeated allocation. Capacity is
  retained across calls, so steady-state calls allocate ~nothing for
  scratch.
- **Applies in braces-rs:** Core expander — a thread-local output arena /
  segment stack reused across `expand` calls: `buf.clear()` keeps capacity.
  Size the initial capacity to the workload (patterns are small: 4–8KB is
  plenty; do not copy the 256KB figure, which was sized for JSON payloads).
- **Avoid:** `static mut` globals or a `Mutex<Vec>` — both are worse than
  thread-local here (UB risk / lock contention). Also avoid returning data
  that borrows the scratch buffer; copy out into the result `Vec<String>`
  (the result strings are the deliverable, scratch is not).

### 9. Scan bytes in a single pass; no regex, no char decoding, no AST

- **Technique:** Work on `&[u8]` with byte comparisons and `memchr`-style
  search. Brace syntax is pure ASCII (`{`, `}`, `,`, `.`, digits, ASCII
  letters, `\` escape) — nothing in it requires UTF-8 char decoding.
  Parse and expand in one pass with an explicit stack, writing directly
  into the output strings.
- **Why:** This is the direct analogue of ajv-napi's core bet: replace an
  interpreted/allocating JS pipeline with a single compiled pass over raw
  bytes (simd-json parses a `&mut [u8]` in place, `src/lib.rs:311`; the
  README's architecture claim is "bypassing V8's garbage collector and
  JSON.parse overhead", `README.md:18`). The JS braces implementation
  tokenises into an AST and then walks it — two passes plus per-node
  allocation. A single-pass byte scanner is where the Rust speedup
  actually comes from; the napi wrapper is not.
- **Applies in braces-rs:** The entire core crate.
- **Avoid:** (a) The `regex` crate anywhere near expansion — sequence/range
  detection (`1..10`, `a..z`, with optional step `..2`) is a hand-rolled
  integer/char parse, faster and allocation-free. (b) `chars()` /
  `char_indices()` iteration. (c) Building a token tree first "for
  cleanliness" — that reintroduces exactly the two-pass cost being removed.

### 10. Pre-size everything from a cheap counting pass

- **Technique:** Before expanding, compute the exact result count (product
  of the alternatives in each brace group — groups are independent) and
  `Vec::with_capacity(count)`; give each output `String` a capacity of
  ~`pattern.len()` (outputs differ from the input only by the removed
  brace syntax) so no output string ever reallocates while growing.
  Hold intermediate segments as borrowed slices: `SmallVec<[&str; 8]>`
  (or a plain stack array) of `&str` pieces pointing into the original
  pattern — copy bytes exactly once, at final string assembly.
- **Why:** Reallocation-and-copy is the silent tax in naive expanders:
  a `String` grown by `push_str` from empty doubles repeatedly, and a
  `Vec` of results grown by `push` does the same. ajv-napi applies the
  same principle at its scale — inputs pre-sized before the timed/measured
  work (`benchmark.js:14-15`, "We pre-allocate all buffers before
  benchmarking to isolate validation throughput"), scratch pre-sized at
  256KB (`src/lib.rs:21`). braces-rs should apply it inside the algorithm,
  not just the bench.
- **Applies in braces-rs:** Core expander (result vec + per-string
  capacity + segment storage).
- **Avoid:** `format!()` / `+` string concatenation in the loop (each is a
  fresh allocation), collecting intermediate `Vec<String>` per brace group
  and then cartesian-joining them with more allocations — write the
  cartesian product directly into the final pre-sized vec.

---

## Release profile and build config

### 11. Copy ajv-napi's profile as the floor — then complete it

ajv-napi's entire release profile is (`Cargo.toml:33-35`):

```toml
[profile.release]
lto = true
```

plus per-arch `target-cpu=native` rustflags in `.cargo/config.toml:1-5`.
That is the floor, not the finished article. For braces-rs use:

```toml
[profile.release]
lto = "fat"
codegen-units = 1
opt-level = 3
strip = true
```

- **Why each line:** `lto = "fat"` + `codegen-units = 1` maximise
  cross-crate inlining for a tiny hot core (the expand loop must inline
  into the napi entry points); `opt-level = 3` is the explicit default
  worth pinning; `strip = true` shrinks the `.node` binary (faster load,
  smaller npm tarballs — ajv-napi ships per-platform binaries, see #13).
- **panic:** ajv-napi sets nothing — it keeps the default `panic = "unwind"`.
  Keep it that way for a napi module: napi-rs converts unwinding panics
  into JS exceptions, while `panic = "abort"` would take down the whole
  Node process on any core bug. Do not add `panic = "abort"` to the
  cdylib. (If the pure core is also built as a standalone benchmark binary,
  `panic = "abort"` may be set for that profile target only.)
- **Applies in braces-rs:** `Cargo.toml` of the workspace / napi crate.
- **Avoid:** Shipping `target-cpu=native` in published binaries — see #12.

### 12. `target-cpu=native` is for local benchmarks, NOT for shipped binaries

- **Technique:** ajv-napi sets `rustflags = ["-C", "target-cpu=native"]`
  for both x86_64 and aarch64 in `.cargo/config.toml:1-5`. That is correct
  for a locally-built benchmark (it lets the byte scanner use the host's
  best instructions) and it is what braces-rs should use when producing
  the headline benchmark numbers **on the dev box**.
- **Why the caution:** A binary built with `target-cpu=native` on one
  machine can execute illegal instructions (SIGILL) on a machine with an
  older CPU. napi modules are distributed as prebuilt per-triple binaries
  (see #13) — the build machine's CPU is not the user's CPU.
- **Applies in braces-rs:** `.cargo/config.toml` (local), CI matrix
  (release builds).
- **Avoid:** Letting the local config leak into CI release builds. For
  shipped triples use a safe baseline (e.g. `target-cpu=x86-64-v2`) or
  runtime feature detection in the hot scanner; keep `native` behind a
  local-only config / env flag.

### 13. Toolchain pinning and vendoring (sense-engine lessons)

- **Technique:** Pin the toolchain (`rust-toolchain.toml`) and keep a
  `cargo vendor` escape hatch.
- **Why:** From the sense-engine record (`~/MEMORY.md`, sense-engine
  section): the sandbox ran Rust **1.99.0 + wasm32 target +
  wasm-bindgen-cli 0.2.129** while the box runs **rustc 1.98.1** —
  cross-machine version skew is real in this user's setup, and an unpinned
  build silently changes perf characteristics. The same project hit a
  crates.io DNS block on the Mac and fixed it with `cargo vendor`
  (**309MB, gitignored**) — vendor once on a machine with network, build
  offline everywhere else.
- **Applies in braces-rs:** Repo root (toolchain file), `.gitignore`
  (vendor dir), build docs.
- **Avoid:** Assuming the build machine always has crates.io access —
  the user's machines have demonstrably lost it before.

---

## Benchmarking discipline

### 14. Correctness gate BEFORE timing — non-negotiable

- **Technique:** ajv-napi's benchmark refuses to time anything until both
  implementations agree: `assertBufEquiv(...)` runs all suites' inputs
  through both the JS and napi validators and throws on any mismatch
  (`benchmark.js:194-207`), printing "Correctness verified" before the
  first timed run.
- **Applies in braces-rs:** The bench harness must first run the full
  conformance corpus (micromatch braces' own test fixtures) through both
  `braces` (JS) and `braces-rs` with deep-equality on the result arrays —
  byte-for-byte, order included — and abort if any case differs. Speed on
  wrong output is worthless; this gate is what makes the headline number
  believable.
- **Avoid:** Spot-checking two or three patterns by eye. Brace edge cases
  (escapes, nested groups, negative steps, zero-padding, empty
  alternatives) are exactly where implementations diverge.

### 15. Fixed-duration runs, JIT warmup, inputs pre-allocated outside the loop

- **Technique (what benchmark.js does right):**
  - Warmup of 5000 iterations before timing, "to trigger V8 JIT
    optimisation" (`benchmark.js:25-26`) — the JS baseline gets its best
    shot; beating a cold interpreter proves nothing.
  - Fixed-duration measurement (3000ms per case) with `process.hrtime.bigint()`
    nanosecond timing, reported as ops/sec (`benchmark.js:24-37`).
  - All inputs pre-generated/pre-allocated before any timing
    (`benchmark.js:14-15,158-165`) so allocation noise stays out of the
    measured operation.
  - A **size ladder** of realistic cases (39B → ~260KB in ajv-napi;
    for braces: single simple pattern → nested multi-group patterns →
    large expansions → batch-of-patterns), which is what exposed the
    FFI crossover point in #1.
- **Applies in braces-rs:** `benchmark.js` equivalent + the README results
  table.
- **Avoid (what benchmark.js does wrong — do better):**
  - **Asymmetric comparison:** the JS baseline is charged
    `JSON.parse(buf.toString())` while napi gets the raw buffer
    (`benchmark.js` suites). That asymmetry is defensible for ajv-napi's
    I/O story; for braces-rs it would be cheating. The honest comparison
    is same-input/same-output: JS `braces.expand(pattern, opts)` vs napi
    `expand(pattern, opts)`, string in, string-array out, both sides.
  - **No statistics:** a single 3s run, no repetitions, no variance or
    median. braces-rs: ≥5 repetitions per case, report median (and min),
    same machine, same Node version, release builds for both sides.
  - **Mixed clocks:** the loop end-condition uses `Date.now()` while
    elapsed time uses `hrtime` (`benchmark.js:30-33`). Use one clock
    (`hrtime.bigint`) for both.
  - **No Rust-side micro-bench:** ajv-napi has none. braces-rs should pair
    the JS harness with **criterion** benches of the pure core (no Node
    involved) so core regressions are visible independently of FFI noise.
    This is only possible if the core is napi-free — see #16.

---

## napi-rs version-specific gotchas

### 16. Structure: generated loader + hand-written wrapper + napi-free core

- **build.rs is three lines and mandatory** (`build.rs:1-5`):
  ```rust
  extern crate napi_build;
  fn main() { napi_build::setup(); }
  ```
  with `napi-build = "2.0"` as a build-dependency (`Cargo.toml:31-32`).
  Skipping it breaks linking (notably on Windows).
- **`index.js` and `index.d.ts` are AUTO-GENERATED by the napi CLI**
  (`index.js:5` and `index.d.ts:4`: "auto-generated by NAPI-RS"). Never
  hand-edit them; regenerate with `napi build --platform --release`
  (`package.json:42`). The build also produces the per-platform
  `<name>.<triple>.node` binary the loader looks for.
- **CamelCase needs explicit help:** `#[napi(js_name = "validateBuffer")]`
  on snake_case Rust fns (`src/lib.rs:304,318`).
- **Derive-generated TS types for function-typed args are wrong** — ajv-napi
  overrides them manually with `#[napi(ts_args_type = "...")]`
  (`src/lib.rs:186,195`). Expect the same for any non-trivial braces-rs
  signature that reaches the `.d.ts`.
- **`napi4` feature, not napi8** (`Cargo.toml:10`): widest Node ABI
  compatibility. Note the inconsistency in ajv-napi: it compiles for napi4
  but declares `engines: node >= 22` (`package.json:38-40`).
- **The loader falls back to per-platform npm packages:** generated
  `index.js` first tries a local `.node` file, else requires
  `@gauravsaini/ajv-napi-<triple>` (e.g. `index.js`, linux/x64 branch).
  That fallback is what the `napi` config's `package.name` + `triples`
  in `package.json:52-66` and the `napi artifacts` / `napi prepublish`
  scripts (`package.json:44-45`) exist to feed.

### 17. Keep the core crate free of napi — test and bench it natively

- **Technique:** ajv-napi's core logic is trapped inside the napi crate
  (single `src/lib.rs`, 365 lines, validator + bindings together) — which
  means its only tests are JS-side (`test.js`, `test-features.js`,
  `test-custom.js`, `test-esm.mjs`; `package.json:47`) and it has no Rust
  unit tests or criterion benches at all.
- **Applies in braces-rs:** Do the opposite: a pure `braces-core` library
  crate (no napi dependency) + a thin napi crate that only marshals.
  This is also the sense-engine pattern: the engine core was proven with
  `cargo test` (9/9 green) and a node smoke test *before* any host
  integration, and the same core targeted wasm32 (173KB build) — a
  platform-free core is what made both possible (`~/MEMORY.md`,
  sense-engine M1 record).
- **Avoid:** ajv-napi's monolith layout. If expansion logic lives behind
  `#[napi]` functions, every test run needs Node and every bench measures
  FFI + core together.

### 18. Heavyweight dependencies are a liability — keep the core dependency-light

- **Technique:** Audit every dependency for load-time and size cost.
- **Why:** sense-engine's M2 record is the cautionary tale: a real
  EmbeddingGemma-2 ONNX integration meant a **~2.4s model load** and
  hundreds of MB of weights (`~/MEMORY.md`, sense-engine section) —
  fine for an ML engine, fatal for a string utility that must feel
  instant. ajv-napi's own dep list is lean by comparison (one validator
  crate + simd-json + serde).
- **Applies in braces-rs:** The core needs **zero** runtime dependencies
  (hand-rolled scanner; `smallvec`/`memchr` are the only justifiable
  exceptions, both tiny). No regex crate, no parser framework.
- **Avoid:** Pulling in a glob/matching crate "for the compile-to-regex
  part" without benchmarking it — braces' `compile` output is a regex
  *string*; generating it is more string building, same hot-loop rules
  as #9/#10.

---

## Packaging the drop-in

### 19. Two JS layers: generated loader below, API-shaped wrapper above

- **Technique (ajv-napi's exact shape):**
  - `package.json:5` — `"main": "wrapper.js"`, with an `exports` map
    routing `import` → `wrapper.mjs` and `require` → `wrapper.js`
    (`package.json:6-11`), `types: index.d.ts`, and a `files` whitelist
    (`package.json:14-19`).
  - `index.js` — generated platform loader (see #16). Not the public API.
  - `wrapper.js` — a hand-written JS class that **recreates the original
    library's API surface** over the raw napi classes: constructor with
    options object, chainable methods (`return this`), property-style
    state (`validate.errors`), and reshaping of native results into the
    original library's exact object format (`formatErrors`,
    `wrapper.js:154-211`). Entry: `module.exports = Ajv;
    module.exports.default = Ajv` (`wrapper.js:213-214`).
  - `wrapper.mjs` — a 6-line ESM shim: `createRequire` + re-export
    (`wrapper.mjs:1-6`).
- **Applies in braces-rs:** Identical skeleton. The public surface to
  reproduce is micromatch/braces v3's (a callable `module.exports`
  function with attached methods — the sibling lane's API study is the
  authority on the exact surface); the napi layer underneath exposes raw
  functions, and the wrapper reproduces callability, method attachments,
  default-export interop, and options semantics. `require('braces-rs')`
  must behave like `require('braces')` with no napi types leaking.
- **Avoid:** Exposing the napi classes directly as the public API (the
  ajv-napi equivalent would have leaked `NapiValidator` naming and
  Rust-shaped errors). Also avoid ajv-napi wrapper's silent-failure habit:
  `addSchema` swallows all errors (`wrapper.js:86` — `catch (e) {}`) and
  `addFormat`/`addKeyword` catch-and-ignore "to maintain drop-in
  compatibility" (`wrapper.js:111-114,132-135`). It buys compatibility
  by hiding bugs; for braces-rs, match the reference's *observable*
  behaviour deliberately per-method instead of blanket-swallowing.

### 20. Per-platform binaries via the napi CLI flow — and don't skip Linux

- **Technique:** `package.json:52-66` holds the `napi` config: `name`,
  scoped `package.name` (`@gauravsaini/ajv-napi`), `triples.defaults: true`
  plus an `additional` list (aarch64 darwin/linux-gnu/linux-musl/windows).
  CI builds each triple with `yarn build --target <triple>`, uploads
  `<name>.*.node` artifacts, then publishes via `napi create-npm-dir` +
  `napi artifacts` + `napi prepublish -t npm`
  (`.github/workflows/CI.yml`, publish job; scripts at `package.json:43-45`).
- **Applies in braces-rs:** Copy the flow wholesale.
- **Avoid (ajv-napi's known gap):** Its CI has **all Linux builds commented
  out** ("Linux builds disabled due to CI runner availability issues",
  `.github/workflows/CI.yml`) and the README concedes "Linux binaries are
  temporarily unavailable". braces-rs must not repeat this: linux-x64-gnu
  is the primary triple (the dev box is Linux) and belongs in the matrix
  from day one. Also set `engines` as wide as the reference library
  supports — ajv-napi's `node >= 22` (`package.json:38-40`) is far
  narrower than what a micromatch-ecosystem drop-in can assume.

---

## Quick-reference: the copy-paste blocks

**Cargo.toml deps (versions proven together in ajv-napi):**

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
napi = { version = "2.12", features = ["napi4", "serde-json"] }
napi-derive = "2.12"

[build-dependencies]
napi-build = "2.0"

[profile.release]
lto = "fat"
codegen-units = 1
opt-level = 3
strip = true
```
(lockfile will resolve napi to 2.16.x — commit the lockfile.)

**build.rs (verbatim from ajv-napi `build.rs:1-5`):**

```rust
extern crate napi_build;

fn main() {
    napi_build::setup();
}
```

**Allocator (from ajv-napi `src/lib.rs:14-17` + `Cargo.toml:18-27`):** see #7.

**Package scripts (from ajv-napi `package.json:42-45`):**
`build: napi build --platform --release` · `artifacts: napi artifacts` ·
`prepublishOnly: napi prepublish -t npm` · devDep `@napi-rs/cli@^2.18.0`.

---

*Sources: all citations are to files in `ajv-napi/` as extracted from the
box (`/mnt/sdb1/braces-rs/ajv-napi`) on 2026-10-20, except sense-engine
items, which cite the sense-engine section of `~/MEMORY.md` (the project's
code lives on the Mac and was out of scope for this lane).*
