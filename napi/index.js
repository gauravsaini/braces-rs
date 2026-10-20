'use strict';

/**
 * braces-rs — drop-in replacement for micromatch/braces v3.
 *
 * Hand-written public API layer over the generated napi-rs loader
 * (`binding.js`). The export shape mirrors the original exactly:
 * a callable default function with expand/compile/create/parse/stringify
 * attached. All heavy lifting happens in the Rust core; this file only
 * reproduces the reference's JS-level dispatch (string vs AST input),
 * maps napi's `Name: message` errors back to typed JS errors, and unpacks
 * the native layer's bulk string results.
 *
 * Bulk results cross the FFI boundary **packed** — one joined string plus
 * a Uint32Array of end-offsets — because creating one JS string per
 * element through N-API costs ~130ns/string, more than the reference's
 * whole per-element budget on large expansions. Unpacking slices the
 * joined string with `substring` (byte-identical results).
 */

const native = require('./binding.js');

/**
 * The napi layer surfaces errors as `Name: message`; rethrow them as typed
 * JS errors so the drop-in keeps instanceof fidelity (the reference throws
 * RangeError for rangeLimit, SyntaxError for maxLength, TypeError elsewhere).
 */
const TYPED = { RangeError, SyntaxError, TypeError };
const rethrow =
  (fn) =>
  (...args) => {
    try {
      return fn(...args);
    } catch (e) {
      const m = /^(RangeError|SyntaxError|TypeError): ([\s\S]*)$/.exec(e && e.message);
      if (m && TYPED[m[1]]) throw new TYPED[m[1]](m[2]);
      throw e;
    }
  };

/** Empty/absent options cross as `undefined` — zero per-field N-API reads. */
const nopts = (o) => (o && Object.keys(o).length > 0 ? o : undefined);

/** Unpack `{ offsets, data }` (end-offsets into one joined string). */
function unpackStrings(p) {
  const { offsets, data } = p;
  const out = new Array(offsets.length);
  let s = 0;
  for (let i = 0; i < offsets.length; i++) {
    const e = offsets[i];
    out[i] = data.substring(s, e);
    s = e;
  }
  return out;
}

/**
 * Unpack the single-string packed format used by expand/expandAst/bracesFn:
 * `[count: u32 as 2 chars][end-offsets: count × u32 as 2 chars][data]`.
 */
function unpackPacked(s) {
  const count = (s.charCodeAt(0) | (s.charCodeAt(1) << 16)) >>> 0;
  const out = new Array(count);
  const base = 2 + count * 2;
  let prev = 0;
  for (let i = 0; i < count; i++) {
    const off = (s.charCodeAt(2 + i * 2) | (s.charCodeAt(3 + i * 2) << 16)) >>> 0;
    out[i] = s.substring(base + prev, base + off);
    prev = off;
  }
  return out;
}

/** Unpack `{ offsets, rowEnds, data }` into an array of string arrays. */
function unpackBatch(p) {
  const { offsets, rowEnds, data } = p;
  const rows = new Array(rowEnds.length);
  let si = 0,
    s = 0;
  for (let r = 0; r < rowEnds.length; r++) {
    const end = rowEnds[r];
    const row = new Array(end - si);
    for (let i = si; i < end; i++) {
      const e = offsets[i];
      row[i - si] = data.substring(s, e);
      s = e;
    }
    rows[r] = row;
    si = end;
  }
  return rows;
}

const expandNative = rethrow(native.expand);
const expandAstNative = rethrow(native.expandAst);
const expandManyNative = rethrow(native.expandMany);
const compileNative = rethrow(native.compile);
const compileAstNative = rethrow(native.compileAst);
const createNative = rethrow(native.create);
const bracesFnNative = rethrow(native.bracesFn);
const parseNative = rethrow(native.parse);
const stringifyNative = rethrow(native.stringify);

const isAstLike = (v) => v !== null && typeof v === 'object' && 'type' in v;

/**
 * Expand the given pattern(s) or compile them to a regex-compatible
 * string, depending on `options.expand`. Non-string inputs follow the
 * reference's dispatch (create() per pattern; array flattening), with the
 * default fn's expand&&nodupes dedupe.
 */
