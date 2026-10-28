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
        other => panic!("неизвестный режим {other:?}, используй save|load"),
    }
}
