use crate::session::creation::CreationRequest;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::path::PathBuf;
use zellij_tile::prelude::*;
use zsm::list::visible_range;
use zsm::session_name;

#[derive(Default)]
pub struct NewSessionInfo {
    name: String,
    layout_list: LayoutList,
    entering_new_session_info: EnteringState,
    pub new_session_folder: Option<PathBuf>,
}

#[derive(Default, Eq, PartialEq)]
enum EnteringState {
    #[default]
    EnteringName,
    EnteringLayoutSearch,
}

/// What pressing Enter on the new-session screen did.
///
/// The caller needs to tell "moved to the next field" apart from "finished",
/// otherwise it cannot know whether to leave the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionOutcome {
    /// Moved from the name field to the layout picker; stay on this screen.
    AdvancedToLayout,
    /// A session was requested; leave this screen.
    Create(CreationRequest),
    /// The name cannot be used; show this message instead.
    Rejected(&'static str),
}

impl NewSessionInfo {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }

    pub fn set_folder(&mut self, folder: Option<PathBuf>) {
        self.new_session_folder = folder;
    }

    pub fn new_session_folder(&self) -> Option<&PathBuf> {
        self.new_session_folder.as_ref()
    }

    pub fn advance_to_layout_selection(&mut self) {
        self.entering_new_session_info = EnteringState::EnteringLayoutSearch;
    }

    pub fn correct_session_name(&mut self) {
        // Go back to session name entry from layout selection
        self.layout_list.clear_search();
        self.entering_new_session_info = EnteringState::EnteringName;
    }
    pub fn layout_search_term(&self) -> &str {
        &self.layout_list.layout_search_term
    }
    pub fn entering_new_session_name(&self) -> bool {
        self.entering_new_session_info == EnteringState::EnteringName
    }
    pub fn entering_layout_search_term(&self) -> bool {
        self.entering_new_session_info == EnteringState::EnteringLayoutSearch
    }
    pub fn add_char(&mut self, character: char) {
        match self.entering_new_session_info {
            EnteringState::EnteringName => {
                self.name.push(character);
            }
            EnteringState::EnteringLayoutSearch => {
                self.layout_list.layout_search_term.push(character);
                self.update_layout_search_term();
            }
        }
    }
    pub fn handle_backspace(&mut self) {
        match self.entering_new_session_info {
            EnteringState::EnteringName => {
                self.name.pop();
            }
            EnteringState::EnteringLayoutSearch => {
                self.layout_list.layout_search_term.pop();
                self.update_layout_search_term();
            }
        }
    }
    pub fn handle_break(&mut self) {
        match self.entering_new_session_info {
            EnteringState::EnteringName => {
                self.name.clear();
            }
            EnteringState::EnteringLayoutSearch => {
                self.layout_list.clear_search();
                self.entering_new_session_info = EnteringState::EnteringName;
            }
        }
    }
    pub fn handle_key(&mut self, key: KeyWithModifier) {
        match key.bare_key {
            BareKey::Backspace if key.has_no_modifiers() => {
                self.handle_backspace();
            }
            BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.handle_break();
            }
            BareKey::Char('r')
                if key.has_modifiers(&[KeyModifier::Ctrl])
                // Ctrl+R to correct session name - only when in layout search mode
                && self.entering_new_session_info == EnteringState::EnteringLayoutSearch =>
            {
                self.correct_session_name();
            }
            BareKey::Esc if key.has_no_modifiers() => {
                // Esc goes back to previous state or clears current input
                match self.entering_new_session_info {
                    EnteringState::EnteringLayoutSearch => {
                        // In layout search, if there's a search term, clear it; otherwise go back to name entry
                        if !self.layout_list.layout_search_term.is_empty() {
                            self.layout_list.clear_search();
                        } else {
                            // No search term, go back to name entry
                            self.entering_new_session_info = EnteringState::EnteringName;
                        }
                    }
                    EnteringState::EnteringName => {
                        // In name entry, clear the name
                        self.name.clear();
                    }
                }
            }
            BareKey::Char(character) if key.has_no_modifiers() => {
                self.add_char(character);
            }
            BareKey::Up if key.has_no_modifiers() => {
                self.move_selection_up();
            }
            BareKey::Down if key.has_no_modifiers() => {
                self.move_selection_down();
            }
            _ => {}
        }
    }

    pub fn quick_creation_request(
        &self,
        default_layout: Option<&str>,
    ) -> Result<CreationRequest, &'static str> {
        CreationRequest::with_default_layout(
            self.name.clone(),
            self.new_session_folder.clone(),
            default_layout,
            &self.layout_list.layout_list,
        )
    }

    /// Return the form to its initial state while retaining the layouts fetched
    /// from Zellij for the next use.
    pub fn reset_after_creation(&mut self) {
        self.name.clear();
        self.new_session_folder = None;
        self.entering_new_session_info = EnteringState::EnteringName;
        self.layout_list.clear_search();
    }

    /// Advance the screen: name field to layout picker, layout picker to
    /// creating the session.
    ///
    /// The name is validated on the way out of the name field as well as at
    /// creation, so a name that cannot work is reported while the user is
    /// still looking at it.
    pub fn handle_selection(
        &mut self,
        current_session_name: &Option<String>,
        is_name_taken: impl FnOnce(&str) -> bool,
    ) -> SelectionOutcome {
        if let Err(message) = session_name::validate_for_creation(
            &self.name,
            current_session_name.as_deref(),
            is_name_taken,
        ) {
            return SelectionOutcome::Rejected(message);
        }

        match self.entering_new_session_info {
            EnteringState::EnteringLayoutSearch => {
                let layout = self.selected_layout_info();
                if self.is_searching() && layout.is_none() {
                    return SelectionOutcome::Rejected(
                        "No matching layout; edit or clear the search",
                    );
                }
                SelectionOutcome::Create(CreationRequest {
                    name: self.name.clone(),
                    folder: self.new_session_folder.clone(),
                    layout,
                })
            }
            EnteringState::EnteringName => {
                self.entering_new_session_info = EnteringState::EnteringLayoutSearch;
                SelectionOutcome::AdvancedToLayout
            }
        }
    }
    pub fn update_layout_list(&mut self, layout_info: Vec<LayoutInfo>) {
        self.layout_list.update_layout_list(layout_info);
    }
    pub fn layout_list(&self, max_rows: usize) -> Vec<(LayoutInfo, bool)> {
        // bool - is_selected
        let range_to_render = visible_range(
            max_rows,
            self.layout_count(),
            Some(self.layout_list.selected_layout_index),
        );
        self.layout_list
            .layout_list
            .iter()
            .enumerate()
            .map(|(i, l)| (l.clone(), i == self.layout_list.selected_layout_index))
            .take(range_to_render.1)
            .skip(range_to_render.0)
            .collect()
    }
    pub fn layouts_to_render(&self, max_rows: usize) -> Vec<(LayoutInfo, Vec<usize>, bool)> {
        // (layout_info,
        // search_indices,
        // is_selected)
        if self.is_searching() {
            self.layout_search_results(max_rows)
                .into_iter()
                .map(|(layout_search_result, is_selected)| {
                    (
                        layout_search_result.layout_info,
                        layout_search_result.indices,
                        is_selected,
                    )
                })
                .collect()
        } else {
            self.layout_list(max_rows)
                .into_iter()
                .map(|(layout_info, is_selected)| (layout_info, vec![], is_selected))
                .collect()
        }
    }
    pub fn layout_search_results(&self, max_rows: usize) -> Vec<(LayoutSearchResult, bool)> {
        // bool - is_selected
        let range_to_render = visible_range(
            max_rows,
            self.layout_list.layout_search_results.len(),
            Some(self.layout_list.selected_layout_index),
        );
        self.layout_list
            .layout_search_results
            .iter()
            .enumerate()
            .map(|(i, l)| (l.clone(), i == self.layout_list.selected_layout_index))
            .take(range_to_render.1)
            .skip(range_to_render.0)
            .collect()
    }
    pub fn is_searching(&self) -> bool {
        !self.layout_list.layout_search_term.is_empty()
    }
    pub fn layout_count(&self) -> usize {
        self.layout_list.layout_list.len()
    }
    pub fn selected_layout_info(&self) -> Option<LayoutInfo> {
        self.layout_list.selected_layout_info()
    }
    fn update_layout_search_term(&mut self) {
        self.layout_list.refresh_search();
        self.layout_list.clear_selection();
    }
    fn move_selection_up(&mut self) {
        self.layout_list.move_selection_up();
    }
    fn move_selection_down(&mut self) {
        self.layout_list.move_selection_down();
    }
}

