//! Крутим Колесо на реальном файле и метим фокус (mark) после каждого
//! оборота — показывает, что полка растёт, а фокус (достижимое от корней)
//! остаётся маленьким относительно неё.

use axmva_death::DeathLog;
use axmva_epoch::EpochState;
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

    let mut epoch = EpochState::new();
    let mut log = DeathLog::new();

    for chunk in bytes.chunks(axmva_core::REVOLUTION_CHUNK_BYTES) {
        let turn = epoch.turn(chunk, BIRTH_THRESHOLD);
        let outcome = log.mark(&epoch);
        println!(
            "оборот {:>4} | полка: {:>7} | новых: {:>5} | фокус-events: focus {:>5} unfocus {:>5} unchanged {:>7}",
            turn.revolution,
            epoch.store.len(),
            turn.new_tokens_count,
            outcome.newly_focused,
            outcome.newly_unfocused,
            outcome.unchanged
        );
    }

    println!("\n=== После mark (только достижимость) ===");
    println!("Всего в полке: {}", epoch.store.len());
    println!("В фокусе: {}", log.focused_set().len());
    println!(
        "Доля в фокусе: {:.2}%\n",
        100.0 * log.focused_set().len() as f64 / epoch.store.len() as f64
    );

    let cut = log.weight_cut(&epoch);
    println!("=== После weight_cut (срез по весу PPMI среди пограничных) ===");
    println!("Пограничных (достижимых не-корней): {}", cut.borderline_count);
    println!("Снято в этом срезе: {}", cut.cut_count);
    println!("В фокусе теперь: {}", log.focused_set().len());
    println!(
        "Доля в фокусе: {:.2}%",
        100.0 * log.focused_set().len() as f64 / epoch.store.len() as f64
    );
    println!("Всего событий в логе: {}", log.events().len());
}
