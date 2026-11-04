//! Offline request-path timing for complete Strict lists supplied as local filenames (never downloads).
//! Run: `cargo run --release -p vw-preview --example filter_timing -- easylist.txt easyprivacy.txt`
use std::{hint::black_box, time::Instant};
use vw_preview::shields::{Mode, compile, matcher::Matcher};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = std::env::args().skip(1).collect::<Vec<_>>();
    if files.len() != 2 {
        return Err("supply the two saved list filenames".into());
    }
    let lists = format!(
        "{}\n{}",
        std::fs::read_to_string(&files[0])?,
        std::fs::read_to_string(&files[1])?
    );
    let start = Instant::now();
    let json = compile(Mode::Strict, Some(&lists))?;
    let count = serde_json::from_str::<Vec<serde_json::Value>>(&json)?.len();
    let matcher = Matcher::new(&json)?;
    println!(
        "rules={count} input_bytes={} json_bytes={} compile_ms={}",
        lists.len(),
        json.len(),
        start.elapsed().as_millis()
    );
    let vectors = [
        ("https://www.bbc.com/news", "document", true),
        (
            "https://ichef.bbci.co.uk/news/1536/cpsprodpb/image.jpg",
            "image",
            false,
        ),
        (
            "https://securepubads.g.doubleclick.net/gampad/ads?iu=/news",
            "script",
            false,
        ),
        (
            "https://www.google-analytics.com/g/collect?v=2",
            "raw",
            false,
        ),
        ("https://cdn.example.net/js/application.js", "script", false),
        ("https://partner.example.net/frame", "document", false),
        ("https://cdn.example.net/fonts/body.woff2", "font", false),
        ("https://cdn.example.net/style.css", "style-sheet", false),
    ];
    let mut samples = Vec::with_capacity(16000);
    let mut blocked = 0;
    for round in 0..2100 {
        for (url, kind, top) in vectors {
            let start = Instant::now();
            blocked += usize::from(black_box(matcher.blocks(
                black_box(url),
                kind,
                "https://www.bbc.com/",
                top,
            )));
            if round >= 100 {
                samples.push(start.elapsed().as_nanos());
            }
        }
    }
    samples.sort_unstable();
    println!(
        "requests={} blocked={blocked} median_ns={} p95_ns={} p99_ns={} max_ns={}",
        samples.len(),
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
        samples[samples.len() * 99 / 100],
        samples[samples.len() - 1]
    );
    Ok(())
}
