//! Обучаем Колесо на файле, строим перевёрнутую библиотеку (Window 2..5,
//! min_level 1) и смотрим топ значимых ассоциаций по PPMI.

use axmva_ppmi::{filter_by_min_level, score, valid_links, CooAccumulator, ContextExtractor, Window};
use axmva_wheel::WheelState;
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));

    println!("=== elements/ppmi: перевёрнутая библиотека ===");
    let mut wheel = WheelState::new();
    wheel.run_corpus(&bytes, BIRTH_THRESHOLD);
    println!(
        "Прогнали файл ({} байт), полка: {}, длина свёртки: {}.\n",
        bytes.len(),
        wheel.store.len(),
        wheel.sequence.ids.len()
    );

    let events = Window::range(2, 5).extract(&wheel.store, &wheel.sequence.ids);
    let filtered = filter_by_min_level(&wheel.store, events, 1);
    println!("Событий контекста (окно 2..5, level>=1): {}", filtered.len());

    let mut coo = CooAccumulator::new();
    coo.accumulate(&filtered);
    println!("Различных пар: {}", coo.distinct_pairs());
    let csr = coo.bake_to_csr();

    let mut valid = valid_links(&csr, 3, 2.0);
    valid.sort_by(|a, b| b.ppmi.partial_cmp(&a.ppmi).unwrap());
    println!("Валидных связей (PPMI >= 2.0, частота >= 3): {}\n", valid.len());

    println!("Топ-15 ассоциаций по PPMI:");
    for entry in valid.iter().take(15) {
        let token_bytes = wheel.store.bytes_of(entry.token);
        let ctx_bytes = wheel.store.bytes_of(entry.context);
        let token_text = String::from_utf8_lossy(&token_bytes);
        let ctx_text = String::from_utf8_lossy(&ctx_bytes);
        println!(
            "  {:?} ~ {:?} | PPMI={:.3} | вес={} ",
            token_text, ctx_text, entry.ppmi, entry.raw_weight
        );
    }

    // Контроль: та же свёртка с перетасованными позициями -> PPMI должен
    // просесть (перемешивание рвёт реальные ассоциации).
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut shuffled = wheel.sequence.ids.clone();
    let mut seed: u64 = 42;
    for i in (1..shuffled.len()).rev() {
        let mut h = DefaultHasher::new();
        seed.hash(&mut h);
        seed = h.finish();
        let j = (seed as usize) % (i + 1);
        shuffled.swap(i, j);
    }
    let shuffled_events = Window::range(2, 5).extract(&wheel.store, &shuffled);
    let shuffled_filtered = filter_by_min_level(&wheel.store, shuffled_events, 1);
    let mut shuffled_coo = CooAccumulator::new();
    shuffled_coo.accumulate(&shuffled_filtered);
    let shuffled_csr = shuffled_coo.bake_to_csr();
    let shuffled_valid = valid_links(&shuffled_csr, 3, 2.0);

    println!("\n=== Контроль: перетасованная последовательность ===");
    println!("Валидных связей на реальном тексте: {}", valid.len());
    println!("Валидных связей на перетасованном:   {}", shuffled_valid.len());
    if let Some(top) = valid.first() {
        let real_pair_shuffled_weight = score(&shuffled_csr, top.token, top.context, 0);
        println!(
            "PPMI топ-пары на реальном тексте: {:.3}, у той же пары на перетасованном: {:?}",
            top.ppmi, real_pair_shuffled_weight
        );
    }
}
