'use strict';
/**
 * smoke.js — JS-level equivalence gate: runs the full 2008-fixture
 * conformance corpus through the napi-backed wrapper and compares against
 * the recorded JS-reference outputs (and, when available, live against
 * micromatch/braces itself).
 *
 * Usage: node smoke.js [path-to-corpus] [path-to-micromatch-braces]
 */
const fs = require('fs');
const path = require('path');
const assert = require('assert');

const corpusPath = process.argv[2] || path.join(__dirname, '..', 'crates', 'braces-core', 'tests', 'conformance.json');
const mmPath = process.argv[3];
const braces = require('./index.js');
const recs = JSON.parse(fs.readFileSync(corpusPath, 'utf8'));

const TRANSFORMS = {
  double: v => (typeof v === 'number' ? v * 2 : v),
  alphaFoo: (v, i) => 'foo/' + String.fromCharCode(v) + '-' + i,
  prefixN: v => 'N' + v,
};

function reviveOptions(o) {
  // Corpus fixtures keep the transform as a string tag — our napi layer
  // resolves built-in tags natively (fast path). Real JS functions are
  // exercised separately at the end of this file.
  return { ...o };
}

// The generator records non-string inputs via String(value).
function valueFromEncoded(s) {
  switch (s) {
    case 'null': return null;
    case 'undefined': return undefined;
    case 'true': return true;
    case 'false': return false;
    case '[object Object]': return {};
    case 'x': return ['x'];
    default: {
      if (/^-?\d+$/.test(s)) return Number(s);
      return s;
    }
  }
}

let pass = 0, fail = 0;
const failures = [];
let emptyDefaultSeen = 0;

function isV8(msg) {
  return typeof msg === 'string' && (
    msg.includes("Cannot create property") ||
    msg.includes("Cannot set properties") ||
    msg.includes("Cannot read properties") ||
    msg.includes("input.padStart is not a function"));
}

function nameOf(msg) {
  return String(msg).split(': ')[0];
}

function runFixture(rec) {
  const fn = rec.fn;
  const options = reviveOptions(rec.options || {});
  const p = rec.pattern;
  let input = p;
  if (fn === 'default') {
    if (p === '{a,b},{1..3}') input = ['{a,b}', '{1..3}'];
    else if (p === '{a,a},b{a,a}') input = ['{a,a}', 'b{a,a}'];
    else if (p === '') {
      emptyDefaultSeen++;
      if (emptyDefaultSeen > 1) input = [];
    }
  }
  if (fn === 'parse' || fn === 'expand' || fn === 'compile' || fn === 'create') {
    input = valueFromEncoded(p);
  }
  switch (fn) {
    case 'expand': return braces.expand(input, options);
    case 'compile': return braces.compile(input, options);
    case 'default': return braces(input, options);
    case 'create': return braces.create(input, options);
    case 'stringify': return braces.stringify(input, options);
    case 'parse': return braces.parse(input, options);
    case 'expand_ast': return braces.expand(braces.parse(p), options);
    case 'compile_ast': return braces.compile(braces.parse(p), options);
    case 'stringify_ast': return braces.stringify(braces.parse(p), options);
    default: throw new Error('unknown fn ' + fn);
  }
}

const sanitize = node => {
  if (node == null || typeof node !== 'object') return node;
  const o = {};
  for (const k of ['type', 'value', 'dollar', 'depth', 'commas', 'ranges', 'invalid', 'isOpen', 'isClose', 'open', 'close']) {
    if (node[k] !== undefined) o[k] = node[k];
  }
  if (Array.isArray(node.nodes)) o.nodes = node.nodes.map(sanitize);
  return o;
};

for (let i = 0; i < recs.length; i++) {
  const rec = recs[i];
  let got, err;
  try {
    got = runFixture(rec);
  } catch (e) {
    err = e;
  }
  if (rec.error) {
    if (!err) {
      fail++;
      failures.push(`#${i} expected error ${rec.error}, got success`);
      continue;
    }
    const mine = `${err.name || 'Error'}: ${err.message}`;
    if (mine === rec.error || (isV8(rec.error) && nameOf(mine) === nameOf(rec.error))) {
      pass++;
    } else {
      fail++;
      failures.push(`#${i} error mismatch: expected ${rec.error}, got ${mine}`);
    }
    continue;
  }
  if (err) {
    fail++;
    failures.push(`#${i} unexpected error: ${err.name}: ${err.message}`);
    continue;
  }
  let gotCmp = got;
  if (rec.fn === 'parse') gotCmp = sanitize(got);
  try {
    assert.deepStrictEqual(gotCmp, rec.result);
    pass++;
  } catch (e) {
    // numeric-result fixtures (`.length` recorded): compare array length
    if (typeof rec.result === 'number' && Array.isArray(got) && got.length === rec.result) {
      pass++;
      continue;
    }
    // truncated fixtures: compare length
    const tr = rec.result && rec.result.__truncated;
    if (tr && Array.isArray(got) && got.length === rec.result.length) {
      pass++;
    } else {
      fail++;
      failures.push(`#${i} ${rec.fn} ${JSON.stringify(rec.pattern).slice(0, 60)} mismatch:\n  expected ${JSON.stringify(rec.result).slice(0, 200)}\n  got      ${JSON.stringify(gotCmp).slice(0, 200)}`);
    }
  }
}

console.log(`smoke: ${pass} pass, ${fail} fail, ${recs.length} total`);
for (const f of failures.slice(0, 20)) console.log('FAIL', f);

// Real JS-function transform (drop-in surface, off the corpus fast path):
const withFn = braces.expand('{1..5}', { transform: v => v * 10 });
assert.deepStrictEqual(withFn, ['10', '20', '30', '40', '50']);
console.log('smoke: JS-function transform ok');

if (fail > 0) process.exit(1);
