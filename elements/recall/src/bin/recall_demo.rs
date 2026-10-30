//! Крутим recall на реальном файле: оборот до предела, запрос — байты из
//! самого файла (по умолчанию) или переданные вторым аргументом,
//! смотрим топ ассоциаций.

use axmva_epoch::EpochState;
use axmva_recall::{recall, RecallConfig};
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;
const MAX_SEQUENCE_LEN: usize = 50_000;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let query_arg = env::args().nth(2);
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));
    let query_bytes: Vec<u8> = query_arg.map(|s| s.into_bytes()).unwrap_or_else(|| bytes[..bytes.len().min(6)].to_vec());

    println!("=== elements/recall: ассоциативное вспоминание ===");
    println!("Файл: {path} ({} байт)", bytes.len());
    println!("Запрос: {:?}\n", String::from_utf8_lossy(&query_bytes));

    let mut epoch = EpochState::new().with_max_sequence_len(MAX_SEQUENCE_LEN);
    epoch.run_corpus(&bytes, BIRTH_THRESHOLD);
    println!("Полка: {}, длина свёртки: {}\n", epoch.store.len(), epoch.sequence.ids.len());

    let config = RecallConfig::default();
    let result = recall(&epoch.store, &epoch.sequence.ids, &query_bytes, None, &config);

    println!(
        "Распознанные токены запроса: {:?}",
        result
            .query_tokens
            .iter()
            .map(|&t| String::from_utf8_lossy(&epoch.store.bytes_of(t)).to_string())
            .collect::<Vec<_>>()
    );
    println!("\nАссоциации (score, PPMI, вес, уровень, текст):");
    for c in &result.candidates {
        println!("  {:>12.3} | ppmi={:>6.3} | вес={:>5} | уровень={:>2} | {:?}", c.score, c.ppmi, c.raw_weight, c.level, c.text);
    }
    println!("\nКонтекстная строка: {:?}", result.to_context_string());
}
