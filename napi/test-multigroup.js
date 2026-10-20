'use strict';
/**
 * test-multigroup.js — JS-level regression test for the segment-join
 * expand walk (Phase-2 perf restructure). Asserts the napi wrapper's
 * output is byte-identical to the JS reference (micromatch/braces v3)
 * for the BENCH.md multi-group pattern and a spread of multi-group
 * variants (nested groups, empty alternatives, escaping interplay).
 *
 * Usage:
 *   node napi/test-multigroup.js [path-to-reference-braces]
 * Default reference: /mnt/sdb1/braces-rs/src-repo/../micromatch-braces
 * (override with BRACES_REF env var).
 */
const assert = require('node:assert');
const path = require('node:path');

const ours = require('./index.js');
const refPath = process.env.BRACES_REF ||
  process.argv[2] ||
  path.join(__dirname, '..', '..', 'micromatch-braces');
const ref = require(refPath);

const MULTI = 'user-{200..300}/project-{a,b,c}-{1..10}';
const CASES = [
  MULTI,
  '{1..3}{a..c}',
  '{a..c}{x..z}',
  'p{q,r}s{1..2}t{u,v}',
  'a{b,c}{1..2}{d,e}',
  '{x,{1..2}}-{p,q}',
  '{a,{b,c}d,e}',
  '{,a}x{,b}',
  'a{,}{,}',
  '{,}',
  'x{a,b}y\\{z}',
  '{a\\,b,c}{1..2}',
  '\\{1..3\\}',
  '{a,a}{b,b}',
  '{0..2}x{0..2}',
  '{200..300}x',
  'pre-{a,b}-mid-{1..5}-post-{x,y,z}',
];

let checks = 0;
for (const p of CASES) {
  const want = ref.expand(p);
  const got = ours.expand(p);
  assert.deepStrictEqual(got, want, `expand mismatch for ${JSON.stringify(p)}`);
  checks++;
  // the default fn (compile mode) returns a 1-element array — deep-compare
  assert.deepStrictEqual(ours(p), ref(p), `default compile mismatch for ${JSON.stringify(p)}`);
  checks++;
  // expand mode of the default fn
  assert.deepStrictEqual(ours(p, { expand: true }), ref(p, { expand: true }), `default expand mismatch for ${JSON.stringify(p)}`);
  checks++;
}

// The headline pattern: exact size + boundary values.
const mg = ours.expand(MULTI);
assert.strictEqual(mg.length, 3030);
assert.strictEqual(mg[0], 'user-200/project-a-1');
assert.strictEqual(mg[3029], 'user-300/project-c-10');
checks += 3;

// expandMany must agree element-for-element on the same set.
const batch = ours.expandMany(CASES);
assert.strictEqual(batch.length, CASES.length);
for (let i = 0; i < CASES.length; i++) {
  assert.deepStrictEqual(batch[i], ref.expand(CASES[i]), `expandMany mismatch for ${JSON.stringify(CASES[i])}`);
  checks++;
}

console.log(`test-multigroup: OK (${checks} assertions, ${CASES.length} patterns)`);
