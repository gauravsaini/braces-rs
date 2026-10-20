'use strict';
/**
 * check-conformance.js — EXACT conformance gate for the napi wrapper.
 *
 * Runs the corpus (generated from micromatch/braces v3.0.3, NEVER edited)
 * through the napi wrapper with strict comparison semantics:
 *
 *   - value fixtures: JSON.stringify(got) === JSON.stringify(expected)
 *   - error fixtures: `${err.name}: ${err.message}` === fixture.error
 *     (the corpus stores errors as a single "Name: message" string)
 *
 * Every fixture is called with the pattern as a STRING (the faithful
 * drop-in surface). Fixtures whose recorded reference call used a
 * non-string input (numbers/null/arrays String()-encoded by the
 * generator) cannot be reproduced by any string-arg call — they are
 * reported separately as corpus artifacts (see classify()).
 *
 * Usage:
 *   node tools/check-conformance.js [--corpus PATH] [--wrapper PATH]
 *                                   [--out-json PATH] [--verbose]
 *
 * Defaults: corpus = crates/braces-core/tests/conformance.json
 *           wrapper = napi/index.js
 * Exit code: 0 when zero REAL failures; 1 otherwise.
 */
const fs = require('fs');
const path = require('path');

const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : dflt;
};
const verbose = args.includes('--verbose');
const root = path.join(__dirname, '..');
const corpusPath = opt('--corpus', path.join(root, 'crates/braces-core/tests/conformance.json'));
const wrapperPath = opt('--wrapper', path.join(root, 'napi/index.js'));
const outJson = opt('--out-json', null);

const braces = require(wrapperPath);
const corpus = JSON.parse(fs.readFileSync(corpusPath, 'utf8'));

const FNS = {
  expand: (p, o) => braces.expand(p, o),
  compile: (p, o) => braces.compile(p, o),
  default: (p, o) => braces(p, o),
  create: (p, o) => braces.create(p, o),
  stringify: (p, o) => braces.stringify(p, o),
  parse: (p, o) => braces.parse(p, o),
  expand_ast: (p, o) => braces.expand(braces.parse(p), o),
  compile_ast: (p, o) => braces.compile(braces.parse(p), o),
  stringify_ast: (p, o) => braces.stringify(braces.parse(p), o),
};

/**
 * Fixtures the generator recorded from a NON-string call. `String(input)`
 * encoding in the generator makes these unrecoverable from the pattern
 * field alone, so a string-arg implementation cannot reproduce them.
 * Returns a reason string, or null when the fixture is a genuine
 * string-arg call.
 */
function corpusArtifactReason(fx) {
  const { fn, pattern } = fx;
  const isEmptyOpts = Object.keys(fx.options || {}).length === 0;
  // errors matrix: for (const bad of [null, undefined, 123, true, {}, ['x']])
  if (['parse', 'expand', 'compile'].includes(fn) && isEmptyOpts) {
    const enc = {
      null: 'null (non-string input)',
      undefined: 'undefined (non-string input)',
      123: '123 (number input)',
      true: 'true (boolean input)',
      '[object Object]': '{} (object input)',
      x: "['x'] (array input)",
    }[pattern];
    if (enc) return `generator encoded ${enc} as the string ${JSON.stringify(pattern)}`;
  }
  // create list included the values 0 and null (not strings)
  if (fn === 'create' && (pattern === '0' || pattern === 'null')) {
    return `generator recorded braces.create(${pattern}) with a non-string input, encoded as ${JSON.stringify(pattern)}`;
  }
  // array-input fixtures for the default fn
  if (fn === 'default') {
    if (pattern === '{a,b},{1..3}')
      return 'generator recorded braces(["{a,b}", "{1..3}"]) — array input joined into the pattern field';
    if (pattern === '{a,a},b{a,a}')
      return 'generator recorded braces(["{a,a}", "b{a,a}"]) — array input joined into the pattern field';
    if (pattern === '' && JSON.stringify(fx.result) === '[]')
      return 'generator recorded braces([]) — empty-array input encoded as ""';
  }
  // `.length` fixtures: the generator recorded `run().length`, not the result
  if (typeof fx.result === 'number') {
    return 'generator recorded the result LENGTH (a number), not the result itself';
  }
  return null;
}

let pass = 0;
const failures = [];

for (let i = 0; i < corpus.length; i++) {
  const fx = corpus[i];
  const fn = FNS[fx.fn];
  if (!fn) {
    failures.push({
      idx: i,
      fx,
      got: '(no mapping)',
      category: 'REAL',
      reason: `unknown fn ${fx.fn}`,
    });
    continue;
  }
  let got,
    err = null;
  try {
    got = fn(fx.pattern, fx.options);
  } catch (e) {
    err = e;
  }
  let ok = false;
  let gotStr;
  if (fx.error !== undefined) {
    gotStr = err ? `${err.name}: ${err.message}` : `(no error; result ${JSON.stringify(got)})`;
    ok = !!err && gotStr === fx.error;
  } else {
    gotStr = err ? `${err.name}: ${err.message}` : JSON.stringify(got);
    ok = !err && JSON.stringify(got) === JSON.stringify(fx.result);
  }
  if (ok) {
    pass++;
  } else {
    const art = corpusArtifactReason(fx);
    failures.push({
      idx: i,
      fx,
      got: gotStr,
      category: art ? 'CORPUS-ARTIFACT' : 'REAL',
      reason: art || 'implementation output differs from the reference recording',
    });
  }
}

const real = failures.filter((f) => f.category === 'REAL');
const artifacts = failures.filter((f) => f.category === 'CORPUS-ARTIFACT');

console.log(
  `conformance (napi wrapper, exact): ${pass} pass / ${failures.length} fail / ${corpus.length} total`,
);
console.log(`  REAL failures: ${real.length}`);
console.log(`  corpus artifacts: ${artifacts.length}`);
const show = (f) => {
  const p = String(f.fx.pattern);
  console.log(
    `#${f.idx} [${f.category}] fn=${f.fx.fn} pattern=${JSON.stringify(p.length > 60 ? p.slice(0, 57) + '...' : p)} options=${JSON.stringify(f.fx.options)}`,
  );
  console.log(
    `    expected: ${(f.fx.error !== undefined ? f.fx.error : JSON.stringify(f.fx.result) || '').slice(0, 160)}`,
  );
  console.log(`    got:      ${String(f.got).slice(0, 160)}`);
  console.log(`    reason:   ${f.reason}`);
};
console.log('\n--- REAL failures ---');
real.forEach(show);
if (verbose) {
  console.log('\n--- corpus artifacts ---');
  artifacts.forEach(show);
} else {
  console.log(`\n(corpus artifacts hidden; pass --verbose to list all ${artifacts.length})`);
}

if (outJson) {
  fs.writeFileSync(
    outJson,
    JSON.stringify({ pass, fail: failures.length, total: corpus.length, failures }, null, 1),
  );
  console.log(`\nwrote ${outJson}`);
}
process.exit(real.length === 0 ? 0 : 1);
