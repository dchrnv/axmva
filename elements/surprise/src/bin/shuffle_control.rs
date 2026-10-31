//! Количественный перетасованный контроль для S-static — честный seeded
//! Fisher-Yates по байтам (не хеш-перемешивание, как в ppmi_demo.rs),
//! воспроизводит методологию чертежа axmg ("S-static реальный текст vs
//! перетасованный контроль"), но измерено на СВОИХ данных, не процитирована
//! цифра axmg 0.1.0 (+78-107% на их корпусе).
//!
//! Методология: файл режется на train (обучаем Колесо) и holdout (не
//! видено Колесом, следующий кусок того же файла). S-static считается на
//! holdout как есть (реальный невиданный текст того же стиля/языка) и на
//! том же holdout, перетасованном по байтам (структура разрушена, частоты
//! байт те же). Разница — честный сигнал: ловит ли S-static структуру
//! языка, а не просто частоты отдельных байт.

use axmva_epoch::EpochState;
use axmva_surprise::s_static;
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;
const TRAIN_BYTES: usize = 1_000_000;
const HOLDOUT_BYTES: usize = 100_000;

/// xorshift64 — маленький seeded PRNG, без внешней зависимости на `rand`.
struct Xorshift64(u64);

impl Xorshift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Честный Fisher-Yates — каждая перестановка равновероятна, в отличие от
/// хеш-перемешивания в ppmi_demo.rs (см. его passport.yaml open_questions).
fn fisher_yates_shuffle(bytes: &[u8], seed: u64) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut rng = Xorshift64(seed);
    for i in (1..out.len()).rev() {
        let j = (rng.next() as usize) % (i + 1);
        out.swap(i, j);
    }
    out
}

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));
    assert!(
        bytes.len() >= TRAIN_BYTES + HOLDOUT_BYTES,
        "файл слишком короткий для train={TRAIN_BYTES} + holdout={HOLDOUT_BYTES}"
    );

    let train = &bytes[..TRAIN_BYTES];
    let holdout = &bytes[TRAIN_BYTES..TRAIN_BYTES + HOLDOUT_BYTES];

    println!("=== elements/surprise: количественный перетасованный контроль ===");
    println!("Файл: {path} ({} байт)", bytes.len());
    println!("train: первые {TRAIN_BYTES} байт | holdout (невиданный): следующие {HOLDOUT_BYTES} байт\n");

    let mut epoch = EpochState::new();
    epoch.run_corpus(train, BIRTH_THRESHOLD);
    println!("Полка после train: {} примитивов\n", epoch.store.len());

    let real_s_static = s_static(&epoch.store, holdout);

    let shuffled = fisher_yates_shuffle(holdout, 42);
    let shuffled_s_static = s_static(&epoch.store, &shuffled);

    println!("S-static на реальном невиданном holdout:      {real_s_static:.4}");
    println!("S-static на перетасованном том же holdout:    {shuffled_s_static:.4}");

    if shuffled_s_static > 0.0 {
        let pct = (real_s_static / shuffled_s_static - 1.0) * 100.0;
        println!("\nРазница: {pct:+.1}% (чертёж axmg 0.1.0 на своём корпусе сообщал +78..+107%)");
    }
}
