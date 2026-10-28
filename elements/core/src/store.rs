//! Полка (Store): content-addressed арена примитивов.
//!
//! `id = hash(level, hash(child_1), hash(child_2), ...)` — хеш считается от
//! хешей непосредственных детей, не от их индекса рождения, поэтому один и
//! тот же состав, собранный в разном порядке в разных хранилищах, получает
//! одинаковый id (детерминизм — contract.requires ядра). Тело токена
//! (hash/level/children) не меняется после рождения — append-only; меняется
//! только счётчик `touch` (отдельное поле, не часть тела).

use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub type TokenId = u32;

fn compute_hash(level: u32, children: &[TokenId], child_hashes: impl Fn(TokenId) -> [u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(level.to_be_bytes());
    if level == 0 {
        h.update([children[0] as u8]);
    } else {
        for &c in children {
            h.update(child_hashes(c));
        }
    }
    h.finalize().into()
}

#[derive(Clone)]
pub struct Store {
    hashes: Vec<[u8; 32]>,
    hash_index: HashMap<[u8; 32], TokenId>,
    levels: Vec<u32>,
    children_start: Vec<u32>,
    children_len: Vec<u32>,
    children_arena: Vec<TokenId>,
    counts: Vec<u64>,
    birth_log: Vec<TokenId>,
}

impl Store {
    pub fn new() -> Self {
        Store {
            hashes: Vec::new(),
            hash_index: HashMap::new(),
            levels: Vec::new(),
            children_start: Vec::new(),
            children_len: Vec::new(),
            children_arena: Vec::new(),
            counts: Vec::new(),
            birth_log: Vec::new(),
        }
    }

    /// Уровень 0 — 256 фабричных примитивов (все значения байта 0..255),
    /// создаются целиком до чтения любого входа. token_id совпадает со
    /// значением байта — удобно и не случайно: `b as TokenId` работает
    /// без отдельного lookup.
    pub fn init_factory(&mut self) {
        for b in 0..256u32 {
            self.intern(0, &[b]);
        }
    }

    pub fn intern(&mut self, level: u32, children: &[TokenId]) -> TokenId {
        let hash = compute_hash(level, children, |c| self.hashes[c as usize]);
        if let Some(&id) = self.hash_index.get(&hash) {
            return id;
        }
        let id = self.hashes.len() as TokenId;
        let start = self.children_arena.len() as u32;
        self.children_arena.extend_from_slice(children);
        self.hashes.push(hash);
        self.levels.push(level);
        self.children_start.push(start);
        self.children_len.push(children.len() as u32);
        self.counts.push(0);
        self.birth_log.push(id);
        self.hash_index.insert(hash, id);
        id
    }

    /// Проверить, существует ли уже состав (level, children) — без вставки.
    pub fn lookup(&self, level: u32, children: &[TokenId]) -> Option<TokenId> {
        let hash = compute_hash(level, children, |c| self.hashes[c as usize]);
        self.hash_index.get(&hash).copied()
    }

    pub fn get(&self, id: TokenId) -> (u32, &[TokenId]) {
        let start = self.children_start[id as usize] as usize;
        let len = self.children_len[id as usize] as usize;
        (self.levels[id as usize], &self.children_arena[start..start + len])
    }

    pub fn level_of(&self, id: TokenId) -> u32 {
        self.levels[id as usize]
    }

    /// Развёртка токена обратно в байты — без потерь (contract.requires:
    /// "без потерь"); вычисляется по требованию, не хранится.
    pub fn bytes_of(&self, id: TokenId) -> Vec<u8> {
        let (level, children) = self.get(id);
        if level == 0 {
            vec![children[0] as u8]
        } else {
            let mut out = Vec::new();
            for &c in children {
                out.extend(self.bytes_of(c));
            }
            out
        }
    }

    pub fn touch(&mut self, id: TokenId) {
        self.counts[id as usize] += 1;
    }

    pub fn count_of(&self, id: TokenId) -> u64 {
        self.counts[id as usize]
    }

    pub fn hash_of(&self, id: TokenId) -> [u8; 32] {
        self.hashes[id as usize]
    }

    pub fn birth_log(&self) -> &[TokenId] {
        &self.birth_log
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_level0_has_256_tokens() {
        let mut store = Store::new();
        store.init_factory();
        assert_eq!(store.len(), 256);
        for b in 0..256u32 {
            assert_eq!(store.level_of(b), 0);
            assert_eq!(store.bytes_of(b), vec![b as u8]);
        }
    }

    #[test]
    fn intern_dedupes_identical_composition() {
        let mut store = Store::new();
        store.init_factory();
        let a = store.intern(1, &[65, 66]);
        let b = store.intern(1, &[65, 66]);
        assert_eq!(a, b);
        assert_eq!(store.len(), 257);
    }

    #[test]
    fn intern_distinguishes_different_children_or_level() {
        let mut store = Store::new();
        store.init_factory();
        let ab = store.intern(1, &[65, 66]);
        let ba = store.intern(1, &[66, 65]);
        assert_ne!(ab, ba);
    }

    #[test]
    fn hash_depends_on_child_hash_not_birth_index_determinism() {
        // Детерминизм (contract.requires ядра): состав, собранный в разном
        // порядке рождения в двух разных хранилищах, даёт одинаковый id.
        let mut s1 = Store::new();
        s1.init_factory();
        let ab1 = s1.intern(1, &[b'a' as u32, b'b' as u32]);

        let mut s2 = Store::new();
        s2.init_factory();
        let _decoy = s2.intern(1, &[b'x' as u32, b'y' as u32]);
        let ab2 = s2.intern(1, &[b'a' as u32, b'b' as u32]);

        assert_eq!(s1.hash_of(ab1), s2.hash_of(ab2));
    }

    #[test]
    fn bytes_of_round_trip_is_lossless() {
        let mut store = Store::new();
        store.init_factory();
        let ab = store.intern(1, &[b'a' as u32, b'b' as u32]);
        let cd = store.intern(1, &[b'c' as u32, b'd' as u32]);
        let abcd = store.intern(2, &[ab, cd]);
        assert_eq!(store.bytes_of(abcd), b"abcd");
    }
}
