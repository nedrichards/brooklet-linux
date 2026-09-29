//! Parse an exported article corpus without GTK, credentials, or network access.
use brooklet::{model::Entry, reader::parse_document};
use std::{error::Error, time::Instant};

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: parse_performance CORPUS.json")?;
    let entries: Vec<Entry> = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut durations = Vec::with_capacity(entries.len());
    let mut blocks = 0;
    for entry in &entries {
        let start = Instant::now();
        let document = parse_document(&entry.html, Some(&entry.url));
        durations.push(start.elapsed().as_secs_f64() * 1000.0);
        blocks += document.len();
        std::hint::black_box(&document);
    }
    durations.sort_by(f64::total_cmp);
    let percentile = |percent: usize| {
        durations
            .get((durations.len().saturating_sub(1) * percent) / 100)
            .copied()
            .unwrap_or(0.0)
    };
    println!(
        "articles={} blocks={} parse_ms p50={:.3} p95={:.3} p99={:.3} max={:.3} over_16.67ms={}",
        entries.len(),
        blocks,
        percentile(50),
        percentile(95),
        percentile(99),
        percentile(100),
        durations
            .iter()
            .filter(|duration| **duration > 1000.0 / 60.0)
            .count()
    );
    Ok(())
}
