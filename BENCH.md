# BENCH.md — braces-rs vs micromatch/braces v3

> Box: `/mnt/sdb1/braces-rs/` · raw outputs: `bench/results.json` (per-run medians)

## Machine

- CPU: (fill from `lscpu`)
- RAM: (fill from `free -h`)
- OS: Ubuntu 25.10 (Linux 6.17.0-41-generic x86_64)
- Node: v20.19.4
- rustc: 1.98.1
- Build: `cargo build --release` with `lto="fat" codegen-units=1 opt-level=3 strip=true`,
  bench-only `RUSTFLAGS="-C target-cpu=native"`, `CARGO_TARGET_DIR=/home/muse/.target/braces-rs-native`
- napi binding: `napi build --platform --release` (@napi-rs/cli 2.18.4)

## Method

`bench/bench.js <micromatch-braces> <braces-rs-napi>`:

1. **Equivalence gate** — every bench input's Rust output is asserted
   `deepStrictEqual` against the JS reference before timing starts.
2. 5000-iteration warmup per pattern.
3. Fixed 3 s measurement window per repetition, `process.hrtime.bigint()`,
   inputs pre-allocated outside the loop.
4. ≥5 repetitions, **median** reported.

## Results (median ops/s)

| pattern | js ops/s | rs ops/s | speedup |
|---------|---------:|---------:|--------:|
| (filled after final run) |

## Reproduce

```sh
cd /mnt/sdb1/braces-rs/src-repo/bench
node bench.js /mnt/sdb1/braces-rs/micromatch-braces ../napi
# core-only (no napi): cargo bench -p braces-core --bench expand_bench
```

## Notes

- The napi binding used for benches is built with `target-cpu=native`
  (bench-only; shipped per-triple builds never set it).
- `expandMany` batch-100 amortises one FFI crossing + one options conversion
  over 100 patterns.
