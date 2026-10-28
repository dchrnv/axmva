//! Подключаем axtime к настоящему epoch+surprise: на каждом реальном
//! обороте вызываем surprise::s_growth(chunk) ДО merge этого оборота и
//! копим в axtime::Time — закрывает интеграцию, которой не хватало
//! (elements/time/passport.yaml, open_questions: "никто пока не вызывает
//! s_growth на каждом реальном обороте").

use axmva_epoch::EpochState;
use axtime::Time;
use std::collections::HashSet;
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));

    println!("=== axtime: виды 1,2,3,5 на реальных данных ===");
    println!("Файл: {path} ({} байт)\n", bytes.len());

    let mut epoch = EpochState::new();
    let mut time: Time<u32> = Time::new();

    for chunk in bytes.chunks(axmva_core::REVOLUTION_CHUNK_BYTES) {
        // S-growth этого оборота — ДО merge, на замороженном сторе этого
        // чанка, как и задумано в черновике ("inner это накопленный
        // S-growth на конец каждого оборота").
        let growth = axmva_surprise::s_growth(&epoch.store, chunk, BIRTH_THRESHOLD);
        time.advance(growth);

        let report = epoch.turn(chunk, BIRTH_THRESHOLD);

        let mut touched_this_turn = HashSet::new();
        for &id in &epoch.sequence.ids {
            if epoch.last_touched_revolution.get(&id) == Some(&report.revolution) {
                touched_this_turn.insert(id);
            }
        }
        for id in touched_this_turn {
            time.touch(id);
        }
    }

    println!("Оборотов: {} | stamp: {}\n", epoch.revolution, time.stamp());

    println!("Примеры age/age_inner/period прямо сейчас (оборот {}):", time.stamp());
    let mut shown = HashSet::new();
    for &id in epoch.sequence.ids.iter() {
        if shown.contains(&id) {
            continue;
        }
        shown.insert(id);
        let bytes_repr = epoch.store.bytes_of(id);
        let preview: String = String::from_utf8_lossy(&bytes_repr).chars().take(10).collect();
        println!(
            "  токен {id:>6} | age = {:>4?} | age_inner = {:>12?} | period = {:>8?} | {preview:?}",
            time.age(id),
            time.age_inner(id),
            time.period(id)
        );
        if shown.len() >= 8 {
            break;
        }
    }
}
