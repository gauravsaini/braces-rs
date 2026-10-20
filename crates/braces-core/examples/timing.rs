//! Temporary: cached write_many probe.
use braces_core::api;
use braces_core::expand::Rope;
use braces_core::Options;
use std::time::Instant;

fn main() {
    let opts = Options::default();
    let n = 2000u32;
    let p = "user-{200..300}/project-{a,b,c}-{1..10}";
    let ropes = api::expand_str_ropes(p, &opts).unwrap();
    let mut buf: Vec<u16> = Vec::with_capacity(60903);
    // warm the caches
    Rope::write_many(&ropes, &mut buf);
    let t = Instant::now();
    for _ in 0..n {
        buf.clear();
        Rope::write_many(&ropes, &mut buf);
    }
    println!("multi trees (cached): {:?}/call ({} units)", t.elapsed() / n, buf.len());
    // fresh ropes each time, caches cold per call — the real napi path
    let t = Instant::now();
    for _ in 0..n {
        let r = api::expand_str_ropes(p, &opts).unwrap();
        buf.clear();
        Rope::write_many(&r, &mut buf);
        std::hint::black_box(buf.len());
    }
    println!("walk+pack (cold)    : {:?}/call", t.elapsed() / n);
}
