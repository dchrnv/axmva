//! Крутим Колесо на реальном файле и проверяем модуль времени (пока только
//! в теории, elements/time/passport.yaml) на настоящих данных: age(token)
//! = текущий оборот - last_touched_revolution(token).

use axmva_epoch::EpochState;
use std::env;
use std::fs;
use std::path::Path;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());

    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));
    let tag = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input")
        .to_string();

    println!("=== elements/epoch: turn ===");
    println!("Файл: {path} ({} байт), тег источника: {tag}\n", bytes.len());

    let mut epoch = EpochState::new();
    let reports = epoch.run_corpus(&bytes, BIRTH_THRESHOLD);

    for r in &reports {
        println!(
            "оборот {:>4} | байт: {:>6} | новых примитивов: {:>5} | длина свёртки: {:>6}",
            r.revolution, r.bytes_ingested, r.new_tokens_count, r.sequence_len
        );
    }

    println!("\n=== Время (elements/time) на реальных данных ===");
    println!("Текущий оборот (stamp): {}", epoch.revolution);
    println!("Всего примитивов в полке: {}", epoch.store.len());
    println!("Длина текущей свёрнутой последовательности: {}\n", epoch.sequence.ids.len());

    // age() для первых 5 различных токенов, встреченных в текущей свёртке —
    // показывает именно то, что elements/time/passport.yaml называл
    // гипотезой: "age = сейчас - last_touched, на лету, не хранится".
    let mut shown = std::collections::HashSet::new();
    println!("Примеры age(token) прямо сейчас (оборот {}):", epoch.revolution);
    for &id in epoch.sequence.ids.iter() {
        if shown.contains(&id) {
            continue;
        }
        shown.insert(id);
        let bytes_repr = epoch.store.bytes_of(id);
        let preview: String = String::from_utf8_lossy(&bytes_repr).chars().take(12).collect();
        println!(
            "  токен {id:>6} | уровень {:>2} | age = {:>3} | фрагмент: {:?}",
            epoch.store.level_of(id),
            epoch.age(id).unwrap_or(u32::MAX),
            preview
        );
        if shown.len() >= 5 {
            break;
        }
    }
}
