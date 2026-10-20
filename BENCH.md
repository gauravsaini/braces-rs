# BENCH.md — braces-rs vs micromatch/braces v3

Final run: 2026-10-20 (box). Raw output: `bench/results.json` in the repo
(copy preserved at `/mnt/sdb1/braces-rs/bench-results-final.json`).

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
| literal (no braces) | 642,937 | 2,292,774 | **3.57x** |
| small list `{a,b,c}` | 255,356 | 496,559 | **1.94x** |
| padded range `{001..300}` | 44,879 | 73,713 | **1.64x** |
| stepped range `{0..100..10}` | 472,424 | 749,035 | **1.59x** |
| alpha range `{a..z}` | 428,622 | 832,353 | **1.94x** |
| nested `{a,{b..d},e}` | 227,424 | 365,310 | **1.61x** |
| big range `{1..1000}` | 16,615 | 23,429 | **1.41x** |
| compile padded `{001..300}` | 673,952 | 1,082,637 | **1.61x** |
| compile stepped `{1..10..2}` | 675,425 | 1,070,797 | **1.59x** |
| compile nested `a/{b,c}/d` | 736,928 | 1,046,690 | **1.42x** |
| default compile (`braces('{a,b,c}')`) | 627,237 | 920,708 | **1.47x** |
| default expand (`braces('{a,b,c}', {expand:true})`) | 251,496 | 266,517 | 1.06x |
| expandMany batch-100 | 1,374 | 1,674 | **1.22x** |
| multi-group (3000 outputs, extra) | 3,576 | 2,352 | 0.66x |

**Every pattern on the required ladder beats the JS reference**
(literal, small list, padded/stepped/alpha ranges, nested, big range,
both compiles, expandMany batch-100).

### Patterns not beaten (honest notes)

- **default expand (1.06x, borderline)** — across repeated full runs this
  case lands at 0.99x–1.06x, i.e. statistical parity with the reference.
  The residual is the `{expand: true}` options-object conversion at the
  napi boundary. Reported as-is; it is the closest case on the board.
- **multi-group (0.66x, extra pattern, not on the required ladder)** —
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
