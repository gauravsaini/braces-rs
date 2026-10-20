# BENCH.md — braces-rs vs micromatch/braces v3

Final run: 2026-10-20 (box). Raw output: `bench/results.json` in the repo
(copy preserved at `/mnt/sdb1/braces-rs/bench-results-round3.json`).

## Machine

- CPU: Intel Core i7-7700 @ 3.60GHz (8 vCPU)
- RAM: 7.2 GiB
- OS: Ubuntu 25.10 (Linux 6.17.0-41-generic x86_64)
- Node: v20.19.4
- rustc: 1.98.1
- Build: release profile `lto="fat" codegen-units=1 opt-level=3 strip=true`,
  bench-only `RUSTFLAGS="-C target-cpu=native"`,
  `CARGO_TARGET_DIR=/home/muse/.target/braces-rs-native`
- napi binding: @napi-rs/cli 2.18.4, `napi build --platform --release`

## Method

`bench/bench.js /mnt/sdb1/braces-rs/micromatch-braces ../napi`:

1. **Equivalence gate** — every bench input's Rust output is asserted
   `deepStrictEqual` against the JS reference before any timing.
2. 5000-iteration warmup per pattern.
3. Fixed 3 s measurement windows (`process.hrtime.bigint()`), inputs
   pre-allocated outside the loop.
4. 5 repetitions, **median** ops/s reported.

## Results (median ops/s)

| pattern | js ops/s | rs ops/s | speedup |
|---------|---------:|---------:|--------:|
| literal (no braces) | 653,044 | 2,221,028 | **3.40x** |
| small list `{a,b,c}` | 278,241 | 522,894 | **1.88x** |
| padded range `{001..300}` | 44,894 | 73,077 | **1.63x** |
| stepped range `{0..100..10}` | 475,742 | 757,981 | **1.59x** |
| alpha range `{a..z}` | 415,747 | 827,131 | **1.99x** |
| nested `{a,{b..d},e}` | 234,752 | 364,009 | **1.55x** |
| big range `{1..1000}` | 16,547 | 23,160 | **1.40x** |
| compile padded `{001..300}` | 691,848 | 1,092,179 | **1.58x** |
| compile stepped `{1..10..2}` | 677,674 | 1,047,351 | **1.55x** |
| compile nested `a/{b,c}/d` | 865,140 | 1,026,530 | **1.19x** |
| default compile (`braces('{a,b,c}')`) | 750,898 | 963,910 | **1.28x** |
| default expand (`braces('{a,b,c}', {expand:true})`) | 278,152 | 274,459 | 0.99x |
| expandMany batch-100 | 1,444 | 1,697 | **1.18x** |
| multi-group (3000 outputs, extra) | 3,756 | 2,385 | 0.63x |

**Every pattern on the required ladder beats the JS reference**
(literal, small list, padded/stepped/alpha ranges, nested, big range,
both compiles, expandMany batch-100).

### Patterns not beaten (honest notes)

- **default expand (0.99x)** — a statistical tie (within ~1%, re-measured
  with the identical methodology on the final binary: 0.99x). The residual
  is the `{expand: true}` options-object conversion at the napi boundary;
  a no-clone shortcut in `braces_units` did not move it.
- **multi-group (0.63x, extra pattern, not on the required ladder)** —
  `user-{200..300}/project-{a,b,c}-{1..10}` (3000 outputs). The queue
  append rebuilds every output string per segment; V8's rope (cons-string)
  concatenation makes the same algorithm cheaper in JS. A segment-join
  restructure of the walk (build outputs as segment lists, join once) is
  the known fix — future work.

## How the wins were achieved

- UTF-16-native core (exact JS string semantics) with faithful ports of
  parse/expand/compile + fill-range + to-regex-range.
- Fast paths: verbatim literal (no braces/backslash/`]`), bare-range
  `{X..Y[..S]}` for both expand and compile, integer digit rendering
  (no f64 formatting), single-allocation zero-padded ranges.
- FFI discipline: empty options cross as `undefined`; options conversion
  reads only present keys; bulk string results cross **packed** into one
  string (`[count|offsets|data]`) and are unpacked with `substring` —
  per-string N-API creation measured at ~134ns/string, which alone would
  exceed the reference's entire budget on large expansions.
- mimalloc allocator; fat-LTO release profile; `expandMany` batch API.

## Reproduce

```sh
cd /mnt/sdb1/braces-rs/src-repo/bench
node bench.js /mnt/sdb1/braces-rs/micromatch-braces ../napi
# core-only (no napi): cargo bench -p braces-core --bench expand_bench
```
