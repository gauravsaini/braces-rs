'use strict';

/**
 * braces-rs — drop-in replacement for micromatch/braces v3.
 *
 * Hand-written public API layer over the generated napi-rs loader
 * (`binding.js`). The export shape mirrors the original exactly:
 * a callable default function with expand/compile/create/parse/stringify
 * attached. All heavy lifting happens in the Rust core; this file only
 * reproduces the reference's JS-level dispatch (string vs AST input).
 */

const native = require('./binding.js');

/**
 * Expand the given pattern(s) or compile them to a regex-compatible
 * string, depending on `options.expand`.
 */
const braces = (input, options = {}) => native.bracesFn(input, options);

/**
 * Parse the given pattern into an AST.
 */
braces.parse = (input, options = {}) => native.parse(input, options);

/**
 * Create a braces string from a pattern or an AST node.
 */
braces.stringify = (input, options = {}) => native.stringify(input, options);

/**
 * Compile a brace pattern into a regex-compatible, optimized string.
 * (Returns a string — the original's JSDoc claiming an array is wrong.)
 */
braces.compile = (input, options = {}) => {
  if (typeof input === 'string') {
    return native.compile(input, options);
  }
  return native.compileAst(input, options);
};

/**
 * Expand a brace pattern into an array of strings.
 */
braces.expand = (input, options = {}) => {
  if (typeof input === 'string') {
    return native.expand(input, options);
  }
  return native.expandAst(input, options);
};

/**
 * Process a pattern and return either an expanded array
 * (if `options.expand` is true) or a compiled regex-compatible string.
 */
braces.create = (input, options = {}) => native.create(input, options);

/**
 * Batch expansion: expands many patterns in a single native call
 * (amortises the FFI crossing; same results as calling expand per pattern).
 */
braces.expandMany = (patterns, options = {}) => native.expandMany(patterns, options);

module.exports = braces;
module.exports.default = braces;
