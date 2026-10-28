//! elements/ppmi: перевёрнутая библиотека — со-встречаемость и значимость
//! связи (PPMI). Основа: `Window::range(2,5)` + `filter_by_min_level` +
//! `CooAccumulator` → `Csr` → `ppmi`/`valid_links`. Повторяет проверенную
//! axmg (context.rs, cooc.rs, ppmi.rs), переписано как своя реализация.
//! См. passport.yaml этого элемента.

pub mod context;
pub mod cooc;
pub mod ppmi;

pub use context::{filter_by_min_level, ContextExtractor, Window};
pub use cooc::{CooAccumulator, Csr};
pub use ppmi::{ppmi as score, valid_links, PpmiEntry, DEFAULT_FREQUENCY_FLOOR, DEFAULT_PPMI_THRESHOLD};
