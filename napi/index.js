'use strict';

/**
 * braces-rs — drop-in replacement for micromatch/braces v3.
 *
 * Hand-written public API layer over the generated napi-rs loader
 * (`binding.js`). The export shape mirrors the original exactly:
 * a callable default function with expand/compile/create/parse/stringify
 * attached. All heavy lifting happens in the Rust core; this file only
 * reproduces the reference's JS-level dispatch (string vs AST input) and
 * maps napi's `Name: message` errors back to typed JS errors.
 */

const native = require('./binding.js');

/**
 * The napi layer surfaces errors as `Name: message`; rethrow them as typed
 * JS errors so the drop-in keeps instanceof fidelity (the reference throws
 * RangeError for rangeLimit, SyntaxError for maxLength, TypeError elsewhere).
 */
const TYPED = { RangeError, SyntaxError, TypeError };
const rethrow = fn => (...args) => {
  try {
    return fn(...args);
  } catch (e) {
    const m = /^(RangeError|SyntaxError|TypeError): ([\s\S]*)$/.exec(e && e.message);
    if (m && TYPED[m[1]]) throw new TYPED[m[1]](m[2]);
    throw e;
  }
};

const expandNative = rethrow(native.expand);
const expandAstNative = rethrow(native.expandAst);
const expandManyNative = rethrow(native.expandMany);
const compileNative = rethrow(native.compile);
const compileAstNative = rethrow(native.compileAst);
const createNative = rethrow(native.create);
const bracesFnNative = rethrow(native.bracesFn);
const parseNative = rethrow(native.parse);
const stringifyNative = rethrow(native.stringify);

const isAstLike = v => v !== null && typeof v === 'object' && 'type' in v;

/**
 * Expand the given pattern(s) or compile them to a regex-compatible
 * string, depending on `options.expand`. Non-string inputs follow the
 * reference's dispatch (create() per pattern; array flattening), with the
 * default fn's expand&&nodupes dedupe.
 */
const braces = (input, options = {}) => {
  if (typeof input === 'string' || Array.isArray(input)) {
    return bracesFnNative(input, options);
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
braces.parse = (input, options = {}) => {
  if (typeof input !== 'string') {
    throw new TypeError('Expected a string');
  }
  return parseNative(input, options);
};

/**
 * Create a braces string from a pattern or an AST node.
 * Non-string inputs mirror the reference: null/undefined throw reading
 * `.value`; non-AST values stringify to ''.
 */
braces.stringify = (input, options = {}) => {
  if (typeof input === 'string' || isAstLike(input)) {
    return stringifyNative(input, options);
  }
  if (input === null || input === undefined) {
    throw new TypeError(`Cannot read properties of ${input === null ? 'null' : 'undefined'} (reading 'value')`);
  }
  return '';
};

/**
 * Compile a brace pattern into a regex-compatible, optimized string.
 * (Returns a string — the original's JSDoc claiming an array is wrong.)
 * Non-string inputs mirror the reference: null/undefined throw reading
 * `.invalid`; numbers/booleans/objects without a `type` compile to ''.
 */
braces.compile = (input, options = {}) => {
  if (typeof input === 'string') {
    return compileNative(input, options);
  }
  if (input === null || input === undefined) {
    throw new TypeError(`Cannot read properties of ${input === null ? 'null' : 'undefined'} (reading 'invalid')`);
  }
  if (isAstLike(input)) {
    return compileAstNative(input, options);
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
braces.expand = (input, options = {}) => {
  if (typeof input === 'string') {
    return expandNative(input, options);
  }
  if (input === null || input === undefined) {
    throw new TypeError(`Cannot set properties of ${input === null ? 'null' : 'undefined'} (setting 'queue')`);
  }
  const t = typeof input;
  if (t === 'number' || t === 'boolean' || t === 'bigint') {
    throw new TypeError(`Cannot create property 'queue' on ${t} '${String(input)}'`);
  }
  if (!isAstLike(input)) {
    throw new TypeError("Cannot read properties of undefined (reading 'length')");
  }
  return expandAstNative(input, options);
};

/**
 * Process a pattern and return either an expanded array
 * (if `options.expand` is true) or a compiled regex-compatible string.
 * Non-string inputs mirror the reference: null/undefined throw reading
 * `.length`; other values have `.length === undefined`, so the
 * short-circuit is skipped and compile() yields '' (or expand() throws).
 */
braces.create = (input, options = {}) => {
  if (typeof input === 'string') {
    return createNative(input, options);
  }
  if (input === null || input === undefined) {
    throw new TypeError(`Cannot read properties of ${input === null ? 'null' : 'undefined'} (reading 'length')`);
  }
  if (options && options.expand === true) {
    return braces.expand(input, options);
  }
  return '';
};

/**
 * Batch expansion: expands many patterns in a single native call
 * (amortises the FFI crossing; same results as calling expand per pattern).
 */
braces.expandMany = (patterns, options = {}) => expandManyNative(patterns, options);

module.exports = braces;
module.exports.default = braces;
