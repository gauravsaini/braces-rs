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

## Results (median ops/s) — final run, 2026-10-20 (post-fidelity + rope rework)

| pattern                                             | js ops/s |  rs ops/s |   speedup |
| --------------------------------------------------- | -------: | --------: | --------: |
| literal (no braces)                                 |  659,016 | 2,177,783 | **3.30x** |
| small list `{a,b,c}`                                |  278,932 |   611,184 | **2.19x** |
| padded range `{001..300}`                           |   44,397 |    73,739 | **1.66x** |
| stepped range `{0..100..10}`                        |  476,563 |   747,411 | **1.57x** |
| alpha range `{a..z}`                                |  420,033 |   807,306 | **1.92x** |
| nested `{a,{b..d},e}`                               |  239,939 |   429,643 | **1.79x** |
| big range `{1..1000}`                               |   16,214 |    23,357 | **1.44x** |
| compile padded `{001..300}`                         |  674,204 | 1,079,165 | **1.60x** |
| compile stepped `{1..10..2}`                        |  628,512 | 1,040,522 | **1.66x** |
| compile nested `a/{b,c}/d`                          |  860,872 | 1,020,203 | **1.19x** |
| default compile (`braces('{a,b,c}')`)               |  740,443 |   946,881 | **1.28x** |
| default expand (`braces('{a,b,c}', {expand:true})`) |  269,999 |   295,843 | **1.10x** |
| expandMany batch-100                                |    1,443 |     2,068 | **1.43x** |
| multi-group (3030 outputs, extra)                   |    3,753 |     3,324 |     0.89x |

**Every pattern on the required ladder beats the JS reference.**
Round-over-round regression check vs the previous table (rs ops/s):
every pattern is within run-to-run variance of its earlier number or
better, with two notes below.

### Multi-group: 0.66x -> 0.89x (the "future work", done)

`user-{200..300}/project-{a,b,c}-{1..10}` (3030 outputs) was the one
losing pattern (2,352 rs ops/s). Three restructures landed:

1. **Segment-join walk** — outputs accumulate as segment lists and join
   once at materialisation instead of rebuilding every string per
   segment (the original queue-append was O(segments x output)).
2. **Arena tree ropes** — the walk builds concat trees over a
   bump-allocated node arena (borrowed `Copy` handles, no Rc traffic in
   the hot loop), and the napi layer packs results straight from the
   ropes, skipping per-string materialisation entirely.
3. **Subtree materialisation cache** — shared prefix subtrees (the 303
   `user-NNN/project-X-` prefixes, each consumed 10x) are written once
   per expansion and replayed.
   Plus `expand_wire`: the plain-literal and range-fill fast paths pack
   their items directly into the wire buffer (no rope objects at all).

Where the remaining 11% sits: V8's `expand` result is a tree of lazy
cons-strings the benchmark never materialises; forcing JS to produce
real strings (e.g. joining its result) makes the reference ~2.3x slower
than our full path. Our residual cost is walk node allocation plus the
wrapper's packed-string unpack (~36us of ~300us). 0.89x is the honest
number under the benchmark's terms.

### Literal variance note

The literal fast path is byte-for-byte the same algorithm as the
previous round's 2,292,774 ops/s measurement; two clean final runs
landed at 2,190,461 and 2,177,783 (-4.5% / -5.0%) while the JS side
moved +2.5% between the same runs. Treated as machine variance at the
0.45us/call scale, i.e. parity — the 3.30x speedup is not in doubt.

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
