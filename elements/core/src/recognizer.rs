//! StreamingRecognizer — потоковое распознавание существующими составами,
//! без права рождения (zero-mutation: только `Store::lookup`, никогда
//! `intern`). Нужен Surprise (S-static — "как хорошо уже известный словарь
//! узнаёт вход") и позже recall/контексту.
//!
//! Инварианты:
//! 1. Read-only: не создаёт новых токенов в Store.
//! 2. Побитовый детерминизм: результат не зависит от того, подаются байты
//!    по одному, произвольными чанками или всем срезом целиком.

use crate::store::{Store, TokenId};
use std::collections::HashSet;

pub struct StreamingRecognizer<'a> {
    store: &'a Store,
    stack: Vec<TokenId>,
    focused: Option<&'a HashSet<TokenId>>,
}

impl<'a> StreamingRecognizer<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store, stack: Vec::new(), focused: None }
    }

    /// Ограничить распознавание множеством `focused` (например, выходом
    /// elements/death) — составы вне фокуса недоступны для свёртки, даже
    /// если физически существуют в Store (тело не удаляется, доступность
    /// для свёртки — отдельное свойство).
    pub fn with_focus(store: &'a Store, focused: &'a HashSet<TokenId>) -> Self {
        Self { store, stack: Vec::new(), focused: Some(focused) }
    }

    pub fn feed_token(&mut self, token: TokenId) {
        self.stack.push(token);
        self.reduce();
    }

    pub fn feed_tokens(&mut self, tokens: &[TokenId]) {
        for &t in tokens {
            self.feed_token(t);
        }
    }

    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.feed_token(b as TokenId);
        }
    }

    /// Каскадная редукция вершины стека до сходимости влево.
    fn reduce(&mut self) {
        loop {
            let len = self.stack.len();
            if len < 2 {
                break;
            }
            let a = self.stack[len - 2];
            let b = self.stack[len - 1];
            let level = self.store.level_of(a).max(self.store.level_of(b)) + 1;
            if let Some(composite_id) = self.store.lookup(level, &[a, b]) {
                let usable = self.focused.is_none_or(|f| f.contains(&composite_id));
                if usable {
                    self.stack.pop();
                    self.stack.pop();
                    self.stack.push(composite_id);
                    continue;
                }
            }
            break;
        }
    }

    pub fn finish(self) -> Vec<TokenId> {
        self.stack
    }

    pub fn recognize(store: &'a Store, bytes: &[u8]) -> Vec<TokenId> {
        Self::recognize_focused(store, bytes, None)
    }

    pub fn recognize_focused(
        store: &'a Store,
        bytes: &[u8],
        focused: Option<&'a HashSet<TokenId>>,
    ) -> Vec<TokenId> {
        let mut recognizer = match focused {
            Some(f) => Self::with_focus(store, f),
            None => Self::new(store),
        };
        recognizer.feed_bytes(bytes);
        recognizer.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_store() -> (Store, TokenId) {
        let mut store = Store::new();
        store.init_factory();
        let a = b'a' as TokenId;
        let b = b'b' as TokenId;
        let c = b'c' as TokenId;
        let ab = store.intern(1, &[a, b]);
        let abc = store.intern(2, &[ab, c]);
        (store, abc)
    }

    #[test]
    fn test_streaming_matches_one_shot() {
        let (store, abc) = build_test_store();
        let one_shot = StreamingRecognizer::recognize(&store, b"abc");
        assert_eq!(one_shot, vec![abc]);

        let mut rec = StreamingRecognizer::new(&store);
        rec.feed_bytes(b"a");
        rec.feed_bytes(b"b");
        rec.feed_bytes(b"c");
        assert_eq!(rec.finish(), vec![abc]);
    }

    #[test]
    fn test_focused_filter_falls_back_when_composite_not_in_focus() {
        let (store, abc) = build_test_store();
        let ab = store.lookup(1, &[b'a' as u32, b'b' as u32]).unwrap();

        let mut focused = HashSet::new();
        focused.insert(ab);
        let tokens = StreamingRecognizer::recognize_focused(&store, b"abc", Some(&focused));
        assert_eq!(tokens, vec![ab, b'c' as TokenId], "composite вне фокуса недоступен для свёртки");

        focused.insert(abc);
        let tokens_full = StreamingRecognizer::recognize_focused(&store, b"abc", Some(&focused));
        assert_eq!(tokens_full, vec![abc]);
    }

    #[test]
    fn test_drain_and_continue() {
        let (store, _abc) = build_test_store();
        let ab = store.lookup(1, &[b'a' as u32, b'b' as u32]).unwrap();
        let mut rec = StreamingRecognizer::new(&store);
        rec.feed_bytes(b"ab");
        assert_eq!(std::mem::take(&mut rec.stack), vec![ab]);
    }
}
