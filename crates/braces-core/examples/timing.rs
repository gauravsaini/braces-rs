//! Temporary: pack simulation timing.
use braces_core::api;
use braces_core::expand::Rope;
use braces_core::Options;
use std::time::Instant;

fn main() {
    let p = "user-{200..300}/project-{a,b,c}-{1..10}";
    let opts = Options::default();
    let n = 2000u32;
    let ropes = api::expand_str_ropes(p, &opts).unwrap();
    println!("ropes: {}", ropes.len());

    let t = Instant::now();
    let mut buf: Vec<u16> = Vec::new();
    for _ in 0..n {
        buf.clear();
        Rope::write_many(&ropes, &mut buf);
    }
    println!("write_many: {:?}/call ({} units)", t.elapsed() / n, buf.len());

    // full pack simulation (header + data)
    let t = Instant::now();
    for _ in 0..n {
        let count = ropes.len() as u32;
        let total: usize = ropes.iter().map(|r| r.unit_len()).sum();
        let mut b: Vec<u16> = Vec::with_capacity(2 + ropes.len() * 2 + total);
        b.push((count & 0xFFFF) as u16);
        b.push((count >> 16) as u16);
        let mut pos: u32 = 0;
        for it in &ropes {
            pos += it.unit_len() as u32;
            b.push((pos & 0xFFFF) as u16);
            b.push((pos >> 16) as u16);
        }
        Rope::write_many(&ropes, &mut b);
        std::hint::black_box(&b);
    }
    println!("pack sim:   {:?}/call", t.elapsed() / n);

    // ropes path alone again for reference
    let t = Instant::now();
    for _ in 0..n {
        let r = api::expand_str_ropes(p, &opts).unwrap();
        std::hint::black_box(r.len());
    }
    println!("ropes:      {:?}/call", t.elapsed() / n);
}
