//! Regression tests for the segment-join expand walk (Phase-2 perf
//! restructure): multi-group cartesian products must stay byte-identical
//! to the JS reference while outputs are built as segment lists and
//! joined once at materialisation.
//!
//! Expected values verified against micromatch/braces v3.0.3 on the box.

use braces_core::api::expand_str;
use braces_core::Options;

fn expand(p: &str) -> Vec<String> {
    expand_str(p, &Options::default()).expect("expand")
}

#[test]
fn multigroup_range_list_range_full_cartesian() {
    // The BENCH.md losing pattern: 101 x 3 x 10 = 3030 outputs.
    let out = expand("user-{200..300}/project-{a,b,c}-{1..10}");
    assert_eq!(out.len(), 3030);
    // Full cartesian product, left groups vary slowest.
    let mut expected = Vec::with_capacity(3030);
    for n in 200..=300 {
        for c in ["a", "b", "c"] {
            for m in 1..=10 {
                expected.push(format!("user-{n}/project-{c}-{m}"));
            }
        }
    }
    assert_eq!(out, expected);
    assert_eq!(out[0], "user-200/project-a-1");
    assert_eq!(out[3029], "user-300/project-c-10");
}

#[test]
fn multigroup_two_ranges() {
    assert_eq!(
        expand("{1..3}{a..c}"),
        ["1a", "1b", "1c", "2a", "2b", "2c", "3a", "3b", "3c"]
    );
}

#[test]
fn multigroup_text_between_groups() {
    assert_eq!(
        expand("p{q,r}s{1..2}t{u,v}"),
        [
            "pqs1tu", "pqs1tv", "pqs2tu", "pqs2tv", "prs1tu", "prs1tv", "prs2tu", "prs2tv"
        ]
    );
}

#[test]
fn nested_groups_inside_multigroup() {
    assert_eq!(expand("{a,{b,c}d,e}"), ["a", "bd", "cd", "e"]);
    assert_eq!(
        expand("{x,{1..2}}-{p,q}"),
        ["x-p", "x-q", "1-p", "1-q", "2-p", "2-q"]
    );
}

#[test]
fn empty_segments_in_multigroup() {
    // Empty alternatives produce empty segments in the join.
    assert_eq!(expand("{,a}x{,b}"), ["x", "xb", "ax", "axb"]);
    assert_eq!(expand("a{,}{,}"), ["a", "a", "a", "a"]);
    assert_eq!(expand("{,}"), ["", ""]);
}

#[test]
fn escaping_interplay_multigroup() {
    // Escaped brace stays literal while real groups expand.
    assert_eq!(expand("x{a,b}y\\{z}"), ["xay{z}", "xby{z}"]);
    // Escaped comma is not a separator.
    assert_eq!(expand("{a\\,b,c}{1..2}"), ["a,b1", "a,b2", "c1", "c2"]);
    // Escaped range braces do not form a range.
    assert_eq!(expand("\\{1..3\\}"), ["{1..3}"]);
}

#[test]
fn segment_join_preserves_order_and_duplicates() {
    // No dedupe by default; order is stable across the product.
    assert_eq!(expand("{a,a}{b,b}"), ["ab", "ab", "ab", "ab"]);
    let out = expand("{0..2}x{0..2}");
    assert_eq!(
        out,
        ["0x0", "0x1", "0x2", "1x0", "1x1", "1x2", "2x0", "2x1", "2x2"]
    );
}
