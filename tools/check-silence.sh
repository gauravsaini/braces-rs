#!/usr/bin/env bash
# check-silence.sh — shipped code must produce zero log output.
#
# The library is a silent drop-in: no println!/eprintln!/dbg! in the Rust
# sources, no console.* in the JS wrapper. (Doc comments and test files
# are exempt — tests may print; shipped src/ code may not.)
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
if grep -rnE 'println!|eprintln!|print!|eprint!|dbg!' crates/braces-core/src crates/braces-napi/src 2>/dev/null | grep -v '^\s*[^:]*:[0-9]*:\s*//' | grep -vE ':\s*//' ; then
  echo "check-silence: Rust logging macros found in shipped sources" >&2
  fail=1
fi
if grep -nE 'console\.(log|error|warn|info|debug)' napi/index.js 2>/dev/null; then
  echo "check-silence: console.* found in napi/index.js" >&2
  fail=1
fi
if [ "$fail" -eq 0 ]; then
  echo "check-silence: OK (no logging in shipped Rust sources or JS wrapper)"
fi
exit "$fail"
