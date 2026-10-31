//! Даёт ли K=3-отсечение фокуса (elements/death) реальный выигрыш в
//! сжатии невиданного продолжения того же файла, по сравнению с
//! неограниченным (без отсечения) словарём? Измерено на своих данных, не
//! процитирована цифра axmg ("13-23% сжатия на Git").
//!
//! Методология (не копия методологии axmg — своя, воспроизводимая):
//! train режется на обороты (REVOLUTION_CHUNK_BYTES), после КАЖДОГО
//! оборота — mark() + weight_cut() (как в обычном цикле по deck-map:
//! "Сила -> Умеренность -> Смерть после каждого оборота"). На holdout
//! (следующий кусок того же файла, не виденный Колесом) сравниваются:
//! S-static с ограничением фокусом (K=3 + weight_cut) и S-static без
//! ограничения (весь когда-либо рождённый словарь).

use axmva_core::REVOLUTION_CHUNK_BYTES;
use axmva_death::DeathLog;
use axmva_epoch::EpochState;
use axmva_surprise::s_static_focused;
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;
const TRAIN_BYTES: usize = 1_000_000;
const HOLDOUT_BYTES: usize = 100_000;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));
    assert!(
        bytes.len() >= TRAIN_BYTES + HOLDOUT_BYTES,
        "файл слишком короткий для train={TRAIN_BYTES} + holdout={HOLDOUT_BYTES}"
    );

    let train = &bytes[..TRAIN_BYTES];
    let holdout = &bytes[TRAIN_BYTES..TRAIN_BYTES + HOLDOUT_BYTES];

    println!("=== death-focus-bench: K=3-отсечение фокуса vs без ограничения ===");
    println!("Файл: {path} ({} байт)", bytes.len());
    println!("train: первые {TRAIN_BYTES} байт | holdout (невиданный): следующие {HOLDOUT_BYTES} байт\n");

    let mut epoch = EpochState::new();
    let mut death = DeathLog::new();

    let mut turns = 0usize;
    for chunk in train.chunks(REVOLUTION_CHUNK_BYTES) {
        epoch.turn(chunk, BIRTH_THRESHOLD);
        death.mark(&epoch);
        death.weight_cut(&epoch);
        turns += 1;
    }

    println!(
        "Оборотов: {turns} | полка: {} примитивов | в фокусе (K=3 + weight_cut): {} ({:.1}%)\n",
        epoch.store.len(),
        death.focused_set().len(),
        100.0 * death.focused_set().len() as f64 / epoch.store.len() as f64
    );

    let focused_score = s_static_focused(&epoch.store, holdout, Some(death.focused_set()));
    let unrestricted_score = s_static_focused(&epoch.store, holdout, None);

    println!("S-static на holdout, ограничено фокусом (K=3 + weight_cut): {focused_score:.4}");
    println!("S-static на holdout, без ограничения (весь словарь):        {unrestricted_score:.4}");

    if unrestricted_score > 0.0 {
        let pct = (focused_score / unrestricted_score - 1.0) * 100.0;
        println!("\nРазница (фокус относительно неограниченного): {pct:+.1}%");
    }
}