const braces = (input, options) => {
  const o = nopts(options);
  if (typeof input === 'string' || Array.isArray(input)) {
    return unpackPacked(bracesFnNative(input, o));
  }
  const r = braces.create(input, options);
  const out = Array.isArray(r) ? r.slice() : [r];
  if (options && options.expand === true && options.nodupes === true) {
    return [...new Set(out)];
  }
  return out;
};

/**
 * Parse the given pattern into an AST.
 */
braces.parse = (input, options) => {
  const o = nopts(options);
  if (typeof input !== 'string') {
    throw new TypeError('Expected a string');
  }
  return parseNative(input, o);
};

/**
 * Create a braces string from a pattern or an AST node.
 * Non-string inputs mirror the reference: null/undefined throw reading
 * `.value`; non-AST values stringify to ''.
 */
braces.stringify = (input, options) => {
  const o = nopts(options);
  if (typeof input === 'string' || isAstLike(input)) {
    return stringifyNative(input, o);
  }
  if (input === null || input === undefined) {
    throw new TypeError(
      `Cannot read properties of ${input === null ? 'null' : 'undefined'} (reading 'value')`,
    );
  }
  return '';
};

/**
 * Compile a brace pattern into a regex-compatible, optimized string.
 * (Returns a string — the original's JSDoc claiming an array is wrong.)
 * Non-string inputs mirror the reference: null/undefined throw reading
 * `.invalid`; numbers/booleans/objects without a `type` compile to ''.
 */
braces.compile = (input, options) => {
  const o = nopts(options);
  if (typeof input === 'string') {
    return compileNative(input, o);
  }
  if (input === null || input === undefined) {
    throw new TypeError(
      `Cannot read properties of ${input === null ? 'null' : 'undefined'} (reading 'invalid')`,
    );
  }
  if (isAstLike(input)) {
    return compileAstNative(input, o);
  }
  return '';
};

/**
 * Expand a brace pattern into an array of strings.
 * Non-string inputs mirror the reference, which walks the value as an
 * AST: null/undefined/primitives throw engine TypeErrors (reproduced
 * with the current V8 wording); objects without AST shape fail reading
 * `.length` inside the walk.
 */
braces.expand = (input, options) => {
  const o = nopts(options);
  if (typeof input === 'string') {
    return unpackPacked(expandNative(input, o));
  }
  if (input === null || input === undefined) {
    throw new TypeError(
      `Cannot set properties of ${input === null ? 'null' : 'undefined'} (setting 'queue')`,
    );
  }
  const t = typeof input;
  if (t === 'number' || t === 'boolean' || t === 'bigint') {
    throw new TypeError(`Cannot create property 'queue' on ${t} '${String(input)}'`);
  }
  if (!isAstLike(input)) {
    throw new TypeError("Cannot read properties of undefined (reading 'length')");
  }
  return unpackPacked(expandAstNative(input, o));
};

/**
 * Process a pattern and return either an expanded array
 * (if `options.expand` is true) or a compiled regex-compatible string.
 * Non-string inputs mirror the reference: null/undefined throw reading
 * `.length`; other values have `.length === undefined`, so the
 * short-circuit is skipped and compile() yields '' (or expand() throws).
 */
braces.create = (input, options) => {
  const o = nopts(options);
  if (typeof input === 'string') {
    const r = createNative(input, o);
    return typeof r === 'string' ? r : unpackStrings(r);
  }
  if (input === null || input === undefined) {
    throw new TypeError(
      `Cannot read properties of ${input === null ? 'null' : 'undefined'} (reading 'length')`,
    );
  }
  if (options && options.expand === true) {
    return braces.expand(input, o);
  }
  return '';
};

/**
 * Batch expansion: expands many patterns in a single native call
 * (amortises the FFI crossing; same results as calling expand per pattern).
 */
braces.expandMany = (patterns, options) => unpackBatch(expandManyNative(patterns, nopts(options)));

module.exports = braces;
module.exports.default = braces;
