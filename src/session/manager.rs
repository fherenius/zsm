use crate::session::listing::SessionListing;
use crate::session::types::SessionAction;
use std::time::Duration;
use zellij_tile::prelude::{delete_dead_session, kill_sessions, switch_session, SessionInfo};
use zsm::session_name;

/// Manages session operations and state
#[derive(Debug, Default)]
pub struct SessionManager {
    /// Currently known sessions from Zellij
    sessions: Vec<SessionInfo>,
    /// Session name pending deletion confirmation
    pending_deletion: Option<String>,
    /// Resurrectable sessions
    resurrectable_sessions: Vec<(String, Duration)>,
    listing: Option<SessionListing>,
}

impl SessionManager {
    /// Update the session list with new session information
    pub fn update_sessions(&mut self, mut sessions: Vec<SessionInfo>) {
        // A metadata scan can omit even our own session. Keep its fresh event
        // data (layouts in particular), and details for socket-confirmed peers.
        for previous in &self.sessions {
            let confirmed = previous.is_current_session
                || self
                    .listing
                    .as_ref()
                    .is_some_and(|list| list.live.iter().any(|(name, _)| name == &previous.name));
            if confirmed && !sessions.iter().any(|s| s.name == previous.name) {
                sessions.push(previous.clone());
            }
        }
        self.sessions = sessions;
        self.reconcile_listing();
    }

    pub fn update_listing(&mut self, listing: SessionListing) {
        self.listing = Some(listing);
        self.reconcile_listing();
    }

    fn reconcile_listing(&mut self) {
        let Some(listing) = &self.listing else { return };
        self.sessions = listing
            .live
            .iter()
            .map(|(name, is_current)| {
                let mut session = self
                    .sessions
                    .iter()
                    .find(|s| &s.name == name)
                    .cloned()
                    .unwrap_or_default();
                session.name = name.clone();
                session.is_current_session = *is_current;
                session
            })
            .collect();
        self.resurrectable_sessions = listing.resurrectable.clone();
    }

    /// Passive notifications use Zellij's peer cache. Only the current session
    /// is fresh; membership is reconciled by explicit full snapshots instead.
    pub fn update_current_session(&mut self, sessions: Vec<SessionInfo>) {
        for session in sessions.into_iter().filter(|s| s.is_current_session) {
            if let Some(existing) = self.sessions.iter_mut().find(|s| s.name == session.name) {
                *existing = session;
            } else {
                self.sessions.push(session);
            }
        }
    }

    /// Update the resurrectable sessions
    pub fn update_resurrectable_sessions(
        &mut self,
        resurrectable_sessions: Vec<(String, Duration)>,
    ) {
        self.resurrectable_sessions = resurrectable_sessions;
        self.reconcile_listing();
    }

    /// Get all sessions
    pub fn sessions(&self) -> &[SessionInfo] {
        &self.sessions
    }

    /// Get all resurrectable sessions
    pub fn resurrectable_sessions(&self) -> &[(String, Duration)] {
        &self.resurrectable_sessions
    }

    /// Execute a session action.
    ///
    /// `Switch` is infallible. `Kill` is now a synchronous confirmation from the
    /// Zellij host (as of zellij-tile 0.44): killing a live session waits for it
    /// to acknowledge, and deleting a resurrectable session reports whether its
    /// cache directory was removed. Any failure message is propagated to the caller.
    pub fn execute_action(&mut self, action: SessionAction) -> Result<(), String> {
        match action {
            SessionAction::Switch(name) => {
                switch_session(Some(&name));
                Ok(())
            }
            SessionAction::Kill(name) => {
                if !self.sessions.iter().any(|session| session.name == name)
                    && self
                        .resurrectable_sessions
                        .iter()
                        .any(|(session_name, _)| session_name == &name)
                {
                    // If the session is resurrectable, we should delete it
                    delete_dead_session(&name)
                } else {
                    // Otherwise, we need to kill the session
                    kill_sessions(&[&name])
                }
            }
        }
    }

    /// Start session deletion confirmation
    pub fn start_deletion(&mut self, session_name: String) {
        self.pending_deletion = Some(session_name);
    }

    /// Confirm session deletion, returning any failure reported by the host
    pub fn confirm_deletion(&mut self) -> Result<(), String> {
        if let Some(session_name) = self.pending_deletion.take() {
            self.execute_action(SessionAction::Kill(session_name))
        } else {
            Ok(())
        }
    }

    /// Cancel session deletion
    pub fn cancel_deletion(&mut self) {
        self.pending_deletion = None;
    }

    /// Get session pending deletion
    pub fn pending_deletion(&self) -> Option<&str> {
        self.pending_deletion.as_deref()
    }

    /// Whether a session of this name exists, live or resurrectable.
    pub fn name_taken(&self, name: &str) -> bool {
        self.sessions.iter().any(|session| session.name == name)
            || self
                .resurrectable_sessions
                .iter()
                .any(|(resurrectable, _)| resurrectable == name)
    }

    /// Generate incremented session name for a base name
    pub fn generate_incremented_name(&self, base_name: &str, separator: &str) -> String {
        session_name::first_free_increment(base_name, separator, |name| self.name_taken(name))
            .unwrap_or_else(|| {
                let unique_suffix = uuid::Uuid::new_v4().to_string();
                session_name::with_suffix(base_name, separator, &unique_suffix[..8])
                    // A separator can itself be invalid or larger than the
                    // budget. The UUID fragment remains a safe name on its own.
                    .unwrap_or_else(|| unique_suffix[..8].to_string())
            })
    }
}
