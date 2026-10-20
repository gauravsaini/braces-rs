'use strict';
// Micro-bench: isolate FFI floor, per-string conversion cost, input conversion.
const native = require('/mnt/sdb1/braces-rs/src-repo/napi/binding.js');

function time(name, fn, iters = 200000) {
  for (let i = 0; i < 5000; i++) fn(); // warmup
  const t0 = process.hrtime.bigint();
  for (let i = 0; i < iters; i++) fn();
  const t1 = process.hrtime.bigint();
  const ns = Number(t1 - t0) / iters;
  console.log(name.padEnd(42), (ns / 1000).toFixed(3) + ' us/call');
  return ns;
}

// FFI floor: literal fast path, 1 output string
time('expand literal x', () => native.expand('x', {}), 200000);
// options=undefined path
time('expand literal x (no opts)', () => native.expand('x'), 200000);
// compile literal
time('compile literal x', () => native.compile('x', {}), 200000);

const LIT2000 = Array(2000).fill('x');
const AB2000 = Array(2000).fill('{a,b}');
time('expandMany 2000 literals', () => native.expandMany(LIT2000, {}), 200);
time('expandMany 2000 {a,b}', () => native.expandMany(AB2000, {}), 200);
// big range: core + 1000 output strings
time('expand {1..1000}', () => native.expand('{1..1000}', {}), 2000);
time('compile {001..300}', () => native.compile('{001..300}', {}), 50000);
