//! elements/consolidate: Умеренность (карта 14 deck-map, код-имя
//! `consolidate` — drafts/axmv2-deck-map.md, строка 140).
//!
//! Перенос "горячих" счётчиков сессии (`Store::count_of`) в "холодный"
//! файл весов, отдельно от топологии, по пропорции α:
//! `cold = (1 − α)·cold + α·hot`. Экспоненциально взвешенное скользящее
//! среднее, не история: `elements/persist` намеренно НЕ персистирует
//! счётчики ("восстановимый вид, не история", см. его passport.yaml) —
//! этот элемент не отменяет то решение, а даёт ДРУГОЙ, более скромный
//! сигнал: сглаженный вес, переживающий перезапуски, не претендующий
//! быть точным счётом встреч.
//!
//! Повторяет предложение чертежа (deck-map, строка 31: "счётчики сессии
//! (горячее) -> файл весов рядом с топологией (холодное)"), в axmg как
//! реализация не существует (чертёж сам помечает это как "набросок,
//! принят как черновик", не как готовый код 0.1.0) — это не faithful-порт,
//! это первая реализация по спецификации черновика.

use axmva_core::{Store, TokenId};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::collections::HashMap;
use std::path::Path;

const WEIGHTS_TABLE: TableDefinition<u32, f64> = TableDefinition::new("cold_weights");

/// Ошибка этого элемента.
#[derive(Debug)]
pub enum ConsolidateError {
    /// α вне (0.0, 1.0] — вне этого диапазона перенос либо не делает
    /// ничего (α=0, cold никогда не меняется), либо отбрасывает всю
    /// память немедленно (α>1 бессмысленно, α<0 инвертирует вес).
    InvalidAlpha(f64),
    Redb(redb::Error),
}

impl std::fmt::Display for ConsolidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsolidateError::InvalidAlpha(a) => write!(f, "α={a} вне допустимого диапазона (0.0, 1.0]"),
            ConsolidateError::Redb(e) => write!(f, "redb: {e}"),
        }
    }
}

impl std::error::Error for ConsolidateError {}

fn redb_err<E: Into<redb::Error>>(e: E) -> ConsolidateError {
    ConsolidateError::Redb(e.into())
}

/// Холодные веса: token -> сглаженный сигнал, персистентный, но не
/// история. Пустой `ColdWeights` эквивалентен "никто ещё не consolidated".
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ColdWeights {
    weights: HashMap<TokenId, f64>,
}

impl ColdWeights {
    pub fn new() -> Self {
        Self::default()
    }

    /// Текущий холодный вес токена; 0.0, если ещё не consolidated.
    pub fn weight_of(&self, token: TokenId) -> f64 {
        self.weights.get(&token).copied().unwrap_or(0.0)
    }

