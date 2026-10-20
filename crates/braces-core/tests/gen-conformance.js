'use strict';
/**
 * gen-conformance.js — runs micromatch/braces v3.0.3 over a wide fixture list
 * and captures {fn, pattern, options, result|error} into conformance.json.
 * Transforms are referenced by tag name (functions are not JSON-serializable).
 */
const fs = require('fs');
const braces = require('./micromatch-braces');

const TRANSFORMS = {
  double: v => (typeof v === 'number' ? v * 2 : v),
  alphaFoo: (v, i) => 'foo/' + String.fromCharCode(v) + '-' + i,
  prefixN: v => 'N' + v,
};

const out = [];

const record = (fn, pattern, options, run) => {
  const rec = { fn, pattern: typeof pattern === 'string' ? pattern : String(pattern), options: options || {} };
  try {
    let result = run();
    if (Array.isArray(result) && result.length > 20000) {
      rec.result = { __truncated: true, length: result.length, head: result.slice(0, 20) };
    } else if (typeof result === 'string' && result.length > 100000) {
      rec.result = { __truncated: true, length: result.length, head: result.slice(0, 200) };
    } else {
      rec.result = result === undefined ? null : result;
    }
  } catch (err) {
    rec.error = err.name + ': ' + err.message;
  }
  out.push(rec);
};

const sanitize = node => {
  if (node == null || typeof node !== 'object') return node;
  const o = {};
  for (const k of ['type', 'value', 'dollar', 'depth', 'commas', 'ranges', 'invalid', 'isOpen', 'isClose', 'open', 'close']) {
    if (node[k] !== undefined) o[k] = node[k];
  }
  if (Array.isArray(node.nodes)) o.nodes = node.nodes.map(sanitize);
  return o;
};

