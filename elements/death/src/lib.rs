//! elements/death: Смерть — generational GC без sweep.
//!
//! Смерть не мутирует тело токена в Store (оно неизменно, см. elements/core)
//! и не удаляет его — вместо этого append-only лог накапливает события
//! `Focus`/`Unfocus`. Воскрешение это просто более поздняя запись `Focus`,
//! не отдельный механизм.
//!
//! Корневое множество: фабричное дно (уровень 0, бессмертно — держит
//! идентичность байтов) + токены, входившие в свёртку потока за последние
//! `ROOT_WINDOW_REVOLUTIONS` оборотов Колеса. Достижимость — вниз по
//! составу (дети) от корней: узел жив, если достижим через живого родителя,
//! даже если сам давно не встречался напрямую.
//!
//! `weight_cut` — второй, отдельный проход: среди ПОГРАНИЧНЫХ (достижимых,
//! но не корней напрямую) квантиль 25% с наименьшим накопленным PPMI-весом
//! живых связей уходит в Unfocus. "Живая связь" — оба конца ещё достижимы;
//! связь с уже мёртвым не в счёт. При равном весе одноконтекстные (меньше
//! валидных связей) уходят раньше.
//!
//! Повторяет axmg (death.rs). Персистентность на диск по-прежнему не
//! перенесена — остаётся глубиной, см. passport.yaml, open_questions.

use axmva_core::{Store, TokenId};
use axmva_ppmi::{self as ppmi, filter_by_min_level, CooAccumulator, ContextExtractor, Window};
use axmva_wheel::WheelState;
use std::collections::{HashMap, HashSet};

/// K из axmg ("Корневое множество... K=3 оборота"), перенесено как есть —
/// решение не пересматривалось, только перенесено (см. open_questions).
pub const ROOT_WINDOW_REVOLUTIONS: u32 = 3;
/// Правка 6 (axmg): доля пограничных, срезаемых по весу за один проход.
pub const WEIGHT_CUT_QUANTILE: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusEvent {
    Focus { token: TokenId, revolution: u32 },
    Unfocus { token: TokenId, revolution: u32 },
}

/// Три естественных исхода одного вызова `DeathLog::mark` — прямое
/// следствие сравнения "было ли достижимо / стало ли достижимо", не
/// отдельная механика.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MarkOutcome {
    pub newly_focused: usize,
    pub newly_unfocused: usize,
    pub unchanged: usize,
}

/// Результат одного вызова `DeathLog::weight_cut`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WeightCutOutcome {
    pub borderline_count: usize,
    pub cut_count: usize,
}

/// Корневое множество на текущий момент Колеса.
pub fn root_set(wheel: &WheelState) -> HashSet<TokenId> {
    let mut roots = HashSet::with_capacity(256 + wheel.last_touched_revolution.len());
    let factory_count = 256.min(wheel.store.len() as TokenId);
    for id in 0..factory_count {
        if wheel.store.level_of(id) == 0 {
            roots.insert(id);
        }
    }
    for (&id, &last) in &wheel.last_touched_revolution {
        if wheel.revolution.saturating_sub(last) < ROOT_WINDOW_REVOLUTIONS {
            roots.insert(id);
        }
    }
    roots
}

/// Достижимость вниз по составу от корней (дети, внуки, ... до байтов).
pub fn reachable_set(store: &Store, roots: &HashSet<TokenId>) -> HashSet<TokenId> {
    let mut reachable: HashSet<TokenId> = HashSet::new();
    let mut stack: Vec<TokenId> = roots.iter().copied().collect();
    while let Some(id) = stack.pop() {
        if !reachable.insert(id) {
            continue;
        }
        let (level, children) = store.get(id);
        if level > 0 {
            for &c in children {
                if !reachable.contains(&c) {
                    stack.push(c);
                }
            }
        }
    }
    reachable
}

