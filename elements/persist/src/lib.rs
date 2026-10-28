//! elements/persist: персистентность на redb — тело Store и лог DeathLog.
//!
//! **Персистится только тело** (level + children, в порядке рождения) —
//! это и есть неизменная история. Счётчики (`counts`) — восстановимый вид,
//! не часть истории, и из одного тела не восстанавливаются; решение их не
//! персистить явное, не тихая экономия места.
//!
//! **Append-only.** `save` дописывает только то, что родилось после
//! последнего сохранения (длина таблицы == следующий id для записи).
//! Инкрементальный save корректен только потому, что Store append-only и
//! тела неизменны — это не оптимизация "про запас", это прямое следствие
//! инварианта ядра.
//!
//! **`load` — верификация лога, не просто десериализация.** Две проверки:
//! 1. Ссылки вперёд: запись уровня >=1 не может ссылаться на ребёнка, который
//!    ещё не встретился раньше в логе.
//! 2. Позиционность: i-я запись обязана воспроизвестись ровно в id `i`.
//! Оба случая — `LoadError`, не паника и не тихая рассинхронизация.
//!
//! Повторяет axmg (persist.rs), переписано как своя реализация. См.
//! passport.yaml этого элемента.

use axmva_core::{Store, TokenId};
use axmva_death::{DeathLog, FocusEvent};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::fmt;
use std::path::Path;

const BODY_TABLE: TableDefinition<u32, (u32, Vec<u32>)> = TableDefinition::new("body");
const DEATH_LOG_TABLE: TableDefinition<u32, (u8, u32, u32)> = TableDefinition::new("death_log");

