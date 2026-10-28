//! Механизм: взять файл и разобрать его на примитивы.
//! Оборот = порция REVOLUTION_CHUNK_BYTES (64 КБ); хранилище (Store) одно
//! на весь файл и растёт по ходу — слияние (merge::run) идёт внутри каждой
//! порции отдельно, через границу оборота пары не сливаются (это и есть
//! смысл "оборота" — единица входа, не технический предел).
//! Тег источника = имя файла (см. elements/external-input/passport.yaml).

use axmva_core::{merge, Store, REVOLUTION_CHUNK_BYTES};
use std::env;
use std::fs;
use std::path::Path;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());

    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));
    let tag = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input")
        .to_string();

    println!("=== elements/core: ingest ===");
    println!("Файл: {path} ({} байт), тег источника: {tag}", bytes.len());
    println!("Порция (оборот) = {REVOLUTION_CHUNK_BYTES} байт, порог рождения = {BIRTH_THRESHOLD}\n");

    let mut store = Store::new();
    store.init_factory();

    for (i, chunk) in bytes.chunks(REVOLUTION_CHUNK_BYTES).enumerate() {
        let before = store.len();
        let ids: Vec<u32> = chunk.iter().map(|&b| b as u32).collect();
        let seq = merge::Sequence::from_ids(&store, ids);
        let out = merge::run(&mut store, seq, BIRTH_THRESHOLD);
        let new_tokens = store.len() - before;
        println!(
            "оборот {i:>4} | байт: {:>6} | свёрнуто в: {:>6} примитивов | новых: {:>5} | всего в полке: {:>6}",
            chunk.len(),
            out.len(),
            new_tokens,
            store.len()
        );
    }

    println!("\n=== Итог ===");
    println!("Всего примитивов в полке (включая 256 фабричных): {}", store.len());
}
