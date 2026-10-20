//! Core micro-benches (napi-free — LEARNINGS.md #17), same ladder as the
//! Node harness in bench/bench.js.

use braces_core::{compile_str, expand_str, Options};

fn main() {
    divan::main();
}

const PATTERNS: &[&str] = &[
    "plain-literal-no-braces",
    "a/{b,c}/d",
    "{a,b,c}",
    "{001..300}",
    "{0..100..10}",
    "{a..z}",
    "{a,{b..d},e}",
    "{1..1000}",
    "user-{200..300}/project-{a,b,c}-{1..10}",
    "{a,b}{c,d}{e,f}{g,h}{i,j}",
];

#[divan::bench(args = PATTERNS)]
fn expand(pattern: &str) -> Vec<String> {
    expand_str(pattern, &Options::default()).unwrap()
}

#[divan::bench(args = ["{001..300}", "{1..10..2}", "a/{b,c}/d"])]
fn compile_bench(pattern: &str) -> String {
    compile_str(pattern, &Options::default()).unwrap()
}