#[derive(Default)]
pub struct LayoutList {
    layout_list: Vec<LayoutInfo>,
    layout_search_results: Vec<LayoutSearchResult>,
    selected_layout_index: usize,
    layout_search_term: String,
}

impl LayoutList {
    pub fn update_layout_list(&mut self, layout_list: Vec<LayoutInfo>) {
        let selected = self.selected_layout_info();
        self.layout_list = layout_list;
        self.refresh_search();
        self.selected_layout_index = selected
            .and_then(|selected| {
                if self.layout_search_term.is_empty() {
                    self.layout_list
                        .iter()
                        .position(|layout| same_layout(layout, &selected))
                } else {
                    self.layout_search_results
                        .iter()
                        .position(|result| same_layout(&result.layout_info, &selected))
                }
            })
            .unwrap_or(0);
    }

    fn refresh_search(&mut self) {
        self.layout_search_results.clear();
        if self.layout_search_term.is_empty() {
            return;
        }
        let matcher = SkimMatcherV2::default().use_cache(true);
        for layout in &self.layout_list {
            if let Some((score, indices)) =
                matcher.fuzzy_indices(layout.name(), &self.layout_search_term)
            {
                self.layout_search_results.push(LayoutSearchResult {
                    layout_info: layout.clone(),
                    score,
                    indices,
                });
            }
        }
        self.layout_search_results
            .sort_by_key(|result| std::cmp::Reverse(result.score));
    }
    pub fn selected_layout_info(&self) -> Option<LayoutInfo> {
        if !self.layout_search_term.is_empty() {
            self.layout_search_results
                .get(self.selected_layout_index)
                .map(|l| l.layout_info.clone())
        } else {
            self.layout_list.get(self.selected_layout_index).cloned()
        }
    }
    pub fn clear_selection(&mut self) {
        self.selected_layout_index = 0;
    }
    fn clear_search(&mut self) {
        self.layout_search_term.clear();
        self.layout_search_results.clear();
        self.clear_selection();
    }
    fn max_index(&self) -> usize {
        if self.layout_search_term.is_empty() {
            self.layout_list.len().saturating_sub(1)
        } else {
            self.layout_search_results.len().saturating_sub(1)
        }
    }
    fn move_selection_up(&mut self) {
        let max_index = self.max_index();
        if self.selected_layout_index > 0 {
            self.selected_layout_index -= 1;
        } else {
            self.selected_layout_index = max_index;
        }
    }
    fn move_selection_down(&mut self) {
        let max_index = self.max_index();
        if self.selected_layout_index < max_index {
            self.selected_layout_index += 1;
        } else {
            self.selected_layout_index = 0;
        }
    }
}

