//! elements/core: ядро-архиватор — байты → примитивы (токены).
//!
//! Алгоритм (Store: content-addressed арена; merge: генерационное жадное
//! слияние частых пар) повторяет отлаженную реализацию axmg
//! (/home/assis/axmg/shelf/src/{store,merge}.rs) — перенесён и переписан
//! как своя реализация этого элемента, а не подключён как зависимость.
//! См. passport.yaml этого элемента и ../../drafts/axmv2-core-passport.md.

pub mod store;
pub mod merge;
pub mod recognizer;

pub use merge::{run, run_traced, Sequence};
pub use recognizer::StreamingRecognizer;
pub use store::{Store, TokenId};

/// Порция (оборот): столько байт берётся за один проход слияния.
/// Значение из ../../drafts/axmv2-deck-map.md — та же величина, что в axmg.
pub const REVOLUTION_CHUNK_BYTES: usize = 64 * 1024;
