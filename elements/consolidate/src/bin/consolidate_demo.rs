//! Крутим несколько "сессий" на реальном файле, consolidate после
//! каждой, перезапускаем процесс (два отдельных запуска, как
//! persist_demo/port_demo) — холодный вес должен переживать рестарт,
//! в отличие от Store::count_of (намеренно не персистируется).

use axmva_consolidate::ColdWeights;
use axmva_core::TokenId;
use axmva_epoch::EpochState;
use std::env;
use std::fs;
use std::path::Path;

const BIRTH_THRESHOLD: u64 = 2;
const ALPHA: f64 = 0.3;
const DB_PATH: &str = "/tmp/axmva_consolidate_demo.redb";

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "session".to_string());

    match mode.as_str() {
        "session" => {
            let file = env::args().nth(2).unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
            let bytes = fs::read(&file).unwrap_or_else(|e| panic!("не удалось прочитать {file}: {e}"));

            println!("=== consolidate: одна сессия ===");
            let mut epoch = EpochState::new();
            epoch.run_corpus(&bytes[..bytes.len().min(300_000)], BIRTH_THRESHOLD);
            println!("Полка: {} примитивов (горячие счётчики живут только здесь, в памяти)", epoch.store.len());

            let mut cold = if Path::new(DB_PATH).exists() {
                ColdWeights::load(Path::new(DB_PATH)).expect("load предыдущей сессии")
            } else {
                ColdWeights::new()
            };
            println!("Холодных весов до переноса: {}", cold.len());

            cold.consolidate(&epoch.store, ALPHA).expect("α в (0,1]");
            println!("Холодных весов после переноса: {}", cold.len());

            let sample: Vec<TokenId> = epoch.sequence.ids.iter().copied().take(3).collect();
            for id in sample {
                println!("  токен {id:>6} | hot(count_of) = {:>6} | cold (вес) = {:>10.3}", epoch.store.count_of(id), cold.weight_of(id));
            }

            cold.save(Path::new(DB_PATH)).expect("save");
            println!("\nСохранено в {DB_PATH} (отдельно от топологии)");
        }
        "check" => {
            println!("=== consolidate: проверка после перезапуска (отдельный процесс) ===");
            let cold = ColdWeights::load(Path::new(DB_PATH)).expect("load");
            println!("Холодных весов: {} — пережили перезапуск процесса", cold.len());
            println!("Вес токена 'a' (97): {:.3}", cold.weight_of(97));
        }
        other => panic!("неизвестный режим {other:?}, используй session|check"),
    }
}
