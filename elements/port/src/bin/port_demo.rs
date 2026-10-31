//! Крутим Port на реальном файле двумя отдельными процессами (как
//! persist_demo save|load) — проверяем, что единственная дверь реально
//! держит и приём, и чтение, и перезапуск.

use axmva_port::{CandidateFilter, Port};
use axmva_recall::RecallConfig;
use std::env;
use std::fs;
use std::path::Path;

const BIRTH_THRESHOLD: u64 = 2;
const DB_PATH: &str = "/tmp/axmva_port_demo.redb";

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "receive".to_string());

    match mode.as_str() {
        "receive" => {
            let file = env::args().nth(2).unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
            let bytes = fs::read(&file).unwrap_or_else(|e| panic!("не удалось прочитать {file}: {e}"));
            let tag = Path::new(&file).file_name().and_then(|n| n.to_str()).unwrap_or("input").to_string();

            println!("=== Port: receive ===");
            let mut port = Port::default();
            // Файл режется по тем же оборотам, что epoch::run_corpus, но
            // через единственную дверь — каждый оборот проходит mark+weight_cut.
            for chunk in bytes.chunks(axmva_core::REVOLUTION_CHUNK_BYTES).take(10) {
                let report = port.receive(chunk, BIRTH_THRESHOLD, &tag).expect("непустой тег");
                println!(
                    "оборот {:>3} | новых: {:>5} | в фокусе: {:>6} | снято weight_cut: {:>4}",
                    report.turn.revolution,
                    report.turn.new_tokens_count,
                    port.death_log().focused_set().len(),
                    report.weight_cut.cut_count
                );
            }

            // Попытка анонимного приёма — отвергается, не проходит молча.
            match port.receive(b"anonymous", BIRTH_THRESHOLD, "") {
                Err(e) => println!("\nПопытка приёма с пустым тегом отвергнута: {e}"),
                Ok(_) => println!("\n!! пустой тег был принят — это баг"),
            }

            port.save(Path::new(DB_PATH)).expect("save");
            println!("\nСохранено в {DB_PATH}");
        }
        "query" => {
            println!("=== Port: query (отдельный процесс, своя память с нуля) ===");
            let port = Port::load(Path::new(DB_PATH), CandidateFilter::default()).expect("load");
            println!(
                "Полка: {} | revolution: {} | в фокусе: {}",
                port.snapshot().store().len(),
                port.epoch().revolution,
                port.death_log().focused_set().len()
            );

            let config = RecallConfig::default();
            let result = port.snapshot().recall(b"the", &config);
            println!("recall('the') через единственную дверь: {:?}", result.to_context_string());

            let surprise = port.snapshot().evaluate_surprise(b"the quick brown fox", BIRTH_THRESHOLD);
            println!("surprise('the quick brown fox'): {surprise:?}");
        }
        other => panic!("неизвестный режим {other:?}, используй receive|query"),
    }
}
