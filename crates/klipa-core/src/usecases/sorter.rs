//! Sorter - port of `Sorter.swift`.

use crate::domain::entities::HistoryItem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    LastCopiedAt,
    FirstCopiedAt,
    NumberOfCopies,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinTo {
    Top,
    Bottom,
}

pub struct Sorter {
    pub sort_by: SortBy,
    pub pin_to: PinTo,
}

impl Default for Sorter {
    fn default() -> Self {
        Self {
            sort_by: SortBy::LastCopiedAt,
            pin_to: PinTo::Top,
        }
    }
}

impl Sorter {
    /// Sort `items` in place: pinned go first/last (per [`PinTo`]),
    /// the rest are ordered by [`SortBy`] descending.
    pub fn sort(&self, items: &mut [HistoryItem]) {
        items.sort_by(|a, b| {
            // Pinned bucket comes first or last depending on PinTo.
            let pin_cmp = match (a.is_pinned(), b.is_pinned()) {
                (true, false) => return match self.pin_to {
                    PinTo::Top => std::cmp::Ordering::Less,
                    PinTo::Bottom => std::cmp::Ordering::Greater,
                },
                (false, true) => return match self.pin_to {
                    PinTo::Top => std::cmp::Ordering::Greater,
                    PinTo::Bottom => std::cmp::Ordering::Less,
                },
                _ => std::cmp::Ordering::Equal,
            };
            if pin_cmp != std::cmp::Ordering::Equal {
                return pin_cmp;
            }
            // Within a bucket, order by the selected key, descending.
            match self.sort_by {
                SortBy::LastCopiedAt => b.last_copied_at.cmp(&a.last_copied_at),
                SortBy::FirstCopiedAt => b.first_copied_at.cmp(&a.first_copied_at),
                SortBy::NumberOfCopies => b.number_of_copies.cmp(&a.number_of_copies),
            }
        });
    }
}
