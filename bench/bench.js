'use strict';
/**
 * bench.js — benchmark harness for braces-rs vs micromatch/braces v3.
 *
 * Methodology (per LEARNINGS.md #14/#15):
 *  1. EQUIVALENCE GATE first: every bench input runs through both the JS
 *     reference and the Rust binding; deep-equality (order included) is
 *     required before any timing — speed on wrong output is worthless.
 *  2. 5000-iteration warmup per case (V8 JIT gets its best shot).
 *  3. Fixed-duration measurement with process.hrtime.bigint() (one clock).
 *  4. Inputs pre-allocated before timing.
 *  5. >=5 repetitions per case; median reported (min also shown).
 *  6. Same input / same output on both sides — no asymmetric work.
 *
 * Usage: node bench.js <path-to-micromatch-braces> <path-to-braces-rs>
 * e.g.   node bench.js /mnt/sdb1/braces-rs/micromatch-braces \
 *                     /mnt/sdb1/braces-rs/src-repo/napi
 */

const assert = require('assert');

const jsPath = process.argv[2] || './micromatch-braces';
const rsPath = process.argv[3] || './napi';
const jsBraces = require(jsPath);
const rsBraces = require(rsPath);

const WARMUP = 5000;
const DURATION_MS = 3000;
const REPS = 5;

function deepEq(a, b, label) {
  assert.deepStrictEqual(a, b, `equivalence failed for ${label}`);
}

function median(xs) {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}

// ---------------- equivalence gate ----------------
const CASES = [
  { name: 'literal (no braces)', fn: 'expand', pattern: 'plain-literal-no-braces-here' },
  { name: 'small list', fn: 'expand', pattern: '{a,b,c}' },
  { name: 'padded range', fn: 'expand', pattern: '{001..300}' },
  { name: 'stepped range', fn: 'expand', pattern: '{0..100..10}' },
  { name: 'alpha range', fn: 'expand', pattern: '{a..z}' },
  { name: 'nested', fn: 'expand', pattern: '{a,{b..d},e}' },
  { name: 'big range', fn: 'expand', pattern: '{1..1000}' },
  { name: 'multi-group', fn: 'expand', pattern: 'user-{200..300}/project-{a,b,c}-{1..10}' },
  { name: 'compile padded', fn: 'compile', pattern: '{001..300}' },
  { name: 'compile stepped', fn: 'compile', pattern: '{1..10..2}' },
  { name: 'compile nested', fn: 'compile', pattern: 'a/{b,c}/d' },
  { name: 'default compile', fn: 'default', pattern: '{a,b,c}' },
  { name: 'default expand', fn: 'default', pattern: '{a,b,c}', options: { expand: true } },
  {
    name: 'expandMany batch-100',
    fn: 'expandMany',
    patterns: Array.from({ length: 100 }, (_, i) => `file-{${i % 10},a,b}-{1..5}.txt`),
  },
];

console.log('== equivalence gate ==');
for (const c of CASES) {
  let jsOut, rsOut;
  if (c.fn === 'expandMany') {
    jsOut = c.patterns.map((p) => jsBraces.expand(p, c.options || {}));
    rsOut = rsBraces.expandMany(c.patterns, c.options || {});
  } else if (c.fn === 'expand') {
    jsOut = jsBraces.expand(c.pattern, c.options || {});
    rsOut = rsBraces.expand(c.pattern, c.options || {});
  } else if (c.fn === 'compile') {
    jsOut = jsBraces.compile(c.pattern, c.options || {});
    rsOut = rsBraces.compile(c.pattern, c.options || {});
  } else {
    jsOut = jsBraces(c.pattern, c.options || {});
    rsOut = rsBraces(c.pattern, c.options || {});
  }
  deepEq(rsOut, jsOut, `${c.name}`);
  console.log(`  ok: ${c.name}`);
}
console.log('Correctness verified — outputs are byte-identical.\n');

// ---------------- timing ----------------
function bench(label, jsFn, rsFn) {
  const jsTimes = [];
  const rsTimes = [];
  for (let r = 0; r < REPS; r++) {
    for (let i = 0; i < WARMUP; i++) jsFn();
    let end = process.hrtime.bigint() + BigInt(DURATION_MS) * 1000000n;
    let n = 0;
    while (process.hrtime.bigint() < end) {
      jsFn();
      n++;
    }
    jsTimes.push(n / (DURATION_MS / 1000));

    for (let i = 0; i < WARMUP; i++) rsFn();
    end = process.hrtime.bigint() + BigInt(DURATION_MS) * 1000000n;
    n = 0;
    while (process.hrtime.bigint() < end) {
      rsFn();
      n++;
    }
    rsTimes.push(n / (DURATION_MS / 1000));
  }
  return {
    label,
    js: median(jsTimes),
    jsMin: Math.min(...jsTimes),
    rs: median(rsTimes),
    rsMin: Math.min(...rsTimes),
  };
}

const results = [];
const mk = (c) => {
  if (c.fn === 'expandMany') {
    const ps = c.patterns;
    return [
      c.name,
      () => {
        for (const p of ps) jsBraces.expand(p);
      },
      () => rsBraces.expandMany(ps),
    ];
  }
  if (c.fn === 'expand') {
    const p = c.pattern;
    return [c.name, () => jsBraces.expand(p), () => rsBraces.expand(p)];
  }
  if (c.fn === 'compile') {
    const p = c.pattern;
    return [c.name, () => jsBraces.compile(p), () => rsBraces.compile(p)];
  }
  const p = c.pattern,
    o = c.options || {};
  return [c.name, () => jsBraces(p, o), () => rsBraces(p, o)];
};

for (const c of CASES) {
  const [label, jsFn, rsFn] = mk(c);
  process.stdout.write(`benching ${label} ... `);
  const r = bench(label, jsFn, rsFn);
  results.push(r);
  console.log(`js=${Math.round(r.js)}/s rs=${Math.round(r.rs)}/s`);
}

console.log('\n== results (median ops/s, 5 reps, 3s each) ==');
console.log('pattern | js ops/s | rs ops/s | speedup (rs/js)');
const rows = [];
for (const r of results) {
  const su = r.rs / r.js;
  const line = `${r.label} | ${Math.round(r.js)} | ${Math.round(r.rs)} | ${su.toFixed(2)}x`;
  console.log(line);
  rows.push({
    pattern: r.label,
    js: Math.round(r.js),
    rs: Math.round(r.rs),
    speedup: +su.toFixed(2),
    jsMin: Math.round(r.jsMin),
    rsMin: Math.round(r.rsMin),
  });
}
require('fs').writeFileSync(
  __dirname + '/results.json',
  JSON.stringify(
    {
      node: process.version,
      machine: require('os').platform() + '/' + require('os').arch(),
      cpus: require('os').cpus()[0].model,
      results: rows,
    },
    null,
    2,
  ),
);
console.log('\nwrote bench/results.json');
