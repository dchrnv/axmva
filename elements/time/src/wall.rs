//! Настенное (календарное) время — стандартная пара к тиковому времени
//! остального модуля. По черновику "Время" (AXMV2, уровни понимания
//! времени 0-8): это уровень 1, "длительность в абсолютных единицах"
//! (секунды), в отличие от тикового счёта (уровень 0, "раньше/позже").
//!
//! Не заменяет тиковое время — у модели по-прежнему нет своих часов
//! внутри (../../drafts/axmv2-time-module.md), тики остаются основным
//! видом времени для домена axmva. Этот модуль — отдельная, опциональная
//! метрика для случаев, где реальное время всё же нужно (логи, таймауты,
//! корреляция с внешними событиями вне модели). Сознательно не реализованы
//! уровни 5-8 того же черновика (релятивистское, ветвящееся, петли,
//! метафизическое время) — это умозрительные уровни для будущих
//! архитектур ИИ в общем, не то, что можно написать реальным кодом сейчас;
//! см. passport.yaml, known_losses.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant, SystemTime};

/// Текущий момент как календарное время — для логов и корреляции с
/// внешним миром. Не использовать для измерения длительности (системные
/// часы могут скакнуть назад, например при синхронизации NTP) — для этого
/// `RealTouchLog`, который построен на `Instant` (монотонные часы).
pub fn now_wall_clock() -> SystemTime {
    SystemTime::now()
}

/// Журнал реального времени последнего касания — тот же контракт, что
/// `TouchLog` из тикового времени, но в `Instant`/`Duration`, не в тиках.
/// Основан на `Instant` (монотонные часы), не `SystemTime` — устойчив к
/// скачкам системных часов, подходит для измерения длительности.
#[derive(Default)]
pub struct RealTouchLog<Id> {
    last_touched: HashMap<Id, Instant>,
}

impl<Id: Eq + Hash + Copy> RealTouchLog<Id> {
    pub fn new() -> Self {
        Self { last_touched: HashMap::new() }
    }

    /// Отметить касание прямо сейчас (реальные часы).
    pub fn touch(&mut self, id: Id) {
        self.last_touched.insert(id, Instant::now());
    }

    /// Отметить касание в явно заданный момент — для воспроизводимых
    /// тестов и для случаев, когда момент касания известен не "прямо
    /// сейчас" (например, восстановлен из лога).
    pub fn touch_at(&mut self, id: Id, at: Instant) {
        self.last_touched.insert(id, at);
    }

    pub fn last_touched(&self, id: Id) -> Option<Instant> {
        self.last_touched.get(&id).copied()
    }

    /// Реальная длительность с последнего касания до текущего момента.
    /// `None`, если id никогда не касались.
    pub fn age(&self, id: Id) -> Option<Duration> {
        self.last_touched(id).map(|t| t.elapsed())
    }

    /// Как `age`, но относительно явно заданного "сейчас" — не зависит от
    /// реальных часов на момент вызова, для воспроизводимых тестов.
    pub fn age_at(&self, id: Id, now: Instant) -> Option<Duration> {
        self.last_touched(id).map(|t| now.saturating_duration_since(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_at_measures_explicit_duration_not_real_clock() {
        let t0 = Instant::now();
        let mut log: RealTouchLog<&str> = RealTouchLog::new();
        log.touch_at("a", t0);

        let five_seconds_later = t0 + Duration::from_secs(5);
        assert_eq!(log.age_at("a", five_seconds_later), Some(Duration::from_secs(5)));
    }

    #[test]
    fn age_is_none_for_id_never_touched() {
        let log: RealTouchLog<&str> = RealTouchLog::new();
        assert_eq!(log.age("ghost"), None);
    }

    #[test]
    fn touching_one_id_does_not_affect_another() {
        let t0 = Instant::now();
        let mut log: RealTouchLog<&str> = RealTouchLog::new();
        log.touch_at("a", t0);
        log.touch_at("b", t0 + Duration::from_secs(10));

        let now = t0 + Duration::from_secs(10);
        assert_eq!(log.age_at("a", now), Some(Duration::from_secs(10)), "касание 'b' не должно менять last_touched('a')");
        assert_eq!(log.age_at("b", now), Some(Duration::ZERO));
    }

    #[test]
    fn real_touch_and_live_age_agree_approximately() {
        // Проверка реальных часов (не детерминированная по значению, но
        // детерминированная по направлению и порядку величины) — именно
        // поэтому age_at с явным Instant выше покрывает основную логику,
        // а этот тест только подтверждает, что touch()/age() без явного
        // времени вообще работают на настоящих Instant::now().
        let mut log: RealTouchLog<&str> = RealTouchLog::new();
        log.touch("a");
        std::thread::sleep(Duration::from_millis(5));
        let age = log.age("a").expect("должно быть Some после touch");
        assert!(age >= Duration::from_millis(5), "возраст обязан быть не меньше реально проспанного времени");
        assert!(age < Duration::from_secs(5), "возраст не обязан взрываться на коротком сне");
    }
}
