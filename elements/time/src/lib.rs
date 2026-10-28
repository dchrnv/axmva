//! axtime: модуль времени — обобщённая библиотека, не привязанная к
//! токенам, Store или Merkle DAG. "Внутри модели времени нет, но оно не
//! может не быть" (../../drafts/axmv2-time-module.md) — поэтому нужны
//! разные виды времени, каждый со своим источником.
//!
//! Набор v1 из ../../drafts/axmv2-time-module.md (виды 1, 2, 3, 5; 4, 6, 7
//! остаются пометками):
//!
//! 1. **Порядковое** (Аристотель, B-ряд Мактаггарта) — номер тика,
//!    штамп на границе. [`stamp`][Time::stamp].
//! 2. **Относительное "сейчас"** (A-ряд Мактаггарта) — возраст = сейчас
//!    − last_touched, считается на лету, не хранится. [`age`][Time::age].
//! 3. **Внутреннее** (Бергсон, длительность) — возраст в накопленном
//!    росте, а не в тиках. [`age_inner`][Time::age_inner].
//! 5. **Циклическое** (Платон) — период появления id по тикам.
//!    [`period`][Time::period].
//!
//! Плюс модуль [`wall`] — настенное (календарное) время, стандартная пара
//! к тиковому: уровень 1 из другого черновика про уровни понимания
//! времени (`AXMV2/Уровни абстракции/Время.md`), "длительность в
//! абсолютных единицах". Независимая, опциональная метрика, не часть
//! `Time<Id>` — домен axmva её сегодня не использует, но библиотека
//! задумана как самостоятельная и полная, не только под один вызывающий
//! код (прямая команда пользователя: "если что-то не нужно сейчас, может
//! пригодится позже").
//!
//! Уровни 5-8 того же черновика (релятивистское, ветвящееся, петли,
//! метафизическое время) сознательно не реализованы — умозрительные
//! уровни для будущих архитектур ИИ в целом, не то, что выражается кодом
//! сейчас. Уровень 2 (стрела времени/причинность) не требует отдельного
//! API — уже гарантирован структурно: `stamp` только растёт, в этом
//! модуле нет метода, который мог бы его уменьшить.
//!
//! Кто-то снаружи (у нас — elements/epoch) должен вызывать [`Time::advance`]
//! раз за цикл и [`Time::touch`] на каждое реальное касание id — сама эта
//! библиотека не знает, что такое "оборот" или "токен".

pub mod wall;

use std::collections::HashMap;
use std::hash::Hash;

pub type Tick = u32;

/// Последний тик, на котором id был реально "тронут". Отдельно от
/// простого присутствия — вызывающий код решает, что считается touch.
#[derive(Default)]
pub struct TouchLog<Id> {
    last_touched: HashMap<Id, Tick>,
}

impl<Id: Eq + Hash + Copy> TouchLog<Id> {
    pub fn new() -> Self {
        Self { last_touched: HashMap::new() }
    }

    pub fn touch(&mut self, id: Id, now: Tick) {
        self.last_touched.insert(id, now);
    }

    pub fn last_touched(&self, id: Id) -> Option<Tick> {
        self.last_touched.get(&id).copied()
    }

    /// Возраст (вид 2) = `now - last_touched`. `None`, если id никогда не
    /// касались.
    pub fn age(&self, id: Id, now: Tick) -> Option<Tick> {
        self.last_touched(id).map(|last| now.saturating_sub(last))
    }
}

/// Накопленный ряд роста, один элемент на тик (вид 3: внутреннее время,
/// Бергсон — длительность измеряется накопленным ростом, не числом тиков).
#[derive(Default)]
pub struct GrowthSeries {
    inner: Vec<f64>,
}

impl GrowthSeries {
    pub fn new() -> Self {
        Self { inner: Vec::new() }
    }

    /// Дописать прирост за текущий тик — накапливается с предыдущим
    /// значением, не заменяет его.
    pub fn push(&mut self, delta: f64) {
        let last = self.inner.last().copied().unwrap_or(0.0);
        self.inner.push(last + delta);
    }

    /// Значение накопленного ряда на конец тика `tick`. `inner[0]` в
    /// хранилище соответствует тику 1 (первый `push`), поэтому тик 0
    /// (до какого-либо `advance`) — это база 0.0, не элемент массива.
    pub fn at(&self, tick: Tick) -> Option<f64> {
        if tick == 0 {
            return Some(0.0);
        }
        self.inner.get((tick - 1) as usize).copied()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// `inner[now] - inner[last_touched]` — внутренний возраст.
    pub fn age_inner(&self, now: Tick, last_touched: Tick) -> Option<f64> {
        let now_v = self.at(now)?;
        let then_v = self.at(last_touched)?;
        Some(now_v - then_v)
    }
}

/// История тиков, на которых id реально встречался — для периода (вид 5).
#[derive(Default)]
pub struct OccurrenceLog<Id> {
    history: HashMap<Id, Vec<Tick>>,
}

impl<Id: Eq + Hash + Copy> OccurrenceLog<Id> {
    pub fn new() -> Self {
        Self { history: HashMap::new() }
    }

    pub fn record(&mut self, id: Id, tick: Tick) {
        self.history.entry(id).or_default().push(tick);
    }

