//! Крутим Колесо на реальном файле и метим фокус (mark) после каждого
//! оборота — показывает, что полка растёт, а фокус (достижимое от корней)
//! остаётся маленьким относительно неё.

use axmva_death::DeathLog;
use axmva_wheel::WheelState;
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));

    println!("=== elements/death: mark по оборотам ===");
    println!("Файл: {path} ({} байт)\n", bytes.len());

    let mut wheel = WheelState::new();
    let mut log = DeathLog::new();

    for chunk in bytes.chunks(axmva_core::REVOLUTION_CHUNK_BYTES) {
        let turn = wheel.turn(chunk, BIRTH_THRESHOLD);
        let outcome = log.mark(&wheel);
        println!(
            "оборот {:>4} | полка: {:>7} | новых: {:>5} | фокус-events: focus {:>5} unfocus {:>5} unchanged {:>7}",
            turn.revolution,
            wheel.store.len(),
            turn.new_tokens_count,
            outcome.newly_focused,
            outcome.newly_unfocused,
            outcome.unchanged
        );
    }

    println!("\n=== Итог ===");
    println!("Всего в полке: {}", wheel.store.len());
    println!("В фокусе сейчас: {}", log.focused_set().len());
    println!(
        "Доля в фокусе: {:.2}%",
        100.0 * log.focused_set().len() as f64 / wheel.store.len() as f64
    );
    println!("Всего событий в логе: {}", log.events().len());
}
