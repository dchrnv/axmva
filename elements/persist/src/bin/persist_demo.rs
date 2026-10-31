//! Два режима в одной команде: `save` — обучить на файле и сохранить на
//! диск; `load` — открыть процесс с нуля и загрузить то, что сохранено.
//! Запускать как два отдельных процесса (не один прогон), чтобы честно
//! проверить персистентность, а не просто переиспользование памяти.

use axmva_death::DeathLog;
use axmva_epoch::EpochState;
use std::env;
use std::fs;
use std::path::Path;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "save".to_string());
    let db_path = Path::new("/tmp/axmva_persist_demo.redb");

    match mode.as_str() {
        "save" => {
            let file = env::args()
                .nth(2)
                .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
            let bytes = fs::read(&file).unwrap_or_else(|e| panic!("не удалось прочитать {file}: {e}"));

            println!("=== persist: save ===");
            let mut epoch = EpochState::new();
            epoch.run_corpus(&bytes, BIRTH_THRESHOLD);
            let mut log = DeathLog::new();
            log.mark(&epoch);
            log.weight_cut(&epoch);

            println!("Полка: {} примитивов, в фокусе: {}", epoch.store.len(), log.focused_set().len());
            axmva_persist::save_all(&epoch.store, &log, db_path).expect("save_all");
            println!("Сохранено в {db_path:?}");
        }
        "load" => {
            println!("=== persist: load (отдельный процесс, своя память с нуля) ===");
            let (mut store, log) = axmva_persist::load_all(db_path).expect("load_all");
            println!("Загружено из {db_path:?}");
            println!("Полка: {} примитивов, в фокусе: {}", store.len(), log.focused_set().len());
            println!("Событий в логе: {}", log.events().len());

            // Детерминизм после перезагрузки: intern() уже виденного состава
            // не создаёт новый токен, а находит старый id.
            if store.len() > 300 {
                let probe = store.intern(0, &[b'a' as u32]);
                println!("Повторный intern байта 'a' после загрузки -> id {probe} (должен быть < 256, фабричный)");
            }
        }
        "save-epoch" => {
            let file = env::args()
                .nth(2)
                .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
            let bytes = fs::read(&file).unwrap_or_else(|e| panic!("не удалось прочитать {file}: {e}"));
            let tag = Path::new(&file).file_name().and_then(|n| n.to_str()).unwrap_or("input").to_string();

            println!("=== persist: save-epoch (sequence/revolution/last_touched/теги, не только тела) ===");
            let mut epoch = EpochState::new();
            epoch.run_corpus_tagged(&bytes, BIRTH_THRESHOLD, &tag);
            println!(
                "Полка: {} | revolution: {} | длина sequence: {}",
                epoch.store.len(),
                epoch.revolution,
                epoch.sequence.ids.len()
            );
            axmva_persist::save_epoch(&epoch, Path::new("/tmp/axmva_persist_epoch_demo.redb")).expect("save_epoch");
            println!("Сохранено в /tmp/axmva_persist_epoch_demo.redb");
        }
        "load-epoch" => {
            println!("=== persist: load-epoch (отдельный процесс, своя память с нуля) ===");
            let loaded =
                axmva_persist::load_epoch(Path::new("/tmp/axmva_persist_epoch_demo.redb")).expect("load_epoch");
            println!(
                "Полка: {} | revolution: {} | длина sequence: {}",
                loaded.store.len(),
                loaded.revolution,
                loaded.sequence.ids.len()
            );

            // recall() сразу после перезагрузки — работает, потому что
            // sequence теперь реально персистится (см.
            // drafts/axmv2-truth-vs-derived.md). Напоминание: score может
            // отличаться от того, что было до сохранения (счётчики не
            // персистятся намеренно, см. passport.yaml этого элемента),
            // но НАЙДЕННЫЕ кандидаты — те же.
            let config = axmva_recall::RecallConfig::default();
            let result = axmva_recall::recall(&loaded.store, &loaded.sequence.ids, b"the", None, &config);
            println!("recall('the') после перезагрузки: {:?}", result.to_context_string());
        }
        other => panic!("неизвестный режим {other:?}, используй save|load|save-epoch|load-epoch"),
    }
}
