//! `klipa-core` - domain + use cases. No I/O, no OS, no UI.
//!
//! Layers:
//! - [`domain`]: entities and ports (traits) the outer world implements
//! - [`usecases`]: application services orchestrating the ports
//!
//! Outer layers depend on this crate; this crate depends on nothing
//! platform-specific.

pub mod domain;
pub mod usecases;

pub use domain::entities::{HistoryItem, HistoryItemId, ItemContent, ItemKind};
pub use domain::error::CoreError;
pub use domain::ports::{ClipboardSource, HistoryStore, PasteboardEvent};
pub use usecases::history::HistoryService;
pub use usecases::search::{SearchMode, Searcher};
pub use usecases::sorter::{SortBy, Sorter};

/// Crate-wide Result alias.
pub type Result<T> = std::result::Result<T, CoreError>;
