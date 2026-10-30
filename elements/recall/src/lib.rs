//! elements/recall: ассоциативное вспоминание по Merkle DAG и PPMI-графу.
//!
//! Честный порт проверенной axmg (recall.rs), переписано как своя
//! реализация:
//! 1. Свёртка `query: &[u8]` через `StreamingRecognizer` в `query_tokens`.
//! 2. Поиск ассоциированных токенов через матрицу PPMI:
//!    `Neighbors(t) = { v | PPMI(t, v) >= threshold && v in focused }`.
//! 3. Ранжирование кандидатов по совместному весу:
//!    `Score(v) = sum_{t in query_tokens} PPMI(t, v) * Weight(v)`.
//! 4. Декодирование `bytes_of(v)` в текст.
//!
//! Это первый шаг "ассоциативного хвоста колоды" (карты 7-20, Колесница..
//! Суд, см. ../../drafts/axmv2-deck-map.md) — целиком один элемент, не
//! разбит на карты. Разбивка на Колесницу/Справедливость/Звезду/Луну/
//! Солнце/Суд — следующий шаг, когда появятся недостающие зависимости
//! (теги источников по токену, расширенная карта ожиданий Отшельника,
//! метки Дьявола/Башни, журнал лидеров для Суда) — см. open_questions в
//! passport.yaml.

use axmva_core::{Store, StreamingRecognizer, TokenId};
use axmva_ppmi::{filter_by_min_level, valid_links, ContextExtractor, CooAccumulator, Csr, Window};
use std::collections::{HashMap, HashSet};

/// `descendant` — потомок `ancestor` в графе композиции Store (включая
/// `ancestor == descendant`).
pub fn is_descendant(store: &Store, ancestor: TokenId, descendant: TokenId) -> bool {
    if ancestor == descendant {
        return true;
    }
    let (level, children) = store.get(ancestor);
    if level <= store.level_of(descendant) {
        return false;
    }
    for &child in children {
        if is_descendant(store, child, descendant) {
            return true;
        }
    }
    false
}

/// Конфигурация ассоциативного вспоминания.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallConfig {
    /// Максимальное число возвращаемых ассоциаций.
    pub limit: usize,
    /// Минимальная частота совместной встречаемости в окне.
    pub frequency_floor: u64,
    /// Минимальный порог PPMI в битах.
    pub ppmi_threshold: f64,
    /// Минимальный уровень композиции токенов (отсекает посимвольный шум).
    pub min_level: u32,
    /// Минимальная дистанция окна контекста.
    pub window_min: usize,
    /// Максимальная дистанция окна контекста.
    pub window_max: usize,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            limit: 5,
            frequency_floor: 1,
            ppmi_threshold: 0.1,
            min_level: 1,
            window_min: 1,
            window_max: 5,
        }
    }
}

/// Ассоциированный концепт.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallCandidate {
    pub token: TokenId,
    pub bytes: Vec<u8>,
    pub text: String,
    pub score: f64,
    pub ppmi: f64,
    pub raw_weight: u64,
    pub level: u32,
}

/// Результат вызова [`recall`].
#[derive(Debug, Clone, PartialEq)]
pub struct RecallResult {
    pub query_tokens: Vec<TokenId>,
    pub candidates: Vec<RecallCandidate>,
}

impl RecallResult {
    /// Компактное текстовое представление ассоциаций.
    pub fn to_context_string(&self) -> String {
        let texts: Vec<&str> = self
            .candidates
            .iter()
            .map(|c| c.text.trim())
            .filter(|s| !s.is_empty())
            .collect();
        texts.join(", ")
    }
}

