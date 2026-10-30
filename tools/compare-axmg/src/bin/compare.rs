//! Параллельная проверка: один и тот же файл через axmg (референс) и через
//! наш порт (elements/core+wheel+death+ppmi), побитовое сравнение, где это
//! имеет смысл (id, длины, множества), не просто "похоже похоже".

use axmg::context::ContextExtractor as _;
use axmva_ppmi::ContextExtractor as _;
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));

    println!("=== compare-axmg ===");
    println!("Файл: {path} ({} байт)\n", bytes.len());

    let mut ok = true;

    // --- Колесо: обучаем оба на одном и том же входе ---
    let mut ref_wheel = axmg::WheelState::new();
    ref_wheel.run_corpus(&bytes, BIRTH_THRESHOLD, axmg::merge::TieBreak::BirthOrder);

    let mut our_epoch = axmva_epoch::EpochState::new();
    our_epoch.run_corpus(&bytes, BIRTH_THRESHOLD);

    println!("--- Колесо ---");
    ok &= check_eq("revolution", ref_wheel.revolution, our_epoch.revolution);
    ok &= check_eq("store.len()", ref_wheel.store.len(), our_epoch.store.len());
    ok &= check_eq("sequence.len()", ref_wheel.sequence.ids.len(), our_epoch.sequence.ids.len());
    ok &= check_eq("sequence.ids == (побитово)", true, ref_wheel.sequence.ids == our_epoch.sequence.ids);
    ok &= check_eq(
        "last_touched_revolution.len()",
        ref_wheel.last_touched_revolution.len(),
        our_epoch.last_touched_revolution.len(),
    );
    // Тела всех токенов (hash, level, children) — побитово, не только размер полки.
    let mut bodies_match = ref_wheel.store.len() == our_epoch.store.len();
    if bodies_match {
        for id in 0..ref_wheel.store.len() as u32 {
            if ref_wheel.store.hash_of(id) != our_epoch.store.hash_of(id)
                || ref_wheel.store.get(id) != our_epoch.store.get(id)
            {
                bodies_match = false;
                println!("  !! тело токена id={id} разошлось");
                break;
            }
        }
    }
    ok &= check_eq("тела всех токенов идентичны", true, bodies_match);

    // --- Смерть: mark + weight_cut на одном и том же состоянии Колеса ---
    let mut ref_log = axmg::DeathLog::new();
    ref_log.mark(&ref_wheel);
    let ref_cut = ref_log.weight_cut(&ref_wheel);

    let mut our_log = axmva_death::DeathLog::new();
    our_log.mark(&our_epoch);
    let our_cut = our_log.weight_cut(&our_epoch);

    println!("\n--- Смерть ---");
    ok &= check_eq("focused_set().len()", ref_log.focused_set().len(), our_log.focused_set().len());
    ok &= check_eq("events().len()", ref_log.events().len(), our_log.events().len());
    ok &= check_eq("weight_cut.borderline_count", ref_cut.borderline_count, our_cut.borderline_count);
    ok &= check_eq("weight_cut.cut_count", ref_cut.cut_count, our_cut.cut_count);
    let ref_focused: std::collections::BTreeSet<u32> = ref_log.focused_set().iter().copied().collect();
    let our_focused: std::collections::BTreeSet<u32> = our_log.focused_set().iter().copied().collect();
    ok &= check_eq("focused_set как множество id — идентично", true, ref_focused == our_focused);

    // --- PPMI: своя перевёрнутая библиотека у каждого, те же параметры ---
    let ref_events = axmg::context::Window::range(2, 5).extract(&ref_wheel.store, &ref_wheel.sequence.ids);
    let ref_filtered = axmg::context::filter_by_min_level(&ref_wheel.store, ref_events, 1);
    let mut ref_coo = axmg::cooc::CooAccumulator::new();
    ref_coo.accumulate(&ref_filtered);
    let ref_csr = ref_coo.bake_to_csr();
    let mut ref_valid = axmg::ppmi::valid_links(&ref_csr, 3, 2.0);
    ref_valid.sort_by(|a, b| a.token.cmp(&b.token).then(a.context.cmp(&b.context)));

    let our_events = axmva_ppmi::Window::range(2, 5).extract(&our_epoch.store, &our_epoch.sequence.ids);
    let our_filtered = axmva_ppmi::filter_by_min_level(&our_epoch.store, our_events, 1);
    let mut our_coo = axmva_ppmi::CooAccumulator::new();
    our_coo.accumulate(&our_filtered);
    let our_csr = our_coo.bake_to_csr();
    let mut our_valid = axmva_ppmi::valid_links(&our_csr, 3, 2.0);
    our_valid.sort_by(|a, b| a.token.cmp(&b.token).then(a.context.cmp(&b.context)));

    println!("\n--- PPMI ---");
    ok &= check_eq("valid_links.len()", ref_valid.len(), our_valid.len());
    let mut links_match = ref_valid.len() == our_valid.len();
    if links_match {
        for (r, o) in ref_valid.iter().zip(our_valid.iter()) {
            if r.token != o.token || r.context != o.context || r.raw_weight != o.raw_weight || (r.ppmi - o.ppmi).abs() > 1e-9 {
                links_match = false;
                println!("  !! разошлось: ref=({}, {}, {}, {:.6}) наш=({}, {}, {}, {:.6})", r.token, r.context, r.raw_weight, r.ppmi, o.token, o.context, o.raw_weight, o.ppmi);
                break;
            }
        }
    }
    ok &= check_eq("valid_links побитово идентичны", true, links_match);

    // --- Recall: запрос — первые несколько байт самого файла, тела уже
    // проверены идентичными выше, поэтому распознавание обязано совпасть ---
    let query_len = bytes.len().min(5);
    let query_bytes = &bytes[..query_len];
    let recall_config_ref = axmg::recall::RecallConfig::default();
    let recall_config_our = axmva_recall::RecallConfig::default();

    let ref_recall = axmg::recall::recall(&ref_wheel.store, &ref_wheel.sequence.ids, query_bytes, None, &recall_config_ref);
    let our_recall = axmva_recall::recall(&our_epoch.store, &our_epoch.sequence.ids, query_bytes, None, &recall_config_our);

    println!("\n--- Recall ---");
    ok &= check_eq("query_tokens", ref_recall.query_tokens.clone(), our_recall.query_tokens.clone());
    ok &= check_eq("candidates.len()", ref_recall.candidates.len(), our_recall.candidates.len());
    let mut candidates_match = ref_recall.candidates.len() == our_recall.candidates.len();
    if candidates_match {
        for (r, o) in ref_recall.candidates.iter().zip(our_recall.candidates.iter()) {
            let close = r.token == o.token
                && r.text == o.text
                && r.raw_weight == o.raw_weight
                && r.level == o.level
                && (r.score - o.score).abs() < 1e-9
                && (r.ppmi - o.ppmi).abs() < 1e-9;
            if !close {
                candidates_match = false;
                println!(
                    "  !! кандидат разошёлся: ref=({}, {:?}, score={:.6}) наш=({}, {:?}, score={:.6})",
                    r.token, r.text, r.score, o.token, o.text, o.score
                );
                break;
            }
        }
    }
    ok &= check_eq("кандидаты recall побитово идентичны", true, candidates_match);

    println!("\n=== {} ===", if ok { "ВСЁ СОВПАЛО" } else { "ЕСТЬ РАСХОЖДЕНИЯ" });
    std::process::exit(if ok { 0 } else { 1 });
}

fn check_eq<T: PartialEq + std::fmt::Debug>(label: &str, a: T, b: T) -> bool {
    let eq = a == b;
    println!("  {} {label}: {a:?} vs {b:?}", if eq { "✓" } else { "✗" });
    eq
}
