//! Растут ли PPMI-связи (CooAccumulator/Csr) без границ в нашей
//! архитектуре, как предупреждает чертёж axmg (раздел 6: "связи слабое
//! место, растут без остановки")? У нас (и в самом axmg — проверено
//! чтением исходников, cooc.rs используется только эфемерно в death.rs/
//! recall.rs, не как долгоживущая структура) CooAccumulator строится
//! заново из sequence на каждый запрос, не накапливается между запросами.
//! Значит рост числа связей структурно равен O(sequence.len()) — и если
//! sequence ограничена (epoch::max_sequence_len), связи ограничены тоже.
//! Проверяем это измерением, не только рассуждением.

use axmva_epoch::EpochState;
use axmva_ppmi::{filter_by_min_level, CooAccumulator, ContextExtractor, Window};
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;

fn measure(mut epoch: EpochState, bytes: &[u8], label: &str) {
    println!("--- {label} ---");
    println!("{:>8} | {:>10} | {:>12} | {:>12}", "оборот", "sequence", "distinct_pairs", "total_weight");
    for (i, chunk) in bytes.chunks(axmva_core::REVOLUTION_CHUNK_BYTES).enumerate() {
        epoch.turn(chunk, BIRTH_THRESHOLD);
        if (i + 1) % 2 != 0 {
            continue; // не на каждом обороте — просто чтобы не захламлять вывод
        }
        let events = Window::range(2, 5).extract(&epoch.store, &epoch.sequence.ids);
        let filtered = filter_by_min_level(&epoch.store, events, 1);
        let mut coo = CooAccumulator::new();
        coo.accumulate(&filtered);
        println!(
            "{:>8} | {:>10} | {:>12} | {:>12}",
            epoch.revolution,
            epoch.sequence.ids.len(),
            coo.distinct_pairs(),
            coo.total_weight()
        );
    }
    println!();
}

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));
    println!("Файл: {path} ({} байт)\n", bytes.len());

    measure(EpochState::new(), &bytes, "БЕЗ max_sequence_len (как было бы по умолчанию)");
    measure(EpochState::new().with_max_sequence_len(20_000), &bytes, "С max_sequence_len = 20000");
}