    pub fn len(&self) -> usize {
        self.weights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    /// Один перенос: для каждого токена в `store` (0..store.len())
    /// `cold = (1-α)·cold + α·hot`, где hot = `store.count_of(token)`.
    /// Проходит ВСЮ полку каждый раз — простая, но не инкрементальная
    /// реализация v1 (см. passport.yaml, cost/open_questions).
    pub fn consolidate(&mut self, store: &Store, alpha: f64) -> Result<(), ConsolidateError> {
        if !(alpha > 0.0 && alpha <= 1.0) {
            return Err(ConsolidateError::InvalidAlpha(alpha));
        }
        for id in 0..store.len() as TokenId {
            let hot = store.count_of(id) as f64;
            let cold = self.weight_of(id);
            let updated = (1.0 - alpha) * cold + alpha * hot;
            self.weights.insert(id, updated);
        }
        Ok(())
    }

    /// Сохранить в отдельный файл — НЕ ту же базу, что топология
    /// (`elements/persist`), по прямому требованию черновика: "отдельный
    /// файл вне ядра". Снимок, не append-only лог: весь набор
    /// перезаписывается целиком (у сглаженного сигнала нет "префикса",
    /// который можно было бы дозаписывать).
    pub fn save(&self, path: &Path) -> Result<(), ConsolidateError> {
        let db = Database::create(path).map_err(redb_err)?;
        let write_txn = db.begin_write().map_err(redb_err)?;
        {
            let mut table = write_txn.open_table(WEIGHTS_TABLE).map_err(redb_err)?;
            for (&token, &weight) in &self.weights {
                table.insert(token, weight).map_err(redb_err)?;
            }
        }
        write_txn.commit().map_err(redb_err)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, ConsolidateError> {
        let db = Database::open(path).map_err(redb::Error::from).map_err(ConsolidateError::Redb)?;
        let read_txn = db.begin_read().map_err(redb::Error::from).map_err(ConsolidateError::Redb)?;
        let table = read_txn.open_table(WEIGHTS_TABLE).map_err(redb::Error::from).map_err(ConsolidateError::Redb)?;

        let mut weights = HashMap::new();
        for entry in table.iter().map_err(redb::Error::from).map_err(ConsolidateError::Redb)? {
            let (key, value) = entry.map_err(redb::Error::from).map_err(ConsolidateError::Redb)?;
            weights.insert(key.value(), value.value());
        }
        Ok(Self { weights })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_counts(counts: &[(u8, u64)]) -> Store {
        let mut store = Store::new();
        store.init_factory();
        for &(byte, count) in counts {
            for _ in 0..count {
                store.touch(byte as TokenId);
            }
        }
        store
    }

    #[test]
    fn consolidate_rejects_alpha_out_of_range() {
        let mut cold = ColdWeights::new();
        let store = store_with_counts(&[]);
        assert!(matches!(cold.consolidate(&store, 0.0), Err(ConsolidateError::InvalidAlpha(_))));
        assert!(matches!(cold.consolidate(&store, 1.5), Err(ConsolidateError::InvalidAlpha(_))));
        assert!(matches!(cold.consolidate(&store, -0.1), Err(ConsolidateError::InvalidAlpha(_))));
        assert!(cold.consolidate(&store, 1.0).is_ok());
    }

    #[test]
    fn first_consolidate_moves_cold_toward_hot_by_exactly_alpha() {
        let mut cold = ColdWeights::new();
        let store = store_with_counts(&[(b'a', 10)]);
        cold.consolidate(&store, 0.5).unwrap();
        // cold было 0.0 -> (1-0.5)*0.0 + 0.5*10 = 5.0
        assert_eq!(cold.weight_of(b'a' as TokenId), 5.0);
    }

    #[test]
    fn repeated_consolidate_with_constant_hot_converges_toward_hot() {
        let mut cold = ColdWeights::new();
        let store = store_with_counts(&[(b'a', 100)]);
        for _ in 0..50 {
            cold.consolidate(&store, 0.3).unwrap();
        }
        let w = cold.weight_of(b'a' as TokenId);
        // (1-0.3)^50 ~= 1.8e-8 остаточной доли исходного cold — не 0 точно,
        // но пренебрежимо мало; допуск шире одного float-ULP на 50 шагов.
        assert!((w - 100.0).abs() < 1e-4, "после многих переносов с постоянным hot cold обязан сойтись к hot, получили {w}");
    }

    #[test]
    fn alpha_one_makes_cold_track_hot_exactly_every_time_no_memory() {
        let mut cold = ColdWeights::new();
        let store_a = store_with_counts(&[(b'a', 10)]);
        cold.consolidate(&store_a, 1.0).unwrap();
        assert_eq!(cold.weight_of(b'a' as TokenId), 10.0);

        let store_b = store_with_counts(&[(b'a', 3)]);
        cold.consolidate(&store_b, 1.0).unwrap();
        assert_eq!(cold.weight_of(b'a' as TokenId), 3.0, "α=1.0 обязано отбросить всю память, cold := hot");
    }

    #[test]
    fn untouched_token_has_zero_weight_before_any_consolidate() {
        let cold = ColdWeights::new();
        assert_eq!(cold.weight_of(42), 0.0);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("axmva_consolidate_test_{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut cold = ColdWeights::new();
        let store = store_with_counts(&[(b'a', 10), (b'b', 20)]);
        cold.consolidate(&store, 0.5).unwrap();

        cold.save(&path).expect("save");
        let loaded = ColdWeights::load(&path).expect("load");
        assert_eq!(loaded, cold);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_is_a_full_snapshot_not_an_incremental_log() {
        // Если токен был consolidated раньше, но в текущей полке не
        // существует больше (гипотетически), снимок всё равно хранит
        // последний известный вес — save() не привязан к наличию токена
        // в какой-то конкретной Store на момент сохранения.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("axmva_consolidate_test_snapshot_{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut cold = ColdWeights::new();
        let store = store_with_counts(&[(b'a', 5)]);
        cold.consolidate(&store, 0.5).unwrap();
        cold.save(&path).expect("first save");

        // Повторный save с тем же состоянием — не ломается, перезаписывает.
        cold.save(&path).expect("second save");
        let loaded = ColdWeights::load(&path).expect("load");
        assert_eq!(loaded.weight_of(b'a' as TokenId), 2.5);

        let _ = std::fs::remove_file(&path);
    }
}
