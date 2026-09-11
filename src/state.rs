use crate::session::creation::CreationRequest;
use std::collections::BTreeMap;
use std::time::Duration;
use zellij_tile::prelude::*;
use zsm::config::Config;
use zsm::list;
use zsm::session_usage::SessionUsage;

use crate::new_session_info::{NewSessionInfo, SelectionOutcome};
use crate::session::{SessionAction, SessionItem, SessionManager};
use crate::zoxide::{SearchEngine, ZoxideDirectory};

/// The main plugin state
#[derive(Default)]
pub struct PluginState {
    pub(crate) permissions_granted: bool,
    pub(crate) is_visible: bool,
    pub(crate) refresh_timer_pending: bool,
    session_usage: SessionUsage,
    /// Plugin configuration
    config: Config,
    /// Session manager
    session_manager: SessionManager,
    /// Zoxide directories (managed separately from sessions)
    zoxide_directories: Vec<ZoxideDirectory>,
    /// Search engine for fuzzy finding
    search_engine: SearchEngine,
    /// New session creation component
    new_session_info: NewSessionInfo,
    /// Current active screen
    active_screen: ActiveScreen,
    /// Error message to display
    error: Option<String>,
    /// Current session name
    current_session_name: Option<String>,
    /// Request IDs for plugin communication
    request_ids: Vec<String>,
    /// Selected index in main list (when not searching)
    selected_index: Option<usize>,
    /// Cached merge of sessions and zoxide directories.
    ///
    /// Rebuilt only when sessions or directories change. It is read several
    /// times per render and once per keystroke, and building it clones every
    /// name and path and matches each session against every directory, so
    /// rebuilding it on demand was pure waste.
    combined_items: Vec<SessionItem>,
}

/// Represents the different screens in the plugin
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ActiveScreen {
    /// Main screen showing zoxide directories and sessions
    #[default]
    Main,
    /// New session creation screen
    NewSession,
}

impl PluginState {
    /// Initialize plugin with configuration
    pub fn initialize(&mut self, configuration: BTreeMap<String, String>) {
        self.config = Config::from_zellij_config(&configuration);
    }

    /// Update session information
    fn update_current_session_info(&mut self, sessions: &[SessionInfo]) {
        // Store current session name
        for session in sessions {
            if session.is_current_session {
                self.current_session_name = Some(session.name.clone());
                self.new_session_info
                    .update_layout_list(session.available_layouts.clone());
                break;
            }
        }
    }

    /// Apply a complete snapshot atomically, including removals.
    pub fn update_session_list(
        &mut self,
        sessions: Vec<SessionInfo>,
        resurrectable_sessions: Vec<(String, Duration)>,
    ) {
        self.update_current_session_info(&sessions);
        self.session_manager.update_sessions(sessions);
        self.session_manager
            .update_resurrectable_sessions(resurrectable_sessions);
        self.rebuild_combined_items();
    }

    pub fn update_session_notification(&mut self, sessions: Vec<SessionInfo>) {
        self.update_current_session_info(&sessions);
        self.session_manager.update_current_session(sessions);
        self.rebuild_combined_items();
    }

    /// Reset the picker on a visibility transition, preserving a filepicker's
    /// in-progress new-session form when focus returns from that plugin.
    pub fn set_visible(&mut self, visible: bool) {
        if visible && !self.is_visible {
            self.search_engine.clear();
            self.selected_index = None;
            self.error = None;
            self.session_manager.cancel_deletion();
        }
        self.is_visible = visible;
    }

    pub fn update_session_usage(&mut self, history: &str) {
        self.session_usage.merge(history);
        self.rebuild_combined_items();
    }