/// Append-only лог событий фокуса + текущий восстановимый вид (какие
/// токены сейчас достижимы) — `events` это история, `focused` это вид,
/// выводимый из истории проходом (`replay`), тот же разрез, что у Store.
#[derive(Default)]
pub struct DeathLog {
    events: Vec<FocusEvent>,
    focused: HashSet<TokenId>,
}

impl DeathLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Собрать из уже готовых частей (например, после восстановления с
    /// диска — см. elements/persist) без повторного прохода mark/replay.
    pub fn from_parts(events: Vec<FocusEvent>, focused: HashSet<TokenId>) -> Self {
        Self { events, focused }
    }

    /// Пересчитать достижимость для текущего состояния Колеса и записать
    /// переходы как события. Порядок обхода — возрастающий TokenId
    /// (детерминизм): порядок событий одного mark() воспроизводим
    /// независимо от порядка обхода HashSet.
    pub fn mark(&mut self, wheel: &WheelState) -> MarkOutcome {
        let roots = root_set(wheel);
        let reachable = reachable_set(&wheel.store, &roots);
        let mut outcome = MarkOutcome::default();

        let mut newly_focused: Vec<TokenId> = reachable
            .iter()
            .copied()
            .filter(|id| !self.focused.contains(id))
            .collect();
        let mut newly_unfocused: Vec<TokenId> = self
            .focused
            .iter()
            .copied()
            .filter(|id| !reachable.contains(id))
            .collect();

        outcome.newly_focused = newly_focused.len();
        outcome.newly_unfocused = newly_unfocused.len();
        outcome.unchanged = wheel
            .store
            .len()
            .saturating_sub(outcome.newly_focused + outcome.newly_unfocused);

        newly_focused.sort_unstable();
        newly_unfocused.sort_unstable();

        let mut i = 0;
        let mut j = 0;
        while i < newly_focused.len() && j < newly_unfocused.len() {
            if newly_focused[i] < newly_unfocused[j] {
                let id = newly_focused[i];
                self.events.push(FocusEvent::Focus { token: id, revolution: wheel.revolution });
                self.focused.insert(id);
                i += 1;
            } else {
                let id = newly_unfocused[j];
                self.events.push(FocusEvent::Unfocus { token: id, revolution: wheel.revolution });
                self.focused.remove(&id);
                j += 1;
            }
        }
        while i < newly_focused.len() {
            let id = newly_focused[i];
            self.events.push(FocusEvent::Focus { token: id, revolution: wheel.revolution });
            self.focused.insert(id);
            i += 1;
        }
        while j < newly_unfocused.len() {
            let id = newly_unfocused[j];
            self.events.push(FocusEvent::Unfocus { token: id, revolution: wheel.revolution });
            self.focused.remove(&id);
            j += 1;
        }

        outcome
    }

    /// Срез по весу (Правка 6) среди ПОГРАНИЧНЫХ (достижимых, но не корней
    /// напрямую) — квантиль 25% с наименьшим накопленным PPMI-весом живых
    /// связей уходит в Unfocus. Тай-брейк при равном весе — одноконтекстные
    /// (меньше валидных связей) раньше в очереди на срез. Требует, чтобы
    /// `mark()` уже был вызван для этого состояния Колеса — срез работает
    /// над уже размеченным достижимым множеством, не пересчитывает его.
    pub fn weight_cut(&mut self, wheel: &WheelState) -> WeightCutOutcome {
        let roots = root_set(wheel);
        let reachable = reachable_set(&wheel.store, &roots);
        let borderline: Vec<TokenId> = reachable.iter().copied().filter(|id| !roots.contains(id)).collect();

        // Вес = накопленный PPMI-вес живых связей (перевёрнутая полка,
        // окно 2..5, тот же модуль PPMI, что судит поток в Справедливости/Звезде).
        let window = Window::range(2, 5);
        let events = window.extract(&wheel.store, &wheel.sequence.ids);
        let filtered = filter_by_min_level(&wheel.store, events, 2);
        let mut coo = CooAccumulator::new();
        coo.accumulate(&filtered);
        let csr = coo.bake_to_csr();
        let valid = ppmi::valid_links(&csr, ppmi::DEFAULT_FREQUENCY_FLOOR, ppmi::DEFAULT_PPMI_THRESHOLD);

        let mut weight: HashMap<TokenId, f64> = HashMap::new();
        let mut diversity: HashMap<TokenId, usize> = HashMap::new();
        for entry in &valid {
            // "живая связь" — оба конца ещё достижимы; связь с уже мёртвым не в счёт.
            if reachable.contains(&entry.token) && reachable.contains(&entry.context) {
                *weight.entry(entry.token).or_insert(0.0) += entry.ppmi;
                *diversity.entry(entry.token).or_insert(0) += 1;
            }
        }

        let mut scored: Vec<(TokenId, f64, usize)> = borderline
            .iter()
            .map(|&id| (id, weight.get(&id).copied().unwrap_or(0.0), diversity.get(&id).copied().unwrap_or(0)))
            .collect();
        // Возрастание по весу; при равном весе одноконтекстные (меньшее
        // разнообразие) раньше; TokenId последним членом — детерминизм.
        scored.sort_by(|a, b| {
            a.1.partial_cmp(&b.1).unwrap().then_with(|| a.2.cmp(&b.2)).then_with(|| a.0.cmp(&b.0))
        });

        let cut_count = ((scored.len() as f64) * WEIGHT_CUT_QUANTILE).round() as usize;
        let mut newly_unfocused = 0usize;
        for &(id, _, _) in scored.iter().take(cut_count) {
            if self.focused.contains(&id) {
                self.events.push(FocusEvent::Unfocus { token: id, revolution: wheel.revolution });
                self.focused.remove(&id);
                newly_unfocused += 1;
            }
        }

        WeightCutOutcome { borderline_count: borderline.len(), cut_count: newly_unfocused }
    }

    pub fn events(&self) -> &[FocusEvent] {
        &self.events
    }

    pub fn is_focused(&self, token: TokenId) -> bool {
        self.focused.contains(&token)
    }

    pub fn focused_set(&self) -> &HashSet<TokenId> {
        &self.focused
    }

    /// Восстановить состояние фокуса проходом по логу — вид выводится из
    /// истории, не хранится отдельно от неё как источник истины.
    pub fn replay(events: &[FocusEvent]) -> HashSet<TokenId> {
        let mut focused = HashSet::new();
        for &event in events {
            match event {
                FocusEvent::Focus { token, .. } => {
                    focused.insert(token);
                }
                FocusEvent::Unfocus { token, .. } => {
                    focused.remove(&token);
                }
            }
        }
        focused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_tokens_are_always_roots_and_reachable() {
        let wheel = WheelState::new();
        let roots = root_set(&wheel);
        for b in 0..256u32 {
            assert!(roots.contains(&b), "байт {b} обязан быть корнем — фабричное дно бессмертно");
        }
    }

    #[test]
    fn token_outside_root_window_is_unfocused_but_reachable_child_stays_focused() {
        let mut wheel = WheelState::new();
        wheel.turn(b"ab", 2);
        let a = wheel.store.intern(0, &[b'a' as u32]);
        let b = wheel.store.intern(0, &[b'b' as u32]);
        let ab = wheel.store.intern(1, &[a, b]);
        for _ in 0..ROOT_WINDOW_REVOLUTIONS {
            wheel.turn(b"xyz", 2);
        }

        let mut log = DeathLog::new();
        log.mark(&wheel);

        assert!(!log.is_focused(ab), "'ab' обязан выйти из фокуса");
        assert!(log.is_focused(a));
        assert!(log.is_focused(b));
    }

    #[test]
    fn resurrection_is_a_later_focus_event_not_a_special_mechanism() {
        let mut wheel = WheelState::new();
        wheel.turn(b"ab ab", 2);
        let mut log = DeathLog::new();
        log.mark(&wheel);

        let ab = wheel
            .sequence
            .ids
            .iter()
            .copied()
            .find(|&id| wheel.store.bytes_of(id) == b"ab")
            .expect("'ab' должен был слиться в композит");
        assert!(log.is_focused(ab));

        for _ in 0..ROOT_WINDOW_REVOLUTIONS {
            wheel.turn(b"zzz", 2);
        }
        log.mark(&wheel);
        assert!(!log.is_focused(ab), "должен выйти из фокуса без новых встреч");

        wheel.turn(b"ab ab", 2);
        log.mark(&wheel);
        assert!(log.is_focused(ab), "новая встреча обязана вернуть в фокус — Focus, не отдельный механизм");

        let transitions: Vec<FocusEvent> = log
            .events()
            .iter()
            .copied()
            .filter(|e| matches!(e, FocusEvent::Focus { token, .. } | FocusEvent::Unfocus { token, .. } if *token == ab))
            .collect();
        assert!(transitions.len() >= 3, "минимум Focus->Unfocus->Focus на этот токен, получили {transitions:?}");
    }

    #[test]
    fn mark_outcome_reports_three_natural_results() {
        let mut wheel = WheelState::new();
        wheel.turn(b"hello world", 2);
        let mut log = DeathLog::new();
        let first = log.mark(&wheel);
        assert_eq!(first.unchanged, 0, "первый mark -> всё либо новое, либо нет");
        assert!(first.newly_focused > 0);

        let second = log.mark(&wheel);
        assert_eq!(second.newly_focused, 0);
        assert_eq!(second.newly_unfocused, 0);
        assert!(second.unchanged > 0, "повторный mark без изменений Колеса обязан дать unchanged");
    }

    #[test]
    fn replay_reconstructs_focused_set_bit_for_bit() {
        let mut wheel = WheelState::new();
        wheel.turn(b"the cat sat on the mat", 2);
        let mut log = DeathLog::new();
        log.mark(&wheel);
        for _ in 0..ROOT_WINDOW_REVOLUTIONS + 1 {
            wheel.turn(b"zzz zzz zzz", 2);
        }
        log.mark(&wheel);

        let replayed = DeathLog::replay(log.events());
        assert_eq!(replayed, *log.focused_set(), "replay обязан дать то же множество, что накопленное состояние");
    }

    #[test]
    fn weight_cut_removes_roughly_a_quarter_of_borderline_tokens() {
        let mut wheel = WheelState::new();
        let text = b"the cat sat on the mat, the cat sat on the mat, the cat ran to the mat, \
                     the dog sat on the log, the dog sat on the log, the dog ran to the log, \
                     a bird flew over the tree, a bird flew over the tree, a bird sang in the tree";
        wheel.turn(text, 2);
        let mut log = DeathLog::new();
        log.mark(&wheel);

        let outcome = log.weight_cut(&wheel);
        assert!(outcome.borderline_count > 0, "должны быть пограничные (достижимые не-корни) на таком тексте");
        let expected = ((outcome.borderline_count as f64) * WEIGHT_CUT_QUANTILE).round() as usize;
        assert_eq!(outcome.cut_count, expected, "срез обязан снять ровно квантиль 25% (с округлением)");
    }

    #[test]
    fn weight_cut_only_acts_on_borderline_not_on_roots() {
        let mut wheel = WheelState::new();
        wheel.turn(b"ab ab cd cd ab cd ef ef gh gh ef gh", 2);
        let mut log = DeathLog::new();
        log.mark(&wheel);
        let roots_before = root_set(&wheel);

        log.weight_cut(&wheel);

        for &root in &roots_before {
            assert!(log.is_focused(root), "срез по весу не имеет права снять корень");
        }
    }
}
