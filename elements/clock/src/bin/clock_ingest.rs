//! Крутим Clock (не просто epoch) на реальном файле — age_inner теперь
//! настоящая часть пайплайна, не tools/time-integration-demo.

use axmva_clock::Clock;
use std::collections::HashSet;
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));

    println!("=== elements/clock: age_inner как часть пайплайна ===");
    println!("Файл: {path} ({} байт)\n", bytes.len());

    let mut clock = Clock::new();
    let reports = clock.run_corpus(&bytes, BIRTH_THRESHOLD);

    for r in &reports {
        println!(
            "оборот {:>4} | байт: {:>6} | новых: {:>5} | growth: {:>10.2}",
            r.revolution, r.bytes_ingested, r.new_tokens_count, r.growth
        );
    }

    println!("\nstamp: {} | полка: {}\n", clock.stamp(), clock.store().len());

    let mut shown = HashSet::new();
    println!("Примеры age/age_inner/period прямо сейчас:");
    for &id in clock.epoch().sequence.ids.iter() {
        if shown.contains(&id) {
            continue;
        }
        shown.insert(id);
        let preview: String = String::from_utf8_lossy(&clock.store().bytes_of(id)).chars().take(10).collect();
        println!(
            "  токен {id:>6} | age = {:>4?} | age_inner = {:>14?} | period = {:>8?} | {preview:?}",
            clock.age(id),
            clock.age_inner(id),
            clock.period(id)
        );
        if shown.len() >= 6 {
            break;
        }
    }
}