/* ---------------------------------- patterns ---------------------------------- */
const PATTERNS = [
  // basic lists
  '{a,b,c}', 'a/{b,c}/d', 'a/{foo,bar,baz}/*.js', '{}', '{a}', '{,}', '{a,}', '{,a}',
  '{a,,b}', 'a{,}', 'a{,}{,}', '{a,b}{c,d}', '{a,b}x{c,d}y', '{1,2}{3,4}{5,6}',
  'a{b,c,de}', '{a,b,c,d,e,f}', '{longer,words,here}', '{a,b}{c,d}{e,f}',
  '{ a , b }', '{a, b}', '{ a,b}', '{a,b }', '{,a,b,}', '{{a}}', '{a,{b}}',
  // numeric ranges
  '{1..3}', '{1..100}', '{0..9}', '{9..0}', '{01..03}', '{001..300}', '{0001..10}',
  '{-5..5}', '{5..-5}', '{4..-4}', '{-9..-1}', '{-10..-5}', '{-3..3}', '{08..12}',
  '{-0..5}', '{+1..5}', '{1..10..2}', '{2..10..2}', '{1..20..3}', '{0..100..10}',
  '{0..100..0}', '{1..5..-1}', '{1..5..0}', '{10..1..2}', '{100..90..5}',
  '{1..1000}', '{1..1001}', '{1..1000000}', '{1..9007199254740991}',
  '{1..}', '{..5}', '{...}', '{a..1}', '{1..a}', '{1..2..3..4}', '{1.5..3}',
  '{1..2.}', '{a...b}', '{a..b..c..d}', '{a....b}', '{.}', '{..}', '{..a}',
  // alpha ranges
  '{a..z}', '{z..a}', '{A..Z}', '{Z..A}', '{a..e..2}', '{a..z..3}', '{j..a}',
  '{a..Z}', '{A..z}', '{a..9}', '{9..a}', '{aa..bb}', '{ab..cd}', '{a..aa}',
  '{!..~}', '{a..e..x}', '{a..e..0}',
  // nested
  'a{b,c,/{x,y}}/e', 'a/{x,{1..5},y}/c', '{a,{b,c}}', '{{a,b},c}', 'x/{a,{b,{c,d}}}/y',
  '{a,{1..3}{4..5}}', '{a,b{c,d{e,f}}}', '{a,b,{c,{d,{e,f}}}}', '{1..3}{a..c}',
  'a/{b,c{d,e}f}/g', '{a,{b,c},d}', '{{1..3},{a,b}}',
  // escapes (JS-escaped below: these are literal backslashes in the pattern)
  'a\\{d,c,b}e', 'a{d,c,b\\}e', 'a{b\\,c}d', 'a{d\\,c,b}e', '\\{a,b}', '{a,b}\\}',
  '{a\\,b,c}', '\\\\{a,b}', '{a\\\\,b}', '{\\a,b}', '{a,b\\/c}', 'x\\ny{a,b}',
  '{a\\ b,c}', 'a\\,b{c,d}', '\\{1..3\\}',
  // quotes
  "{'a,b'}", '{"a,b"}', '{`foo,bar`}', "{a,'b,c',d}", 'a"b,c{d,e}"f',
  "{'a,b}", '"{a,b}"', "{`a`,`b`}", 'a\'b{c,d}e', '{"a","b"}',
  // brackets (glob char classes)
  'a/[{b,c}]/d', 'a/{a,b,[{c,d}]}/e', '[]', 'a[bc]d{e,f}', ']', 'a]b{c,d}',
  '[a-z]{1,2}', 'a[', 'a[b{c,d}', '[{a,b}]', 'a[]b{c,d}', '[[a]]{x,y}',
  // parens / extglob-ish
  '*(a|{b|c,d})', '{a,@(b,c)}', '{a,*(b|c,d)}', '(a|b)', 'a(b{c,d}', 'a)b{c,d}',
  '?(a|{b,c})', '+(a|b){x,y}', '@(a|b){1..2}',
  // dollar braces (bash parameter expansion lookalikes)
  '${a,b}', 'a${1..3}b', '${a}', '${foo}', 'x${a,b}y', '$ {a,b}',
  // invalid / unbalanced
  '{a,b', 'a,b}', '{', '}', '{a', '{{a,b}', 'a{b}}c', '{a,b}}', '{}{a,b}',
  '{a,b}{', '}{a,b}{', '{a{b,c}', 'a{b,c}}d', '{}a{b,c}', '{a,b}c}',
  // plain / trivial
  '', 'a', 'ab', 'no braces at all', '  {a,b}  ', '/path/to/file.js',
  '{a,b}/file-{1..3}.txt', 'user-{200..300}/project-{a,b,c}-{1..10}',
  // unicode & weird chars
  '{α,β,γ}', '{é,è,ê}', '🎉{a,b}', 'a{😀,😃}b', '{a b,c}', '{﻿a,b}',
  '{a,b}﻿', '{日本,中国}', '{\\u0041,b}', '{tab\there,other}',
  // mixed list + range
  '{a,1..3,b}', '{x,{1..2},y,{a..b}}', '{0,2,4..8,10}', '{a..c,1..3}',
  '{1..3,a..c}', '{-2..2,{p,q}}',
  // regex-quantifier lookalikes
  'a/b{1,3}/{x,y,z}', 'a{2,4}b', 'x{1,}y', 'z{,5}w', 'a{2}b{3}',
  // steps with ranges edge
  '{0..10..3}', '{10..0..-3}', '{-10..10..5}', '{5..5}', '{a..a}', '{0..0}',
  '{00..05}', '{-05..05}', '{099..101}',
];

/* ------------------------------ main fn matrix ------------------------------ */
for (const p of PATTERNS) {
  record('expand', p, {}, () => braces.expand(p));
  record('compile', p, {}, () => braces.compile(p));
  record('default', p, {}, () => braces(p));
}