#[derive(Debug)]
pub enum LoadError {
    Redb(redb::Error),
    /// i-я запись лога воспроизвелась не в id `i` — лог повреждён,
    /// переставлен или писался не через `save()`.
    CorruptLog { expected_id: TokenId, replayed_id: TokenId },
    /// Запись уровня >=1 ссылается на ребёнка, который ещё не появился в
    /// логе к этому моменту — композит построен из будущего.
    DanglingChild { record_id: TokenId, child: TokenId },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Redb(e) => write!(f, "redb: {e}"),
            LoadError::CorruptLog { expected_id, replayed_id } => {
                write!(f, "лог повреждён: запись #{expected_id} воспроизвелась в id {replayed_id}")
            }
            LoadError::DanglingChild { record_id, child } => {
                write!(f, "лог повреждён: запись #{record_id} ссылается на несуществующего ребёнка {child}")
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl From<redb::Error> for LoadError {
    fn from(e: redb::Error) -> Self {
        LoadError::Redb(e)
    }
}

fn encode_event(event: FocusEvent) -> (u8, u32, u32) {
    match event {
        FocusEvent::Focus { token, revolution } => (0, token, revolution),
        FocusEvent::Unfocus { token, revolution } => (1, token, revolution),
    }
}

fn decode_event(raw: (u8, u32, u32)) -> FocusEvent {
    let (kind, token, revolution) = raw;
    match kind {
        0 => FocusEvent::Focus { token, revolution },
        1 => FocusEvent::Unfocus { token, revolution },
        other => panic!("неизвестный тип записи лога фокуса: {other} (повреждённый файл?)"),
    }
}

/// Дописать в лог тела токенов, рождённые после последнего save. Создаёт
/// файл, если его ещё нет.
pub fn save(store: &Store, path: &Path) -> Result<(), redb::Error> {
    let db = Database::create(path)?;
    let write_txn = db.begin_write()?;
    {
        let mut table = write_txn.open_table(BODY_TABLE)?;
        let already_persisted = table.len()? as u32;
        for id in already_persisted..store.len() as TokenId {
            let (level, children) = store.get(id);
            table.insert(id, (level, children.to_vec()))?;
        }
    }
    write_txn.commit()?;
    Ok(())
}

/// Восстановить Store проходом по логу, заново вызывая `intern()` в
/// порядке рождения. Счётчики после загрузки — нулевые у всех токенов.
pub fn load(path: &Path) -> Result<Store, LoadError> {
    let db = Database::open(path).map_err(redb::Error::from)?;
    let read_txn = db.begin_read().map_err(redb::Error::from)?;
    let table = read_txn.open_table(BODY_TABLE).map_err(redb::Error::from)?;

    let mut store = Store::new();
    for entry in table.iter().map_err(redb::Error::from)? {
        let (key, value) = entry.map_err(redb::Error::from)?;
        let id = key.value();
        let (level, children) = value.value();

        if level >= 1 {
            if let Some(&bad_child) = children.iter().find(|&&c| c >= store.len() as TokenId) {
                return Err(LoadError::DanglingChild { record_id: id, child: bad_child });
            }
        }

        let replayed = store.intern(level, &children);
        if replayed != id {
            return Err(LoadError::CorruptLog { expected_id: id, replayed_id: replayed });
        }
    }
    Ok(store)
}

/// Атомарно сохраняет тело Store и события DeathLog в единой транзакции.
pub fn save_all(store: &Store, death_log: &DeathLog, path: &Path) -> Result<(), redb::Error> {
    let db = Database::create(path)?;
    let write_txn = db.begin_write()?;
    {
        let mut body_table = write_txn.open_table(BODY_TABLE)?;
        let already_persisted_tokens = body_table.len()? as u32;
        for id in already_persisted_tokens..store.len() as TokenId {
            let (level, children) = store.get(id);
            body_table.insert(id, (level, children.to_vec()))?;
        }

        let mut death_table = write_txn.open_table(DEATH_LOG_TABLE)?;
        let already_persisted_events = death_table.len()? as u32;
        for (i, &event) in death_log.events().iter().enumerate().skip(already_persisted_events as usize) {
            death_table.insert(i as u32, encode_event(event))?;
        }
    }
    write_txn.commit()?;
    Ok(())
}

/// Восстанавливает Store и DeathLog из redb. Если таблица лога отсутствует
/// (старая база), возвращает пустой DeathLog.
pub fn load_all(path: &Path) -> Result<(Store, DeathLog), LoadError> {
    let db = Database::open(path).map_err(redb::Error::from)?;
    let read_txn = db.begin_read().map_err(redb::Error::from)?;

    let body_table = read_txn.open_table(BODY_TABLE).map_err(redb::Error::from)?;
    let mut store = Store::new();
    for entry in body_table.iter().map_err(redb::Error::from)? {
        let (key, value) = entry.map_err(redb::Error::from)?;
        let id = key.value();
        let (level, children) = value.value();

        if level >= 1 {
            if let Some(&bad_child) = children.iter().find(|&&c| c >= store.len() as TokenId) {
                return Err(LoadError::DanglingChild { record_id: id, child: bad_child });
            }
        }

        let replayed = store.intern(level, &children);
        if replayed != id {
            return Err(LoadError::CorruptLog { expected_id: id, replayed_id: replayed });
        }
    }

    let death_log = match read_txn.open_table(DEATH_LOG_TABLE) {
        Ok(death_table) => {
            let mut events = Vec::new();
            for entry in death_table.iter().map_err(redb::Error::from)? {
                let (_key, value) = entry.map_err(redb::Error::from)?;
                events.push(decode_event(value.value()));
            }
            let focused = DeathLog::replay(&events);
            DeathLog::from_parts(events, focused)
        }
        Err(_) => DeathLog::new(),
    };

    Ok((store, death_log))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axmva_wheel::WheelState;

    fn tmp_db_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("axmva_persist_test_{}_{}.redb", name, std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn round_trip_preserves_body_and_ids_not_counters() {
        let path = tmp_db_path("round_trip");

        let mut store = Store::new();
        store.init_factory();
        let ab = store.intern(1, &[b'a' as u32, b'b' as u32]);
        let cd = store.intern(1, &[b'c' as u32, b'd' as u32]);
        let abcd = store.intern(2, &[ab, cd]);
        store.touch(abcd);

        save(&store, &path).expect("save");
        let loaded = load(&path).expect("load");

        assert_eq!(loaded.len(), store.len());
        for id in 0..store.len() as TokenId {
            assert_eq!(loaded.get(id), store.get(id), "тело токена id={id} разошлось");
            assert_eq!(loaded.hash_of(id), store.hash_of(id), "хеш id={id} разошёлся");
        }
        assert_eq!(loaded.bytes_of(abcd), b"abcd");
        assert_eq!(loaded.count_of(abcd), 0, "счётчики не персистятся — решение явное");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn incremental_save_matches_one_shot_save_bit_for_bit() {
        let incremental_path = tmp_db_path("incr_vs_oneshot__incr");
        let one_shot_path = tmp_db_path("incr_vs_oneshot__oneshot");

        let mut store = Store::new();
        store.init_factory();
        let ab = store.intern(1, &[b'a' as u32, b'b' as u32]);
        store.touch(ab);
        save(&store, &incremental_path).expect("первая дозапись");

        let cd = store.intern(1, &[b'c' as u32, b'd' as u32]);
        let abcd = store.intern(2, &[ab, cd]);
        store.touch(cd);
        store.touch(abcd);
        save(&store, &incremental_path).expect("вторая дозапись");

        save(&store, &one_shot_path).expect("контрольный один save на итоговом сторе");

        let loaded_incremental = load(&incremental_path).expect("load инкрементального");
        let loaded_one_shot = load(&one_shot_path).expect("load одноразового");

        assert_eq!(loaded_incremental.len(), loaded_one_shot.len());
        for id in 0..store.len() as TokenId {
            assert_eq!(loaded_incremental.get(id), loaded_one_shot.get(id));
            assert_eq!(loaded_incremental.hash_of(id), loaded_one_shot.hash_of(id));
        }

        let _ = std::fs::remove_file(&incremental_path);
        let _ = std::fs::remove_file(&one_shot_path);
    }

    #[test]
    fn load_reconstructs_dedup_index_not_just_bodies() {
        let path = tmp_db_path("dedup_after_load");

        let mut store = Store::new();
        store.init_factory();
        let ab = store.intern(1, &[b'a' as u32, b'b' as u32]);
        save(&store, &path).expect("save");

        let mut loaded = load(&path).expect("load");
        let ab_again = loaded.intern(1, &[b'a' as u32, b'b' as u32]);
        assert_eq!(ab_again, ab, "тот же состав после загрузки обязан дать тот же id");
        assert_eq!(loaded.len(), store.len(), "интерн уже существующего состава не должен расти");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_detects_corrupt_log_instead_of_silently_desyncing() {
        let path = tmp_db_path("corrupt_detection");
        let db = Database::create(&path).expect("create");
        {
            let write_txn = db.begin_write().expect("begin_write");
            {
                let mut table = write_txn.open_table(BODY_TABLE).expect("open_table");
                table.insert(0u32, (1u32, vec![5u32])).expect("insert corrupt row");
            }
            write_txn.commit().expect("commit");
        }
        drop(db);

        let result = load(&path);
        assert!(matches!(result, Err(_)), "повреждённый лог обязан вернуть Err, не тихо собранный стор");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_all_and_load_all_round_trip() {
        let path = tmp_db_path("save_all_round_trip");
        let mut wheel = WheelState::new();
        wheel.turn(b"hello world of axmva", 2);

        let mut death_log = DeathLog::new();
        death_log.mark(&wheel);
        death_log.weight_cut(&wheel);

        save_all(&wheel.store, &death_log, &path).expect("save_all");
        let (loaded_store, loaded_log) = load_all(&path).expect("load_all");

        assert_eq!(loaded_store.len(), wheel.store.len());
        assert_eq!(loaded_log.events().len(), death_log.events().len());
        assert_eq!(loaded_log.focused_set(), death_log.focused_set());

        let _ = std::fs::remove_file(&path);
    }
}