/// Поиск ассоциаций по уже построенной матрице со-встречаемости CSR.
pub fn recall_from_csr(
    store: &Store,
    csr: &Csr,
    query_tokens: &[TokenId],
    focused: Option<&HashSet<TokenId>>,
    config: &RecallConfig,
) -> Vec<RecallCandidate> {
    if query_tokens.is_empty() {
        return Vec::new();
    }

    let valid = valid_links(csr, config.frequency_floor, config.ppmi_threshold);

    // кандидат -> (сумма PPMI, максимум PPMI, суммарный сырой вес)
    let mut candidate_stats: HashMap<TokenId, (f64, f64, u64)> = HashMap::new();

    // Расширяем запрос прямыми детьми составных токенов — иначе
    // ассоциации к частям фразы потерялись бы.
    let mut expanded_query = HashSet::new();
    for &q in query_tokens {
        expanded_query.insert(q);
        let (level, children) = store.get(q);
        if level > 0 {
            for &c in children {
                expanded_query.insert(c);
            }
        }
    }

    for &q in &expanded_query {
        for entry in &valid {
            let neighbor = if entry.token == q {
                Some(entry.context)
            } else if entry.context == q {
                Some(entry.token)
            } else {
                None
            };

            if let Some(cand) = neighbor {
                if expanded_query.contains(&cand) {
                    continue; // не возвращаем сам запрос как ассоциацию
                }
                if let Some(focus_set) = focused {
                    if !focus_set.contains(&cand) {
                        continue;
                    }
                }
                if store.level_of(cand) < config.min_level {
                    continue;
                }

                let stat = candidate_stats.entry(cand).or_insert((0.0, 0.0, 0));
                stat.0 += entry.ppmi;
                stat.1 = stat.1.max(entry.ppmi);
                stat.2 += entry.raw_weight;
            }
        }
    }

    let mut ranked: Vec<RecallCandidate> = candidate_stats
        .into_iter()
        .map(|(token, (sum_ppmi, max_ppmi, raw_weight))| {
            let level = store.level_of(token);
            let count = store.count_of(token).max(1);

            // Score(v) = sum PPMI(t, v) * Weight(v)
            let weight = (1.0 + (count as f64).log2()) * (1.0 + 0.5 * (level as f64));
            let score = sum_ppmi * weight;

            let bytes = store.bytes_of(token);
            let text = String::from_utf8_lossy(&bytes).to_string();

            RecallCandidate { token, bytes, text, score, ppmi: max_ppmi, raw_weight, level }
        })
        .collect();

    ranked.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    let mut unique_texts = HashSet::new();
    ranked.retain(|c| {
        let trimmed = c.text.trim();
        if trimmed.len() < 2 {
            return false;
        }
        unique_texts.insert(trimmed.to_string())
    });

    ranked.truncate(config.limit);
    ranked
}