    /// Record the source and destination before switching; this also covers
    /// sessions where ZSM has not been opened yet.
    fn record_session_visit(&mut self, destination: Option<&str>) {
        if !self.permissions_granted {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut records = Vec::new();
        if let Some(name) = &self.current_session_name {
            records.push(self.session_usage.record(name, now));
        }
        if let Some(name) = destination.filter(|name| !name.is_empty()) {
            records.push(self.session_usage.record(name, now.saturating_add(1)));
        }
        let mut command = vec![
            "sh",
            "-c",
            include_str!("session/usage.sh"),
            "zsm-session-usage",
        ];
        command.extend(records.iter().map(String::as_str));
        run_command(
            &command,
            BTreeMap::from([("session_usage".into(), "true".into())]),
        );
        self.rebuild_combined_items();
    }

    pub fn refresh_session_usage(&mut self) {
        self.record_session_visit(None);
    }

    fn hide_picker(&mut self) {
        // Mark this immediately, even if the host's Visible(false) notification
        // is delayed, so the next open always starts with an empty search.
        self.set_visible(false);
        hide_self();
    }

    /// Update zoxide directories (managed separately from sessions)
    pub fn update_zoxide_directories(&mut self, directories: Vec<ZoxideDirectory>) {
        self.zoxide_directories = directories;
        self.rebuild_combined_items();
    }

    /// Handle key input
    pub fn handle_key(&mut self, key: KeyWithModifier) -> bool {
        // Clear error on any key press
        if self.error.is_some() {
            self.error = None;
            return true;
        }

        // Handle session deletion confirmation
        if let Some(session_name) = self
            .session_manager
            .pending_deletion()
            .map(|s| s.to_string())
        {
            return self.handle_deletion_confirmation(key, &session_name);
        }

        match self.active_screen {
            ActiveScreen::Main => self.handle_main_screen_key(key),
            ActiveScreen::NewSession => self.handle_new_session_key(key),
        }
    }

    /// Get current screen
    pub fn active_screen(&self) -> ActiveScreen {
        self.active_screen
    }

    /// The merged session and directory list, as last built.
    pub fn combined_items(&self) -> &[SessionItem] {
        &self.combined_items
    }

    /// How many rows are on screen: search results while searching, otherwise
    /// the full list.
    pub fn visible_item_count(&self) -> usize {
        if self.search_engine.is_searching() {
            self.search_engine.results().len()
        } else {
            self.combined_items.len()
        }
    }

    /// Rebuild the cached item list, then bring the search and selection back
    /// in line with it.
    fn rebuild_combined_items(&mut self) {
        let selected = self.selected_item();
        self.combined_items = self.build_combined_items();
        if !self.search_engine.is_searching() {
            if let Some(selected) = selected {
                if let Some(index) = self
                    .combined_items
                    .iter()
                    .position(|item| item.same_identity(&selected))
                {
                    self.selected_index = Some(index);
                }
            }
        }
        self.update_search_if_needed();
        self.clamp_selection();
    }

    /// Combine sessions and zoxide directories for display
    fn build_combined_items(&self) -> Vec<SessionItem> {
        let mut items = Vec::new();

        // Show every live Zellij session. A zoxide directory is matched only to
        // decorate the entry with its path (by generated/incremented name) — it
        // never gates visibility. Gating on a match would silently hide sessions
        // with random auto-names, sessions whose cwd isn't in zoxide, and sessions
        // created under a different `base_paths` config, leaving no way to switch
        // to or kill them.
        for session in self.session_manager.sessions() {
            let directory = self
                .zoxide_directories
                .iter()
                .find(|zoxide_dir| {
                    session.name == zoxide_dir.session_name
                        || self.is_incremented_session(&session.name, &zoxide_dir.session_name)
                })
                .map(|zoxide_dir| zoxide_dir.directory.clone())
                .unwrap_or_default();

            items.push(SessionItem::ExistingSession {
                name: session.name.clone(),
                directory,
                is_current: session.is_current_session,
            });
        }

        // Add all resurrectable sessions if configured to show them — again
        // regardless of whether they map to a zoxide directory.
        if self.config.show_resurrectable_sessions {
            for (name, duration) in self.session_manager.resurrectable_sessions() {
                if self
                    .session_manager
                    .sessions()
                    .iter()
                    .any(|session| &session.name == name)
                {
                    continue;
                }
                items.push(SessionItem::ResurrectableSession {
                    name: name.clone(),
                    duration: *duration,
                });
            }
        }

        items.sort_by(|a, b| {
            self.session_usage.compare(
                a.session_name().unwrap_or_default(),
                b.session_name().unwrap_or_default(),
            )
        });

        // Then add all zoxide directories (always show directories, even if sessions exist)
        for dir in &self.zoxide_directories {
            items.push(SessionItem::Directory {
                path: dir.directory.clone(),
                session_name: dir.session_name.clone(),
            });
        }

        items
    }

    /// Pull `selected_index` back inside the item list.
    ///
    /// The list moves underneath the selection whenever zoxide is re-queried
    /// (every time the plugin is shown) or a session starts or dies. A stale
    /// index used to scroll the render window off the end of the list, drawing
    /// an empty table while items existed, and made Enter a silent no-op.
    fn clamp_selection(&mut self) {
        self.selected_index = list::clamp_selection(self.selected_index, self.combined_items.len());
    }

    /// Check if session name is an incremented version of base name  
    fn is_incremented_session(&self, session_name: &str, base_name: &str) -> bool {
        if session_name.len() <= base_name.len() || !session_name.starts_with(base_name) {
            return false;
        }

        let remainder = &session_name[base_name.len()..];
        if !remainder.starts_with(&self.config.session_separator) {
            return false;
        }

        // An empty remainder fails to parse, so it needs no separate check.
        remainder[self.config.session_separator.len()..]
            .parse::<u32>()
            .is_ok()
    }

    /// Get search engine (for UI rendering)
    pub fn search_engine(&self) -> &SearchEngine {
        &self.search_engine
    }

    /// Get new session info (for UI rendering)
    pub fn new_session_info(&self) -> &NewSessionInfo {
        &self.new_session_info
    }

    /// Get session manager (for UI rendering)
    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    /// Get selected index for main screen
    pub fn selected_index(&self) -> Option<usize> {
        if self.search_engine.is_searching() {
            self.search_engine.selected_index()
        } else {
            self.selected_index
        }
    }

    /// Whether `name` is the session the plugin is running in.
    pub fn is_current_session(&self, name: &str) -> bool {
        self.current_session_name.as_deref() == Some(name)
    }

    /// Show error message
    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    /// Get current error
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Get current configuration
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Get selected item
    pub fn selected_item(&self) -> Option<SessionItem> {
        if self.search_engine.is_searching() {
            self.search_engine.selected_item().cloned()
        } else {
            self.selected_index
                .and_then(|index| self.combined_items.get(index).cloned())
        }
    }

    /// Handle main screen key input
    fn handle_main_screen_key(&mut self, key: KeyWithModifier) -> bool {
        match key.bare_key {
            BareKey::Up if key.has_no_modifiers() => {
                self.move_selection_up();
                true
            }
            BareKey::Down if key.has_no_modifiers() => {
                self.move_selection_down();
                true
            }
            BareKey::Enter if key.has_no_modifiers() => {
                self.handle_item_selection();
                true
            }
            BareKey::Enter if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.handle_quick_session_creation();
                true
            }
            BareKey::Char('d') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.handle_kill_shortcut();
                true
            }
            BareKey::Char(c) if key.has_no_modifiers() && c != '\n' => {
                // Always search the full item list, never the previous results.
                self.search_engine.add_char(c, &self.combined_items);
                true
            }
            BareKey::Backspace if key.has_no_modifiers() => {
                self.search_engine.backspace(&self.combined_items);
                true
            }
            BareKey::Esc if key.has_no_modifiers() => {
                if self.search_engine.is_searching() {
                    self.search_engine.clear();
                    true
                } else {
                    self.hide_picker();
                    false
                }
            }
            BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.hide_picker();
                false
            }
            BareKey::Char('r') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                // reload zoxide directories and re-pull the session list
                self.fetch_zoxide_directories();
                self.fetch_sessions();
                self.refresh_session_usage();
                true
            }
            _ => false,
        }
    }

    /// Handle new session screen key input
    fn handle_new_session_key(&mut self, key: KeyWithModifier) -> bool {
        match key.bare_key {
            BareKey::Enter if key.has_no_modifiers() => {
                // Only leave the screen once a session was actually requested.
                // Enter in the name field just moves on to the layout picker;
                // returning to the main screen there threw the name away and
                // made the layout step unreachable.
                match self
                    .new_session_info
                    .handle_selection(&self.current_session_name, |name| {
                        self.session_manager.name_taken(name)
                    }) {
                    SelectionOutcome::AdvancedToLayout => {}
                    SelectionOutcome::Create(request) => self.create_session(request),
                    SelectionOutcome::Rejected(message) => self.set_error(message.to_string()),
                }
                true
            }
            BareKey::Enter if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                match self
                    .new_session_info
                    .quick_creation_request(self.config.default_layout.as_deref())
                {
                    Ok(request) => self.create_session(request),
                    Err(message) => self.set_error(message.to_string()),
                }
                true
            }
            BareKey::Esc if key.has_no_modifiers() => {
                // Special handling for Esc when entering session name - go back to main
                if self.new_session_info.entering_new_session_name()
                    && self.new_session_info.name().is_empty()
                {
                    self.active_screen = ActiveScreen::Main;
                } else {
                    // Let NewSessionInfo handle its own escape logic
                    self.new_session_info.handle_key(key);
                }
                true
            }
            BareKey::Char('f') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                // Handle filepicker
                self.launch_filepicker();
                true
            }
            BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                // Clear session folder - don't delegate to NewSessionInfo
                self.new_session_info.set_folder(None);
                true
            }
            _ => {
                // Delegate other keys to NewSessionInfo component
                self.new_session_info.handle_key(key);
                true
            }
        }
    }

    /// Handle deletion confirmation
    fn handle_deletion_confirmation(&mut self, key: KeyWithModifier, _session_name: &str) -> bool {
        match key.bare_key {
            BareKey::Char('y') | BareKey::Char('Y') if key.has_no_modifiers() => {
                match self.session_manager.confirm_deletion() {
                    // Re-pull the list: our copy still holds the session we
                    // just killed, so the row would linger until the next
                    // SessionUpdate happened to arrive.
                    Ok(()) => self.fetch_sessions(),
                    Err(e) => self.set_error(format!("Failed to delete session: {}", e)),
                }
                true
            }
            BareKey::Char('n') | BareKey::Char('N') | BareKey::Esc if key.has_no_modifiers() => {
                self.session_manager.cancel_deletion();
                true
            }
            _ => false,
        }
    }

    /// Move selection up
    fn move_selection_up(&mut self) {
        if self.search_engine.is_searching() {
            self.search_engine.move_selection_up();
        } else {
            self.selected_index =
                list::select_previous(self.selected_index, self.combined_items.len());
        }
    }

    /// Move selection down
    fn move_selection_down(&mut self) {
        if self.search_engine.is_searching() {
            self.search_engine.move_selection_down();
        } else {
            self.selected_index = list::select_next(self.selected_index, self.combined_items.len());
        }
    }

    /// Handle item selection (Enter key)
    fn handle_item_selection(&mut self) {
        // Get the selected item data before any mutable borrows
        let selected_item_data = self.selected_item().map(|item| match item {
            SessionItem::ExistingSession { name, .. } => (true, name, String::new()),
            SessionItem::Directory {
                session_name, path, ..
            } => (false, session_name, path),
            SessionItem::ResurrectableSession { name, .. } => (true, name, String::new()),
        });

        if let Some((is_session, name, path)) = selected_item_data {
            if is_session {
                self.record_session_visit(Some(&name));
                // Switch to existing session (infallible)
                let _ = self
                    .session_manager
                    .execute_action(SessionAction::Switch(name));
                self.hide_picker();
            } else {
                // Create new session with incremented name
                let incremented_name = self
                    .session_manager
                    .generate_incremented_name(&name, &self.config.session_separator);

                // Set up new session creation
                self.new_session_info.set_name(&incremented_name);
                self.new_session_info
                    .set_folder(Some(std::path::PathBuf::from(&path)));
                self.new_session_info.advance_to_layout_selection();
                self.active_screen = ActiveScreen::NewSession;
            }
        }
    }

    /// Handle the kill-session shortcut
    fn handle_kill_shortcut(&mut self) {
        // Get the selected item data before any mutable borrows
        let selected_session_name = self.selected_item().and_then(|item| match item {
            SessionItem::ExistingSession { name, .. } => Some(name),
            SessionItem::ResurrectableSession { name, .. } => Some(name),
            _ => None,
        });

        if let Some(session_name) = selected_session_name {
            self.session_manager.start_deletion(session_name);
        }
    }

    /// Re-run the active search against the rebuilt item list.
    fn update_search_if_needed(&mut self) {
        if self.search_engine.is_searching() {
            let term = self.search_engine.search_term().to_string();
            self.search_engine.update_search(term, &self.combined_items);
        }
    }

    /// Launch filepicker for new session folder selection
    fn launch_filepicker(&mut self) {
        use uuid::Uuid;
        use zellij_tile::prelude::{pipe_message_to_plugin, MessageToPlugin};

        let request_id = Uuid::new_v4();
        let mut config = BTreeMap::new();
        let mut args = BTreeMap::new();

        self.request_ids.push(request_id.to_string());

        // we insert this into the config so that a new plugin will be opened (the plugin's
        // uniqueness is determined by its name/url as well as its config)
        config.insert("request_id".to_owned(), request_id.to_string());

        // Start filepicker at the current session folder if set
        if let Some(folder) = self.new_session_info.new_session_folder() {
            config.insert(
                "caller_cwd".to_owned(),
                folder.to_string_lossy().to_string(),
            );
        }

        // we also insert this into the args so that the plugin will have an easier access to it
        args.insert("request_id".to_owned(), request_id.to_string());

        pipe_message_to_plugin(
            MessageToPlugin::new("filepicker")
                .with_plugin_url("filepicker")
                .with_plugin_config(config)
                .new_plugin_instance_should_have_pane_title("Select folder for the new session...")
                .with_args(args),
        );
    }

    /// Check if a request ID is valid (exists in our request list)
    pub fn is_valid_request_id(&self, request_id: &str) -> bool {
        self.request_ids.contains(&request_id.to_string())
    }

    /// Remove a request ID from our tracking list
    pub fn remove_request_id(&mut self, request_id: &str) {
        self.request_ids.retain(|id| id != request_id);
    }

    /// Set new session folder
    pub fn set_new_session_folder(&mut self, folder: Option<std::path::PathBuf>) {
        self.new_session_info.set_folder(folder);
    }

    /// Validate and execute every new-session request through one path.
    fn create_session(&mut self, request: CreationRequest) {
        if let Err(message) = request.validate(self.current_session_name.as_deref(), |name| {
            self.session_manager.name_taken(name)
        }) {
            self.set_error(message.to_string());
            return;
        }
        self.record_session_visit(Some(&request.name));
        request.execute();
        self.new_session_info.reset_after_creation();
        self.active_screen = ActiveScreen::Main;
        self.hide_picker();
    }

    fn handle_quick_session_creation(&mut self) {
        match self.selected_item() {
            Some(
                SessionItem::ExistingSession { name, .. }
                | SessionItem::ResurrectableSession { name, .. },
            ) => {
                self.record_session_visit(Some(&name));
                let _ = self
                    .session_manager
                    .execute_action(SessionAction::Switch(name));
                self.hide_picker();
            }
            Some(SessionItem::Directory {
                session_name, path, ..
            }) => {
                let name = self
                    .session_manager
                    .generate_incremented_name(&session_name, &self.config.session_separator);
                let layouts = self
                    .session_manager
                    .sessions()
                    .iter()
                    .find(|session| session.is_current_session)
                    .map(|session| session.available_layouts.as_slice())
                    .unwrap_or_default();
                match CreationRequest::with_default_layout(
                    name,
                    Some(path.into()),
                    self.config.default_layout.as_deref(),
                    layouts,
                ) {
                    Ok(request) => self.create_session(request),
                    Err(message) => self.set_error(message.to_string()),
                }
            }
            None => self.set_error("Please select a directory".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str, current: bool) -> SessionInfo {
        SessionInfo {
            name: name.into(),
            is_current_session: current,
            ..Default::default()
        }
    }

    fn names(state: &PluginState) -> Vec<&str> {
        state
            .combined_items()
            .iter()
            .filter_map(SessionItem::session_name)
            .collect()
    }

    #[test]
    fn stale_notifications_cannot_hide_peers_or_restore_removed_sessions() {
        let mut state = PluginState::default();
        state.config.show_resurrectable_sessions = true;
        state.update_session_list(
            vec![session("current", true), session("peer", false)],
            vec![("dead".into(), Duration::from_secs(10))],
        );
        state.update_session_notification(vec![session("current", true)]);
        assert_eq!(names(&state), ["current", "dead", "peer"]);

        state.update_session_list(vec![session("current", true)], vec![]);
        state.update_session_notification(vec![session("current", true), session("peer", false)]);
        assert_eq!(names(&state), ["current"]);
    }

    #[test]
    fn sessions_are_visible_without_zoxide_matches_and_live_entries_are_unique() {
        let mut state = PluginState::default();
        state.config.show_resurrectable_sessions = true;
        state.update_zoxide_directories(vec![ZoxideDirectory {
            directory: "/work/project".into(),
            session_name: "project".into(),
            ranking: 100.0,
        }]);
        state.update_session_list(
            vec![
                session("random-name", true),
                session("old-config-name", false),
            ],
            vec![
                ("random-name".into(), Duration::ZERO),
                ("dead".into(), Duration::ZERO),
            ],
        );
        assert_eq!(names(&state), ["dead", "old-config-name", "random-name"]);
        assert!(matches!(
            state.combined_items().last(),
            Some(SessionItem::Directory { .. })
        ));
    }

    #[test]
    fn reopening_clears_search_results_selection_and_pending_dialogs() {
        let mut state = PluginState::default();
        state.update_session_list(vec![session("alpha", true), session("beta", false)], vec![]);
        state.set_visible(true);
        state
            .search_engine
            .update_search("alpha".into(), &state.combined_items);
        state.selected_index = Some(1);
        state.error = Some("old error".into());
        state.session_manager.start_deletion("alpha".into());
        state.set_visible(false);
        state.set_visible(true);
        assert_eq!(state.search_engine.search_term(), "");
        assert!(!state.search_engine.is_searching());
        assert!(state.search_engine.results().is_empty());
        assert_eq!(state.selected_index(), None);
        assert_eq!(state.visible_item_count(), 2);
        assert!(state.error().is_none());
        assert!(state.session_manager.pending_deletion().is_none());
    }

    #[test]
    fn refreshes_and_repeated_visible_events_preserve_an_active_search() {
        let mut state = PluginState::default();
        state.set_visible(true);
        state.update_session_list(vec![session("alpha", true)], vec![]);
        state
            .search_engine
            .update_search("beta".into(), &state.combined_items);
        assert_eq!(state.visible_item_count(), 0);
        state.update_session_list(vec![session("alpha", true), session("beta", false)], vec![]);
        state.set_visible(true);
        assert_eq!(state.search_engine.search_term(), "beta");
        assert_eq!(state.visible_item_count(), 1);
        assert_eq!(state.selected_item().unwrap().session_name(), Some("beta"));
    }

    #[test]
    fn recency_refresh_keeps_the_selected_session_and_sessions_before_directories() {
        let mut state = PluginState::default();
        state.update_session_list(vec![session("alpha", true), session("zulu", false)], vec![]);
        state.update_zoxide_directories(vec![ZoxideDirectory {
            directory: "/work/project".into(),
            session_name: "project".into(),
            ranking: 100.0,
        }]);
        state.selected_index = Some(1);
        let history = state.session_usage.record("zulu", 20);
        state.update_session_usage(&history);
        assert_eq!(names(&state), ["zulu", "alpha"]);
        assert_eq!(state.selected_index(), Some(0));
        assert_eq!(state.selected_item().unwrap().session_name(), Some("zulu"));
        assert!(matches!(
            state.combined_items().last(),
            Some(SessionItem::Directory { .. })
        ));
    }

    #[test]
    fn visibility_changes_preserve_the_new_session_form() {
        let mut state = PluginState {
            active_screen: ActiveScreen::NewSession,
            ..Default::default()
        };
        state.new_session_info.set_name("project");
        state.new_session_info.advance_to_layout_selection();
        state.set_visible(true);
        state.set_visible(false);
        state.set_visible(true);
        assert_eq!(state.active_screen(), ActiveScreen::NewSession);
        assert_eq!(state.new_session_info.name(), "project");
        assert!(state.new_session_info.entering_layout_search_term());
    }
}
