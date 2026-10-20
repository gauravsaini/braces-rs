'use strict';
/**
 * test-corpus-artifacts.js — real-input equivalence tests for the 27
 * corpus "artifact" fixtures.
 *
 * The conformance corpus (crates/braces-core/tests/conformance.json)
 * records some calls whose inputs were NOT strings; the generator stored
 * `String(input)` in the pattern field, so no string-arg call can
 * reproduce those recordings (see tools/conformance-report.md). This
 * suite closes that gap honestly: it replays the ACTUAL calls — the
 * same inputs the generator used — against BOTH the JS reference
 * (micromatch/braces v3) and this wrapper, and asserts byte-identical
 * outcomes: deep-equal return values, and for thrown errors identical
 * `name` AND `message`.
 *
 * Coverage mirrors the generator (gen-conformance.js) exactly:
 *   - errors matrix: parse/expand/compile × [null, undefined, 123,
 *     true, {}, ['x']]                                    (18 cases)
 *   - create × [0, null] × [{}, {expand:true}]            (4 cases)
 *   - default fn: braces(['{a,b}','{1..3}']) × [{},
 *     {expand:true}], braces(['{a,a}','b{a,a}'],
 *     {expand:true, nodupes:true}), braces([])            (4 cases)
 *   - the `.length` fixture: expand('{1..5000}',
 *     {rangeLimit:false}) compared as a full array         (1 case)
 *
 * Usage: node test-corpus-artifacts.js [path-to-micromatch-braces]
 * (defaults to $BRACES_REF or the box reference checkout).
 */
const assert = require('assert');

const refPath =
  process.argv[2] || process.env.BRACES_REF || '/mnt/sdb1/braces-rs/micromatch-braces';
const ref = require(refPath);
const ours = require('./index.js');

let assertions = 0;
let cases = 0;

/** Run `fn`, capturing value-or-error as a comparable outcome. */
function capture(fn) {
  try {
    return { threw: false, value: fn() };
  } catch (e) {
    return {
      threw: true,
      name: e && e.name,
      message: e && e.message,
    };
  }
}

/**
 * Assert both sides produce the identical outcome for the same call:
 * same threw/value kind, deep-equal values, byte-identical error
 * name + message.
 */
function eq(label, refFn, oursFn) {
  cases++;
  const r = capture(refFn);
  const o = capture(oursFn);
  assertions++;
  assert.strictEqual(
    o.threw,
    r.threw,
    `${label}: reference ${r.threw ? `threw ${r.name}: ${r.message}` : 'returned'} but ours ${o.threw ? `threw ${o.name}: ${o.message}` : 'returned'}`,
  );
  if (r.threw) {
    assertions += 2;
    assert.strictEqual(o.name, r.name, `${label}: error name differs`);
    assert.strictEqual(o.message, r.message, `${label}: error message differs`);
  } else {
    assertions++;
    assert.deepStrictEqual(o.value, r.value, `${label}: return value differs`);
  }
}

const show = (v) => (v === undefined ? 'undefined' : (JSON.stringify(v) ?? String(v)));

// ---- errors matrix: parse/expand/compile on non-string inputs ----
for (const bad of [null, undefined, 123, true, {}, ['x']]) {
  eq(
    `parse(${show(bad)})`,
    () => ref.parse(bad),
    () => ours.parse(bad),
  );
  eq(
    `expand(${show(bad)})`,
    () => ref.expand(bad),
    () => ours.expand(bad),
  );
  eq(
    `compile(${show(bad)})`,
    () => ref.compile(bad),
    () => ours.compile(bad),
  );
}

// ---- create on non-string inputs ----
for (const bad of [0, null]) {
  eq(
    `create(${show(bad)})`,
    () => ref.create(bad),
    () => ours.create(bad),
  );
  eq(
    `create(${show(bad)}, {expand:true})`,
    () => ref.create(bad, { expand: true }),
    () => ours.create(bad, { expand: true }),
  );
}

// ---- default fn on array inputs ----
eq(
  "braces(['{a,b}','{1..3}'])",
  () => ref(['{a,b}', '{1..3}']),
  () => ours(['{a,b}', '{1..3}']),
);
eq(
  "braces(['{a,b}','{1..3}'], {expand:true})",
  () => ref(['{a,b}', '{1..3}'], { expand: true }),
  () => ours(['{a,b}', '{1..3}'], { expand: true }),
);
eq(
  "braces(['{a,a}','b{a,a}'], {expand:true, nodupes:true})",
  () => ref(['{a,a}', 'b{a,a}'], { expand: true, nodupes: true }),
  () => ours(['{a,a}', 'b{a,a}'], { expand: true, nodupes: true }),
);
eq(
  'braces([])',
  () => ref([]),
  () => ours([]),
);

// ---- the `.length` fixture: full-array comparison, expected from ref ----
eq(
  "expand('{1..5000}', {rangeLimit:false})",
  () => ref.expand('{1..5000}', { rangeLimit: false }),
  () => ours.expand('{1..5000}', { rangeLimit: false }),
);

console.log(`test-corpus-artifacts: OK (${assertions} assertions, ${cases} cases)`);
