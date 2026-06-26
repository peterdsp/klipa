use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Stable identity for a clipboard entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HistoryItemId(pub Uuid);

impl HistoryItemId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for HistoryItemId {
    fn default() -> Self {
        Self::new()
    }
}

/// The kind of content captured from the clipboard. Mirrors the
/// macOS app's distinction between text / rich text / image / files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Text,
    Html,
    Rtf,
    Image,
    Files,
}

/// One representation of an item (e.g. plain text + RTF for the same copy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemContent {
    pub kind: ItemKind,
    /// UTF-8 string for textual kinds; base64 for binary (image / files).
    pub value: String,
}

/// One entry in the clipboard history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub id: HistoryItemId,
    pub contents: Vec<ItemContent>,
    pub title: String,
    pub application: Option<String>,
    pub pin: Option<String>,
    pub number_of_copies: u32,
    pub first_copied_at: OffsetDateTime,
    pub last_copied_at: OffsetDateTime,
}

impl HistoryItem {
    pub fn new(contents: Vec<ItemContent>, application: Option<String>) -> Self {
        let now = OffsetDateTime::now_utc();
        let title = contents
            .iter()
            .find(|c| matches!(c.kind, ItemKind::Text))
            .map(|c| c.value.clone())
            .unwrap_or_else(|| match contents.first() {
                Some(c) => format!("[{:?}]", c.kind),
                None => String::new(),
            });
        Self {
            id: HistoryItemId::new(),
            contents,
            title,
            application,
            pin: None,
            number_of_copies: 1,
            first_copied_at: now,
            last_copied_at: now,
        }
    }

    pub fn is_pinned(&self) -> bool {
        self.pin.is_some()
    }

    /// True when `self` and `other` represent the same conceptual copy
    /// (same kinds + same plain-text value if textual).
    pub fn matches(&self, other: &HistoryItem) -> bool {
        if self.contents.len() != other.contents.len() {
            return false;
        }
        self.contents.iter().zip(other.contents.iter()).all(|(a, b)| {
            a.kind == b.kind && a.value == b.value
        })
    }
}
