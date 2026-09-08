use zellij_tile::prelude::Text;

/// Colour roles for the plugin UI.
///
/// Every colour is an *index*, which Zellij resolves against whatever theme
/// the user has configured: 0 = dim/subtle, 1 = warning, 2 = active, 3 = info.
/// Nothing here reads the palette, so no RGB value is ever hardcoded and the
/// UI follows the user's theme for free.
#[derive(Copy, Clone, Debug, Default)]
pub struct Theme;

impl Theme {
    /// Text for warnings.
    pub fn warning(&self, text: &str) -> Text {
        Text::new(text).color_range(1, ..)
    }

    /// The screen title.
    pub fn title(&self, text: &str) -> Text {
        Text::new(text).color_range(3, ..)
    }

    /// The key hints along the bottom of the screen.
    pub fn help(&self, text: &str) -> Text {
        Text::new(text).color_range(1, ..)
    }

    /// The search prompt: the label is highlighted, the typed term is not.
    pub fn search_prompt(&self, term: &str) -> Text {
        const LABEL: &str = "Search:";
        Text::new(format!("{} {}_", LABEL, term)).color_range(2, ..LABEL.len())
    }

    /// Text for regular content.
    pub fn content(&self, text: &str) -> Text {
        Text::new(text)
    }

    /// The session the plugin is running in.
    pub fn current_session(&self, text: &str) -> Text {
        Text::new(text).color_range(2, ..)
    }

    /// A session that can be switched to.
    pub fn available_session(&self, text: &str) -> Text {
        Text::new(text).color_range(3, ..)
    }

    /// The characters a search term matched.
    pub fn highlight(&self, text: Text, indices: Vec<usize>) -> Text {
        text.color_indices(3, indices)
    }
}
