/// Represents different types of items that can be displayed in the session list
#[derive(Debug, Clone)]
pub enum SessionItem {
    /// An existing Zellij session
    ExistingSession {
        name: String,
        directory: String,
        is_current: bool,
    },
    /// A resurrectable session that can be restored
    ResurrectableSession {
        name: String,
        duration: std::time::Duration,
    },
    /// A zoxide directory that can be used to create a new session
    Directory { path: String, session_name: String },
}

impl SessionItem {
    /// Check if this is an existing session
    pub fn is_session(&self) -> bool {
        matches!(self, SessionItem::ExistingSession { .. })
    }
    pub fn is_resurrectable_session(&self) -> bool {
        matches!(self, SessionItem::ResurrectableSession { .. })
    }

    /// The row text for this item, before it is shortened to fit the pane.
    ///
    /// Both the renderer and the search engine go through this, so fuzzy match
    /// indices always address the string that is actually drawn. Keeping two
    /// copies of this formatting previously let them drift apart.
    pub fn display_text(&self) -> String {
        match self {
            SessionItem::ExistingSession {
                name,
                directory,
                is_current,
            } => {
                let prefix = if *is_current {
                    "\u{25cf} "
                } else {
                    "\u{25cb} "
                };
                // Sessions with no matching zoxide directory (random auto-names,
                // cwd not in zoxide, names from an old base_paths scheme) have an
                // empty directory - drop the "()" rather than render it empty.
                if directory.is_empty() {
                    format!("{}{}", prefix, name)
                } else {
                    format!("{}{} ({})", prefix, name, directory)
                }
            }
            SessionItem::ResurrectableSession { name, duration } => format!(
                "\u{21ba} {} (created {} ago)",
                name,
                humantime::format_duration(*duration)
            ),
            SessionItem::Directory { path, .. } => path.clone(),
        }
    }
}

/// Actions that can be performed on sessions
#[derive(Debug, Clone)]
pub enum SessionAction {
    /// Switch to an existing session
    Switch(String),
    /// Kill an existing session
    Kill(String),
}
