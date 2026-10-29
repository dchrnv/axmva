//! Измеряет время каждого оборота (turn) как функцию накопленной длины
//! последовательности — реальная диагностика перед решением о лимите
//! (axmg режет на 100_000 ad hoc, мы — нет изначально, см.
//! elements/epoch/passport.yaml). Второй аргумент (опционально) включает
//! `with_max_sequence_len` — показывает, что время на оборот перестаёт
//! расти после включения лимита, и что обрезка видна в `truncated_count`.

use axmva_epoch::EpochState;
use std::env;
use std::fs;
use std::time::Instant;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/games/Carlsen.pgn".to_string());
    let max_sequence_len: Option<usize> = env::args().nth(2).map(|s| s.parse().expect("max_sequence_len должен быть числом"));
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));

    println!("=== epoch-cost-bench ===");
    println!(
        "Файл: {path} ({} байт), лимит последовательности: {}\n",
        bytes.len(),
        max_sequence_len.map(|m| m.to_string()).unwrap_or_else(|| "нет (как раньше)".to_string())
    );
    println!("{:>6} {:>14} {:>10} {:>10}", "оборот", "seq_len_после", "обрезано", "мс");

    let mut epoch = EpochState::new();
    if let Some(max) = max_sequence_len {
        epoch = epoch.with_max_sequence_len(max);
    }

    for chunk in bytes.chunks(axmva_core::REVOLUTION_CHUNK_BYTES) {
        let start = Instant::now();
        let report = epoch.turn(chunk, BIRTH_THRESHOLD);
        let elapsed = start.elapsed();
        println!(
            "{:>6} {:>14} {:>10} {:>10.3}",
            report.revolution,
            report.sequence_len,
            report.truncated_count,
            elapsed.as_secs_f64() * 1000.0
        );
    }
}
