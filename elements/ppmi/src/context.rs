//! Экстракторы контекста для перевёрнутой библиотеки.
//!
//! Только `Window::range(2,5)` — принятая в axmg конфигурация (A4): дистанция
//! 1 — юрисдикция каскада слияния (он и так это съедает в том же поколении),
//! окно должно считать только то, что каскад принципиально не может съесть.
//! `Neighbor` и `Parent` сознательно не перенесены (решение A4 в axmg: Neighbor
//! закрыт структурно — почти ноль сигнала на финальной свёртке; Parent не
//! источник для перевёрнутой библиотеки — эти рёбра уже лежат прямо в Store).

use axmva_core::{Store, TokenId};

pub trait ContextExtractor {
    /// (token, context_token, weight).
    fn extract(&self, store: &Store, sequence: &[TokenId]) -> Vec<(TokenId, TokenId, u64)>;
}

/// Оставить только события, где оба токена имеют уровень >= `min_level` —
/// байтовый и первый уровень дают шум посимвольной статистики.
pub fn filter_by_min_level(
    store: &Store,
    events: Vec<(TokenId, TokenId, u64)>,
    min_level: u32,
) -> Vec<(TokenId, TokenId, u64)> {
    events
        .into_iter()
        .filter(|&(token, context, _)| {
            store.level_of(token) >= min_level && store.level_of(context) >= min_level
        })
        .collect()
}

/// Контекст = все токены на дистанции `min..=max` позиций (в обе стороны).
/// Симметрично.
pub struct Window {
    pub min: usize,
    pub max: usize,
}

impl Window {
    pub fn range(min: usize, max: usize) -> Self {
        assert!(min >= 1, "дистанция 0 — это сама позиция, не контекст");
        assert!(max >= min, "max обязан быть не меньше min");
        Window { min, max }
    }
}

impl ContextExtractor for Window {
    fn extract(&self, _store: &Store, sequence: &[TokenId]) -> Vec<(TokenId, TokenId, u64)> {
        let len = sequence.len();
        let mut out = Vec::new();
        for i in 0..len {
            let lo = i.saturating_sub(self.max);
            let hi = (i + self.max + 1).min(len);
            for j in lo..hi {
                if j == i {
                    continue;
                }
                let distance = i.abs_diff(j);
                if distance >= self.min && distance <= self.max {
                    out.push((sequence[i], sequence[j], 1));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_range_2_5_excludes_distance_1() {
        let store = Store::new();
        let seq: Vec<u32> = vec![10, 20, 30, 40, 50, 60, 70];
        let got = Window::range(2, 5).extract(&store, &seq);
        assert!(
            !got.contains(&(40, 30, 1)) && !got.contains(&(40, 50, 1)),
            "дистанция 1 обязана быть исключена из range(2,5)"
        );
        assert!(got.contains(&(40, 20, 1)));
        assert!(got.contains(&(40, 60, 1)));
        assert!(got.contains(&(40, 10, 1)));
        assert!(got.contains(&(40, 70, 1)));
    }

    #[test]
    fn window_range_bounds_are_inclusive() {
        let store = Store::new();
        let seq: Vec<u32> = (0..10).collect();
        let got = Window::range(2, 5).extract(&store, &seq);
        assert!(got.contains(&(5, 3, 1)));
        assert!(got.contains(&(5, 7, 1)));
        assert!(got.contains(&(5, 0, 1)));
        assert!(!got.contains(&(5, 4, 1)));
        assert!(!got.contains(&(5, 6, 1)));
    }

    #[test]
    fn filter_by_min_level_keeps_only_pairs_where_both_sides_qualify() {
        let mut store = Store::new();
        store.init_factory();
        let ab = store.intern(1, &[b'a' as u32, b'b' as u32]);
        let cd = store.intern(1, &[b'c' as u32, b'd' as u32]);
        let abcd = store.intern(2, &[ab, cd]);

        let events = vec![
            (abcd, abcd, 1),
            (abcd, ab, 1),
            (ab, cd, 1),
            (b'a' as u32, abcd, 1),
        ];
        let filtered = filter_by_min_level(&store, events, 2);
        assert_eq!(filtered, vec![(abcd, abcd, 1)]);
    }
}