/// Полный цикл ассоциативного вспоминания: запрос (байты) →
/// `StreamingRecognizer` → PPMI-граф по `sequence` → ранжирование →
/// декодирование.
pub fn recall(
    store: &Store,
    sequence: &[TokenId],
    query_bytes: &[u8],
    focused: Option<&HashSet<TokenId>>,
    config: &RecallConfig,
) -> RecallResult {
    let query_tokens = StreamingRecognizer::recognize(store, query_bytes);
    if query_tokens.is_empty() || sequence.is_empty() {
        return RecallResult { query_tokens, candidates: Vec::new() };
    }

    let window = Window::range(config.window_min, config.window_max);
    let events = window.extract(store, sequence);

    // min_level фильтрует события; при нехватке составных событий —
    // мягкий fallback к min_level = 0.
    let mut filtered = filter_by_min_level(store, events.clone(), config.min_level);
    let effective_config = if filtered.is_empty() && config.min_level > 0 {
        filtered = filter_by_min_level(store, events, 0);
        let mut c = config.clone();
        c.min_level = 0;
        c
    } else {
        config.clone()
    };

    let mut coo = CooAccumulator::new();
    coo.accumulate(&filtered);
    let csr = coo.bake_to_csr();

    // Токены sequence, которые содержат query_tokens как потомков (или
    // наоборот), тоже участвуют в поиске соседей.
    let mut target_query_tokens = query_tokens.clone();
    for &s in sequence {
        for &q in &query_tokens {
            if is_descendant(store, s, q) || is_descendant(store, q, s) {
                target_query_tokens.push(s);
            }
        }
    }
    target_query_tokens.sort_unstable();
    target_query_tokens.dedup();

    let mut candidates =
        recall_from_csr(store, &csr, &target_query_tokens, focused, &effective_config);

    // Фоллбэк: если PPMI-кандидатов нет, но есть составные токены
    // последовательности, содержащие запрос — вернуть их напрямую.
    if candidates.is_empty() {
        for &s in sequence {
            for &q in &query_tokens {
                if (is_descendant(store, s, q) || is_descendant(store, q, s)) && s != q {
                    let level = store.level_of(s);
                    if level >= effective_config.min_level {
                        let bytes = store.bytes_of(s);
                        let text = String::from_utf8_lossy(&bytes).to_string();
                        candidates.push(RecallCandidate {
                            token: s,
                            bytes,
                            text,
                            score: 1.0,
                            ppmi: 1.0,
                            raw_weight: store.count_of(s),
                            level,
                        });
                    }
                }
            }
        }
        candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        let mut unique_texts = HashSet::new();
        candidates.retain(|c| {
            let trimmed = c.text.trim();
            trimmed.len() >= 2 && unique_texts.insert(trimmed.to_string())
        });
        candidates.truncate(effective_config.limit);
    }

    RecallResult { query_tokens, candidates }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axmva_epoch::EpochState;

    const BIRTH_THRESHOLD: u64 = 2;

    #[test]
    fn recall_associates_related_words() {
        let mut epoch = EpochState::new();
        let corpus = b"the cat sat on the mat. the dog sat on the rug. the cat chased the dog. the dog barked at the cat.";
        epoch.turn(corpus, BIRTH_THRESHOLD);

        let config = RecallConfig::default();
        let result = recall(&epoch.store, &epoch.sequence.ids, b"cat", None, &config);
        assert!(!result.candidates.is_empty(), "recall должен найти ассоциации к 'cat'");
        assert!(!result.to_context_string().is_empty());
    }

    #[test]
    fn recall_empty_query_returns_empty() {
        let epoch = EpochState::new();
        let config = RecallConfig::default();
        let result = recall(&epoch.store, &epoch.sequence.ids, b"", None, &config);
        assert!(result.candidates.is_empty());
        assert_eq!(result.to_context_string(), "");
    }

    #[test]
    fn recall_empty_sequence_returns_empty_even_with_known_query() {
        let mut store = Store::new();
        store.init_factory();
        let config = RecallConfig::default();
        let result = recall(&store, &[], b"a", None, &config);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn recall_from_csr_respects_focused_filter() {
        // Проверяем сам PPMI-путь (recall_from_csr) напрямую, не полный
        // recall() — у полного recall() есть фоллбэк по is_descendant,
        // который focused не учитывает вовсе (см.
        // fallback_does_not_respect_focused_filter_same_as_axmg ниже).
        let mut epoch = EpochState::new();
        let corpus = b"the cat sat on the mat. the dog sat on the rug. the cat chased the dog. the dog barked at the cat.";
        epoch.turn(corpus, BIRTH_THRESHOLD);

        let mut config = RecallConfig::default();
        config.min_level = 0; // не зависеть здесь от фоллбэка recall() к min_level=0
        let window = Window::range(config.window_min, config.window_max);
        let events = window.extract(&epoch.store, &epoch.sequence.ids);
        let filtered = filter_by_min_level(&epoch.store, events, config.min_level);
        let mut coo = CooAccumulator::new();
        coo.accumulate(&filtered);
        let csr = coo.bake_to_csr();

        let query_tokens = StreamingRecognizer::recognize(&epoch.store, b"cat");
        let unrestricted = recall_from_csr(&epoch.store, &csr, &query_tokens, None, &config);
        assert!(!unrestricted.is_empty());

        let empty_focus: HashSet<TokenId> = HashSet::new();
        let restricted =
            recall_from_csr(&epoch.store, &csr, &query_tokens, Some(&empty_focus), &config);
        assert!(
            restricted.is_empty(),
            "пустой focused обязан отсечь все PPMI-кандидаты"
        );
    }

    #[test]
    fn fallback_does_not_respect_focused_filter_same_as_axmg() {
        // Задокументированное наследие axmg (не тихо исправлено здесь):
        // когда PPMI-граф пуст, recall() переходит в фоллбэк по
        // is_descendant, который НЕ проверяет focused вообще — короткий
        // текст ниже не набирает частотный пол для PPMI, поэтому
        // единственный источник кандидата — фоллбэк, и он срабатывает
        // даже с пустым focused. См. passport.yaml, open_questions.
        let mut epoch = EpochState::new();
        epoch.turn(b"ab ab", BIRTH_THRESHOLD);

        let mut config = RecallConfig::default();
        config.min_level = 0;
        let empty_focus: HashSet<TokenId> = HashSet::new();
        let result = recall(&epoch.store, &epoch.sequence.ids, b"a", Some(&empty_focus), &config);
        assert!(
            !result.candidates.is_empty(),
            "фоллбэк игнорирует focused — это унаследованное от axmg поведение, не новое"
        );
    }

    #[test]
    fn recall_never_returns_the_query_token_itself_as_a_candidate() {
        let mut epoch = EpochState::new();
        let corpus = b"the cat sat on the mat. the dog sat on the rug. the cat chased the dog. the dog barked at the cat.";
        epoch.turn(corpus, BIRTH_THRESHOLD);

        let config = RecallConfig::default();
        let result = recall(&epoch.store, &epoch.sequence.ids, b"cat", None, &config);
        let query_set: HashSet<TokenId> = result.query_tokens.iter().copied().collect();
        assert!(result.candidates.iter().all(|c| !query_set.contains(&c.token)));
    }

    #[test]
    fn recall_limit_truncates_candidates() {
        let mut epoch = EpochState::new();
        let corpus = b"the cat sat on the mat. the dog sat on the rug. the cat chased the dog. the dog barked at the cat. a bird flew over the cat and the dog and the mat.";
        epoch.turn(corpus, BIRTH_THRESHOLD);

        let mut config = RecallConfig::default();
        config.limit = 1;
        let result = recall(&epoch.store, &epoch.sequence.ids, b"cat", None, &config);
        assert!(result.candidates.len() <= 1);
    }

    #[test]
    fn is_descendant_true_for_self_and_real_ancestor_false_for_unrelated() {
        let mut store = Store::new();
        store.init_factory();
        let a = b'a' as TokenId;
        let b = b'b' as TokenId;
        let c = b'c' as TokenId;
        let ab = store.intern(1, &[a, b]);
        let abc = store.intern(2, &[ab, c]);

        assert!(is_descendant(&store, abc, abc), "токен — потомок самого себя");
        assert!(is_descendant(&store, abc, ab), "ab — прямой потомок abc");
        assert!(is_descendant(&store, abc, a), "a — транзитивный потомок abc");
        assert!(!is_descendant(&store, ab, abc), "abc не может быть потомком своего же ребёнка");
        assert!(!is_descendant(&store, a, b), "a и b — несвязанные листья");
    }

    #[test]
    fn recall_falls_back_to_descendant_match_when_ppmi_graph_has_no_valid_links() {
        // Короткий текст: PPMI-граф пуст (частотный пол не набирается), но
        // составной токен, содержащий запрос как потомка, существует —
        // фоллбэк должен его найти.
        let mut epoch = EpochState::new();
        epoch.turn(b"ab ab", BIRTH_THRESHOLD);

        let mut config = RecallConfig::default();
        config.min_level = 0;
        let result = recall(&epoch.store, &epoch.sequence.ids, b"a", None, &config);
        assert!(
            !result.candidates.is_empty(),
            "фоллбэк по is_descendant обязан найти составной токен, содержащий 'a'"
        );
    }
}
