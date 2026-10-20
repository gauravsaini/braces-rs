'use strict';
const native = require('/mnt/sdb1/braces-rs/src-repo/napi/binding.js');
function time(name, fn, iters) {
  for (let i = 0; i < 200; i++) fn();
  const t0 = process.hrtime.bigint();
  for (let i = 0; i < iters; i++) fn();
  const t1 = process.hrtime.bigint();
  const ns = Number(t1 - t0) / iters;
  console.log(name.padEnd(38), (ns / 1000).toFixed(2) + ' us/call', '(' + ns.toFixed(0) + ' ns)');
  return ns;
}
// pure output conversion: n short strings back
const t1000 = time('mkstrings(1000)', () => native.mkstrings(1000), 3000);
const t100 = time('mkstrings(100)', () => native.mkstrings(100), 20000);
const t10 = time('mkstrings(10)', () => native.mkstrings(10), 100000);
console.log('per-string (1000 vs 100):', ((t1000 - t100) / 900).toFixed(1) + ' ns');
console.log('per-string (100 vs 10):', ((t100 - t10) / 90).toFixed(1) + ' ns');
// core+conversion for the big range with the simple-range fast path
time("expand {1..1000}", () => native.expand('{1..1000}', {}), 2000);
time("expand {001..300}", () => native.expand('{001..300}', {}), 5000);
time("expand {0..100..10}", () => native.expand('{0..100..10}', {}), 50000);
time("expand {a..z}", () => native.expand('{a..z}', {}), 50000);
time("expand {a,b,c}", () => native.expand('{a,b,c}', {}), 100000);
time("expand literal", () => native.expand('plain-literal-no-braces-here', {}), 200000);
// appended: latin1 probe
const t1000l1 = time('mkstringsL1(1000)', () => native.mkstringsL1(1000), 3000);
const t100l1 = time('mkstringsL1(100)', () => native.mkstringsL1(100), 20000);
console.log('per-string latin1:', ((t1000l1 - t100l1) / 900).toFixed(1) + ' ns');
