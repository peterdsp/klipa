//! Search - ports of `Search.swift` semantics to Rust.

use crate::domain::entities::HistoryItem;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Exact,
    Fuzzy,
    Regex,
    Mixed,
}

#[derive(Debug, Clone)]
pub struct SearchResult<'a> {
    pub item: &'a HistoryItem,
    pub score: i64,
    pub ranges: Vec<(usize, usize)>,
}

pub struct Searcher {
    fuzzy: SkimMatcherV2,
}

impl Default for Searcher {
    fn default() -> Self {
        Self {
            fuzzy: SkimMatcherV2::default(),
        }
    }
}

impl Searcher {
    pub fn search<'a>(
        &self,
        query: &str,
        items: &'a [HistoryItem],
        mode: SearchMode,
    ) -> Vec<SearchResult<'a>> {
        if query.is_empty() {
            return items
                .iter()
                .map(|i| SearchResult {
                    item: i,
                    score: 0,
                    ranges: vec![],
                })
                .collect();
        }
        match mode {
            SearchMode::Exact => self.exact(query, items),
            SearchMode::Fuzzy => self.fuzzy(query, items),
            SearchMode::Regex => self.regex(query, items),
            SearchMode::Mixed => self.mixed(query, items),
        }
    }

    fn exact<'a>(&self, q: &str, items: &'a [HistoryItem]) -> Vec<SearchResult<'a>> {
        let q_lower = q.to_lowercase();
        items
            .iter()
            .filter_map(|i| {
                let title_lower = i.title.to_lowercase();
                title_lower.find(&q_lower).map(|start| SearchResult {
                    item: i,
                    score: 1,
                    ranges: vec![(start, start + q.len())],
                })
            })
            .collect()
    }

    fn fuzzy<'a>(&self, q: &str, items: &'a [HistoryItem]) -> Vec<SearchResult<'a>> {
        let mut out: Vec<_> = items
            .iter()
            .filter_map(|i| {
                self.fuzzy
                    .fuzzy_indices(&i.title, q)
                    .map(|(score, indices)| SearchResult {
                        item: i,
                        score,
                        ranges: contiguous_ranges(&indices),
                    })
            })
            .collect();
        out.sort_by(|a, b| b.score.cmp(&a.score));
        out
    }

    fn regex<'a>(&self, q: &str, items: &'a [HistoryItem]) -> Vec<SearchResult<'a>> {
        let Ok(re) = regex::Regex::new(q) else {
            return vec![];
        };
        items
            .iter()
            .filter_map(|i| {
                re.find(&i.title).map(|m| SearchResult {
                    item: i,
                    score: 1,
                    ranges: vec![(m.start(), m.end())],
                })
            })
            .collect()
    }

    fn mixed<'a>(&self, q: &str, items: &'a [HistoryItem]) -> Vec<SearchResult<'a>> {
        let mut out = self.exact(q, items);
        if out.is_empty() {
            out = self.fuzzy(q, items);
        }
        out
    }
}

/// Collapse a sorted list of character indices into (start, end_exclusive) ranges.
fn contiguous_ranges(indices: &[usize]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = vec![];
    for &i in indices {
        match out.last_mut() {
            Some(last) if last.1 == i => last.1 = i + 1,
            _ => out.push((i, i + 1)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{ItemContent, ItemKind};

    fn item(title: &str) -> HistoryItem {
        HistoryItem::new(
            vec![ItemContent {
                kind: ItemKind::Text,
                value: title.into(),
            }],
            None,
        )
    }

    #[test]
    fn exact_finds_substring() {
        let items = vec![item("hello world"), item("goodbye")];
        let r = Searcher::default().search("world", &items, SearchMode::Exact);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].ranges, vec![(6, 11)]);
    }

    #[test]
    fn empty_query_returns_all() {
        let items = vec![item("a"), item("b")];
        let r = Searcher::default().search("", &items, SearchMode::Fuzzy);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn regex_invalid_returns_empty() {
        let items = vec![item("a")];
        let r = Searcher::default().search("[", &items, SearchMode::Regex);
        assert!(r.is_empty());
    }
}
