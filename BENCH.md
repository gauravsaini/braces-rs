# BENCH.md — braces-rs vs micromatch/braces v3

Final run: 2026-10-20 (box). Raw output: `bench/results.json` in the repo
(md5 f32a112806a47d22ac52908d588cf47a, verified against the harness's
own write; copy preserved at `/mnt/sdb1/braces-rs/bench-results-run6.json`).

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

## Results (median ops/s) — final run, 2026-10-20

| pattern         | js ops/s |  rs ops/s | speedup |
| --------------- | -------: | --------: | ------: |
| literal         |  653,284 | 1,826,904 |   2.80x |
| small list      |  261,050 | 1,077,382 |   4.13x |
| padded range    |   44,605 |    74,790 |   1.68x |
| stepped range   |  476,261 |   698,556 |   1.47x |
| alpha range     |  424,838 |   771,352 |   1.82x |
| nested          |  229,528 |   294,044 |   1.28x |
| big range       |   16,544 |    24,187 |   1.46x |
| multi-group     |    3,610 |     6,083 |   1.69x |
| compile padded  |  664,825 | 1,096,466 |   1.65x |
| compile stepped |  655,214 | 1,083,533 |   1.65x |
| compile nested  |  860,630 |   969,408 |   1.13x |
| default compile |  722,066 |   899,343 |   1.25x |
| default expand  |  257,733 |   410,791 |   1.59x |
| expandMany      |    1,372 |     2,743 |   2.00x |

**Every pattern in the table beats the JS reference** — the required
ladder and the multi-group extra alike.

## Multi-group: the flat-cartesian path

`user-{200..300}/project-{a,b,c}-{1..10}` (3030 outputs) previously ran
through the general rope walk: 3,324 rs ops/s (0.89x). A stage profile
(parse 1.9µs, walk 221µs, wire pack ~70µs of the ~291µs native call)
put the cost in the walk, not the boundary.

The general walk computes a left-to-right cartesian product of the
per-segment expansions — each root stage pops the whole accumulated
queue and appends the next segment. For patterns whose top level is
only text leaves and simple braces, that product is now computed
directly (`expand_wire` / `expand_str_out` in `crates/braces-core/src/api.rs`):

- **Per-segment items, computed once.** Range braces use the identical
  `reduce` + `exceeds_limit` + `fill` calls the walk uses; list braces
  are comma-split groups of plain values with the walk's exact
  empty-item semantics. Patterns with nesting, parens, or anything the
  walk would recurse into fall back to the general walk unchanged.
- **Single-item segments coalesce** into their neighbours — a text run
  contributes the same bytes to every output, so it is folded into the
  adjacent group's items once instead of being revisited per output.
- **Level-by-level write.** Outputs are built one segment at a time
  into a single staging buffer; the final level's cumulative end
  offsets are the wire header's offsets, and the data section is copied
  once into the packed result. Zero rope-node allocation, zero
  per-output allocation.

Measured effect on the multi-group call (core, box): wire time
291µs → 39µs; end-to-end through the napi wrapper the pattern runs at
the table's 1.69x in the final run. Across the day's
repeated full-bench runs the pattern measured 1.61x–1.94x (the box was
under heavy concurrent load during the session; the table is the final
run, whose results.json was md5-verified against the harness output).

## How the wins were achieved

- UTF-16-native core (exact JS string semantics) with faithful ports of
  parse/expand/compile + fill-range + to-regex-range.
- Fast paths: verbatim literal (no braces/backslash/`]`), bare-range
  `{X..Y[..S]}` for both expand and compile, flat-cartesian for
  multi-segment patterns (above), integer digit rendering
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