/* ------------------------------- options matrix ------------------------------ */
const CORE = [
  '{a,b,c}', 'a/{x,y}/b', '{1..30}', '{a..j}', '{a,a,b,b}c', '{,a,,b,}',
  'a\\{b,c}d', '{a,b}', '{01..20}', '{1..10..3}', '${a,b}', '{a,b',
  '{`q,w`}', 'a[bc]{d,e}', '{x,{y,z}}', '{a...b}', '{-4..4}',
];
const OPTSETS = [
  { expand: true },
  { expand: true, nodupes: true },
  { nodupes: true },
  { expand: true, noempty: true },
  { noempty: true },
  { keepEscaping: true },
  { keepEscaping: true, expand: true },
  { keepQuotes: true },
  { keepQuotes: true, expand: true },
  { escapeInvalid: true },
  { escapeInvalid: true, expand: true },
  { quantifiers: true },
  { quantifiers: true, expand: true },
  { rangeLimit: 5 },
  { rangeLimit: false },
  { rangeLimit: 0 },
  { maxLength: 4 },
  { step: 2 },
  { capture: true },
  { wrap: true },
  { strictZeros: true },
  { shorthand: true },
  { relaxZeros: false },
  { transform: 'double' },
  { transform: 'alphaFoo' },
  { transform: 'prefixN' },
];
for (const p of CORE) {
  for (const opts of OPTSETS) {
    const label = JSON.parse(JSON.stringify(opts));
    const real = { ...opts };
    if (typeof real.transform === 'string') real.transform = TRANSFORMS[real.transform];
    record('expand', p, label, () => braces.expand(p, real));
    record('compile', p, label, () => braces.compile(p, real));
    record('default', p, label, () => braces(p, real));
  }
}

/* ------------------------------ create / stringify ---------------------------- */
for (const p of ['', 'a', 'ab', 'abc', '{a,b}', 'a{b,c}d', '{1..5}', 0, null]) {
  record('create', p, {}, () => braces.create(p));
  record('create', p, { expand: true }, () => braces.create(p, { expand: true }));
}
for (const p of ['a/{b,c}/d', '{a,b', '${x,y}', '{a...b}', 'plain', '{1..3}']) {
  record('stringify', p, {}, () => braces.stringify(p));
  record('stringify', p, { escapeInvalid: true }, () => braces.stringify(p, { escapeInvalid: true }));
}

/* --------------------------------- array input -------------------------------- */
record('default', ['{a,b}', '{1..3}'], {}, () => braces(['{a,b}', '{1..3}']));
record('default', ['{a,b}', '{1..3}'], { expand: true }, () => braces(['{a,b}', '{1..3}'], { expand: true }));
record('default', ['{a,a}', 'b{a,a}'], { expand: true, nodupes: true }, () => braces(['{a,a}', 'b{a,a}'], { expand: true, nodupes: true }));
record('default', [], {}, () => braces([]));

/* ----------------------------------- errors ----------------------------------- */
for (const bad of [null, undefined, 123, true, {}, ['x']]) {
  record('parse', bad, {}, () => braces.parse(bad));
  record('expand', bad, {}, () => braces.expand(bad));
  record('compile', bad, {}, () => braces.compile(bad));
}
record('expand', 'x'.repeat(10001), {}, () => braces.expand('x'.repeat(10001)));
record('expand', '{a,b}', { maxLength: 3 }, () => braces.expand('{a,b}', { maxLength: 3 }));
record('parse', 'x'.repeat(65 * 1024), {}, () => braces.parse('x'.repeat(65 * 1024)));
record('expand', '{1..5000}', {}, () => braces.expand('{1..5000}'));
record('expand', '{1..5000}', { rangeLimit: false }, () => braces.expand('{1..5000}', { rangeLimit: false }).length);
record('expand', '{a..z}{a..z}{a..z}', {}, () => braces.expand('{a..z}{a..z}{a..z}'));

/* ------------------------------- AST round-trips -------------------------------- */
for (const p of ['a/{b,c}/d', '{1..5}', '{a,b', '${x,y}', 'a[{b,c}]d', '{a,{b,c}}', "'{a,b}'", '{a...b}', '{,a,}']) {
  record('parse', p, {}, () => sanitize(braces.parse(p)));
}
for (const p of ['a/{b,c}/d', '{01..10}', '{a,b']) {
  record('expand_ast', p, {}, () => braces.expand(braces.parse(p)));
  record('compile_ast', p, {}, () => braces.compile(braces.parse(p)));
  record('stringify_ast', p, {}, () => braces.stringify(braces.parse(p)));
}

/* ------------------------------------ write ----------------------------------- */
fs.writeFileSync('/tmp/braceslane-a/conformance.json', JSON.stringify(out, null, 1));
console.log('fixtures:', out.length);
const errs = out.filter(r => r.error);
console.log('errors captured:', errs.length);