#[derive(Clone)]
pub struct LayoutSearchResult {
    pub layout_info: LayoutInfo,
    pub score: i64,
    pub indices: Vec<usize>,
}

// File metadata can change without changing a layout's identity.
fn same_layout(a: &LayoutInfo, b: &LayoutInfo) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b) && a.name() == b.name()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_preserves_identity_and_discards_removed_search_results() {
        let mut form = NewSessionInfo::default();
        let a = LayoutInfo::BuiltIn("alpha".into());
        let b = LayoutInfo::BuiltIn("beta".into());
        form.update_layout_list(vec![a.clone(), b.clone()]);
        form.advance_to_layout_selection();
        form.move_selection_down();
        form.update_layout_list(vec![b.clone(), a.clone()]);
        assert_eq!(form.selected_layout_info(), Some(b.clone()));
        form.add_char('b');
        form.update_layout_list(vec![a]);
        assert!(form.selected_layout_info().is_none());
        assert!(form.layouts_to_render(10).is_empty());
        assert!(matches!(
            form.handle_selection(&None, |_| false),
            SelectionOutcome::Rejected(_)
        ));
        form.update_layout_list(vec![b.clone()]);
        assert_eq!(form.selected_layout_info(), Some(b));
    }

    #[test]
    fn creation_prepares_a_request_without_resetting_the_form() {
        let mut form = NewSessionInfo::default();
        form.set_name("project");
        form.set_folder(Some("/work/project".into()));
        assert_eq!(
            form.handle_selection(&None, |_| false),
            SelectionOutcome::AdvancedToLayout
        );
        let SelectionOutcome::Create(request) = form.handle_selection(&None, |_| false) else {
            panic!("expected creation request")
        };
        assert_eq!(request.name, "project");
        assert_eq!(request.folder, Some("/work/project".into()));
        assert_eq!(form.name(), "project");
        form.reset_after_creation();
        assert!(form.name().is_empty());
        assert!(form.new_session_folder().is_none());
    }
}
