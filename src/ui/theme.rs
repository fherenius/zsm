use zellij_tile::prelude::Text;
use zsm::text::{
    display_width, elide_middle, elide_start, remap_indices_after_elide_middle, truncate_columns,
};

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
    pub fn search_prompt(&self, term: &str, width: usize) -> Text {
        self.field("Search:", &format!("{term}_"), "", width)
    }

    /// Fit the editable value first; show the optional hint only if room remains.
    pub fn field(&self, label: &str, value: &str, hint: &str, width: usize) -> Text {
        let label = truncate_columns(label, width);
        let remaining = width.saturating_sub(display_width(&label));
        let space = if remaining > 0 { " " } else { "" };
        let available = remaining.saturating_sub(space.len());
        let hint = if !hint.is_empty()
            && available >= display_width(hint) + 1 + display_width(value).min(12)
        {
            format!(" {hint}")
        } else {
            String::new()
        };
        let value = elide_start(value, available.saturating_sub(display_width(&hint)));
        let start = label.chars().count() + space.len();
        let end = start + value.chars().count();
        Text::new(format!("{label}{space}{value}{hint}"))
            .color_range(2, ..label.chars().count())
            .color_range(1, start..end)
            .color_range(3, end..)
    }

    pub fn layout(&self, name: &str, builtin: bool, indices: &[usize], width: usize) -> Text {
        let suffix = if builtin && width >= 16 {
            " (built-in)"
        } else {
            ""
        };
        let name_width = width.saturating_sub(suffix.len());
        let shortened = elide_middle(name, name_width);
        let name_end = shortened.chars().count();
        let indices = remap_indices_after_elide_middle(name, name_width, indices);
        Text::new(format!("{shortened}{suffix}"))
            .color_range(1, ..name_end)
            .color_range(0, name_end..)
            .color_indices(3, indices)
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