    pub fn occurrences(&self, id: Id) -> &[Tick] {
        self.history.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Период (вид 5) — средний промежуток между соседними встречами.
    /// `None`, если встреч меньше двух (периода не из чего вывести).
    pub fn period(&self, id: Id) -> Option<f64> {
        let h = self.history.get(&id)?;
        if h.len() < 2 {
            return None;
        }
        let gaps_sum: u32 = h.windows(2).map(|w| w[1] - w[0]).sum();
        Some(gaps_sum as f64 / (h.len() - 1) as f64)
    }
}

/// Фасад над тремя журналами — минимальный API из черновика
/// (`stamp`/`age`/`age_inner`/`period`), один тик = один вызов `advance`.
#[derive(Default)]
pub struct Time<Id> {
    now: Tick,
    touches: TouchLog<Id>,
    growth: GrowthSeries,
    occurrences: OccurrenceLog<Id>,
}

impl<Id: Eq + Hash + Copy> Time<Id> {
    pub fn new() -> Self {
        Self {
            now: 0,
            touches: TouchLog::new(),
            growth: GrowthSeries::new(),
            occurrences: OccurrenceLog::new(),
        }
    }

    /// Один тик вперёд. `growth_delta` — прирост (например, S-growth этого
    /// цикла), дописывается в накопленный ряд до любых `touch` этого тика.
    pub fn advance(&mut self, growth_delta: f64) {
        self.now += 1;
        self.growth.push(growth_delta);
    }

    /// Реальное касание id на текущем тике — обновляет и last_touched, и
    /// историю встреч одновременно (одно событие, не два независимых).
    pub fn touch(&mut self, id: Id) {
        self.touches.touch(id, self.now);
        self.occurrences.record(id, self.now);
    }

    /// Вид 1: порядковое время — номер текущего тика.
    pub fn stamp(&self) -> Tick {
        self.now
    }

    /// Вид 2: возраст в тиках с последнего касания.
    pub fn age(&self, id: Id) -> Option<Tick> {
        self.touches.age(id, self.now)
    }

    /// Вид 3: возраст в накопленном росте с последнего касания.
    pub fn age_inner(&self, id: Id) -> Option<f64> {
        let last = self.touches.last_touched(id)?;
        self.growth.age_inner(self.now, last)
    }

    /// Вид 5: период появления id по тикам.
    pub fn period(&self, id: Id) -> Option<f64> {
        self.occurrences.period(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_matches_number_of_advances() {
        let mut t: Time<u32> = Time::new();
        assert_eq!(t.stamp(), 0);
        t.advance(0.0);
        t.advance(0.0);
        assert_eq!(t.stamp(), 2);
    }

    #[test]
    fn age_is_zero_right_after_touch_and_grows_afterwards() {
        let mut t: Time<&str> = Time::new();
        t.advance(0.0);
        t.touch("a");
        assert_eq!(t.age("a"), Some(0));
        t.advance(0.0);
        t.advance(0.0);
        assert_eq!(t.age("a"), Some(2));
    }

    #[test]
    fn age_is_none_for_id_never_touched() {
        let t: Time<&str> = Time::new();
        assert_eq!(t.age("ghost"), None);
    }

    #[test]
    fn stale_id_sitting_untouched_does_not_get_age_reset_by_other_touches() {
        let mut t: Time<&str> = Time::new();
        t.advance(0.0);
        t.touch("a");
        t.advance(0.0);
        t.touch("b"); // трогаем другой id — "a" не должен обновиться
        assert_eq!(t.age("a"), Some(1), "касание чужого id не должно сбрасывать возраст 'a'");
    }

    #[test]
    fn age_inner_tracks_accumulated_growth_not_raw_ticks() {
        let mut t: Time<&str> = Time::new();
        t.advance(10.0); // tick 1, inner = 10
        t.touch("a");
        t.advance(1.0); // tick 2, inner = 11 (почти не растёт)
        t.advance(1.0); // tick 3, inner = 12
        // age по тикам = 2, но age_inner = 12 - 10 = 2.0 — здесь оба
        // совпадают численно, проверяем на примере с разным темпом роста:
        let mut t2: Time<&str> = Time::new();
        t2.advance(100.0); // tick 1, inner = 100
        t2.touch("a");
        t2.advance(0.1); // tick 2, inner = 100.1
        t2.advance(0.1); // tick 3, inner = 100.2
        assert_eq!(t2.age("a"), Some(2), "возраст в тиках — всегда целое число тиков");
        let inner = t2.age_inner("a").unwrap();
        assert!((inner - 0.2).abs() < 1e-9, "возраст во внутреннем времени — малый, несмотря на 2 тика (темп роста упал)");
    }

    #[test]
    fn period_is_none_with_fewer_than_two_occurrences() {
        let mut t: Time<&str> = Time::new();
        t.advance(0.0);
        t.touch("a");
        assert_eq!(t.period("a"), None);
    }

    #[test]
    fn period_is_average_gap_between_occurrences() {
        let mut t: Time<&str> = Time::new();
        for _ in 0..3 {
            t.advance(0.0);
        }
        t.touch("a"); // tick 3
        for _ in 0..3 {
            t.advance(0.0);
        }
        t.touch("a"); // tick 6, gap 3
        for _ in 0..5 {
            t.advance(0.0);
        }
        t.touch("a"); // tick 11, gap 5
        assert_eq!(t.period("a"), Some(4.0), "средний промежуток (3+5)/2 = 4.0");
    }
}
