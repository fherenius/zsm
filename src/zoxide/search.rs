use crate::session::SessionItem;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use zsm::list;

/// Search result containing an item and match information
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The matched item
    pub item: SessionItem,
    /// Fuzzy match score
    pub score: i64,
    /// Character indices that matched the search term
    pub indices: Vec<usize>,
}

/// Handles fuzzy searching across sessions and directories
pub struct SearchEngine {
    /// Current search term
    search_term: String,
    /// Fuzzy matcher instance
    matcher: SkimMatcherV2,
    /// Current search results
    results: Vec<SearchResult>,
    /// Selected result index
    selected_index: Option<usize>,
    /// Whether we're currently searching
    is_searching: bool,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self {
            search_term: String::new(),
            matcher: SkimMatcherV2::default().use_cache(true),
            results: Vec::new(),
            selected_index: None,
            is_searching: false,
        }
    }
}

impl SearchEngine {
    /// Update search term and perform search
    pub fn update_search(&mut self, term: String, items: &[SessionItem]) {
        let selected = if term == self.search_term {
            self.selected_item().cloned()
        } else {
            self.selected_index = None;
            None
        };
        self.search_term = term;
        self.is_searching = !self.search_term.is_empty();

        if self.is_searching {
            self.perform_search(items);
            if let Some(selected) = selected {
                if let Some(index) = self
                    .results
                    .iter()
                    .position(|result| result.item.same_identity(&selected))
                {
                    self.selected_index = Some(index);
                }
            }
        } else {
            self.results.clear();
            self.selected_index = None;
        }
    }

    /// Add character to search term
    pub fn add_char(&mut self, c: char, items: &[SessionItem]) {
        let mut term = self.search_term.clone();
        term.push(c);
        self.update_search(term, items);
    }

    /// Remove last character from search term
    pub fn backspace(&mut self, items: &[SessionItem]) {
        let mut term = self.search_term.clone();
        term.pop();
        self.update_search(term, items);
    }

    /// Clear search term
    pub fn clear(&mut self) {
        self.search_term.clear();
        self.results.clear();
        self.selected_index = None;
        self.is_searching = false;
    }

    /// Get current search term
    pub fn search_term(&self) -> &str {
        &self.search_term
    }

    /// Check if currently searching
    pub fn is_searching(&self) -> bool {
        self.is_searching
    }

    /// Get search results
    pub fn results(&self) -> &[SearchResult] {
        &self.results
    }

    /// Get selected index
    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    /// Move selection up
    pub fn move_selection_up(&mut self) {
        self.selected_index = list::select_previous(self.selected_index, self.results.len());
    }

    /// Move selection down
    pub fn move_selection_down(&mut self) {
        self.selected_index = list::select_next(self.selected_index, self.results.len());
    }

    /// Get currently selected item
    pub fn selected_item(&self) -> Option<&SessionItem> {
        self.selected_index
            .and_then(|i| self.results.get(i))
            .map(|result| &result.item)
    }

    /// Perform fuzzy search on items
    fn perform_search(&mut self, items: &[SessionItem]) {
        let mut matches = Vec::new();

        for item in items {
            // Match against the same text the renderer draws, so the returned
            // indices can be mapped straight onto the rendered row.
            let display_text = item.display_text();

            if let Some((score, indices)) =
                self.matcher.fuzzy_indices(&display_text, &self.search_term)
            {
                matches.push(SearchResult {
                    item: item.clone(),
                    score,
                    indices,
                });
            }
        }

        // Keep matching sessions in last-used order; rank directories by score.
        matches.sort_by(|a, b| {
            let a_is_session = a.item.is_session() || a.item.is_resurrectable_session();
            let b_is_session = b.item.is_session() || b.item.is_resurrectable_session();

            match (a_is_session, b_is_session) {
                (true, false) => std::cmp::Ordering::Less, // a (session) comes first
                (false, true) => std::cmp::Ordering::Greater, // b (session) comes first
                (true, true) => std::cmp::Ordering::Equal, // Stable sort retains recency
                _ => b
                    .item
                    .is_pinned()
                    .cmp(&a.item.is_pinned())
                    .then_with(|| b.score.cmp(&a.score)), // Same type, sort by score
            }
        });

        self.results = matches;

        // Update selected index
        if self.results.is_empty() {
            self.selected_index = None;
        } else {
            match self.selected_index {
                Some(idx) if idx >= self.results.len() => {
                    self.selected_index = Some(self.results.len().saturating_sub(1));
                }
                None => {
                    self.selected_index = Some(0);
                }
                _ => {} // Keep current selection if valid
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str) -> SessionItem {
        SessionItem::ExistingSession {
            name: name.into(),
            directory: String::new(),
            is_current: false,
        }
    }

    #[test]
    fn pins_sort_before_other_matching_directories_but_after_sessions() {
        let mut search = SearchEngine::default();
        search.update_search(
            "abc".into(),
            &[
                SessionItem::Directory {
                    path: "abc".into(),
                    session_name: "abc".into(),
                    pinned: false,
                },
                SessionItem::Directory {
                    path: "/a/long/b/path/c".into(),
                    session_name: "c".into(),
                    pinned: true,
                },
                session("abc"),
            ],
        );
        assert!(search.results()[0].item.is_session());
        assert!(search.results()[1].item.is_pinned());
        assert!(!search.results()[2].item.is_pinned());
    }

    #[test]
    fn matching_sessions_keep_recency_order_even_with_different_scores() {
        let mut search = SearchEngine::default();
        search.update_search(
            "abc".into(),
            &[
                session("a-long-b-long-c"),
                session("abc"),
                SessionItem::Directory {
                    pinned: false,
                    path: "abc".into(),
                    session_name: "abc".into(),
                },
            ],
        );
        assert_eq!(search.results().len(), 3);
        assert_eq!(
            search.results()[0].item.session_name(),
            Some("a-long-b-long-c")
        );
        assert_eq!(search.results()[1].item.session_name(), Some("abc"));
        assert!(search.results()[2].item.session_name().is_none());
    }

    #[test]
    fn refresh_preserves_selection_and_a_new_query_selects_the_first_match() {
        let mut search = SearchEngine::default();
        search.update_search("a".into(), &[session("alpha"), session("beta")]);
        search.move_selection_down();
        search.update_search("a".into(), &[session("beta"), session("alpha")]);
        assert_eq!(search.selected_item().unwrap().session_name(), Some("beta"));
        search.add_char('l', &[session("beta"), session("alpha")]);
        assert_eq!(
            search.selected_item().unwrap().session_name(),
            Some("alpha")
        );
        search.clear();
        assert!(!search.is_searching());
        assert!(search.results().is_empty());
        assert!(search.selected_item().is_none());
    }
}
