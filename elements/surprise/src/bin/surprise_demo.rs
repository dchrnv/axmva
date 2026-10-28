//! Обучаем Колесо на файле, затем проверяем Surprise на знакомом фрагменте
//! того же файла против выдуманной тарабарщины.

use axmva_surprise::surprise;
use axmva_epoch::EpochState;
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));

    println!("=== elements/surprise: обучение ===");
    let mut epoch = EpochState::new();
    epoch.run_corpus(&bytes, BIRTH_THRESHOLD);
    println!("Прогнали файл ({} байт), в полке {} примитивов.\n", bytes.len(), epoch.store.len());

    // Знакомый фрагмент: кусок того же файла, который Колесо уже видело.
    let familiar = &bytes[1000..1200];
    let r1 = surprise(&epoch.store, familiar, BIRTH_THRESHOLD);
    println!("Знакомый фрагмент (байты 1000..1200 того же файла):");
    println!(
        "  s_static={:.3} s_growth={:.3} score={:.3} verdict={:?}\n",
        r1.s_static, r1.s_growth, r1.surprise_score, r1.verdict
    );

    // Выдуманная тарабарщина с повторяющимся новым паттерном.
    let novel = b"zqx zqx zqx vwk vwk qzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
    let r2 = surprise(&epoch.store, novel, BIRTH_THRESHOLD);
    println!("Тарабарщина с повторами (\"zqx zqx zqx vwk vwk qzzz...\"):");
    println!(
        "  s_static={:.3} s_growth={:.3} score={:.3} verdict={:?}",
        r2.s_static, r2.s_growth, r2.surprise_score, r2.verdict
    );
}
