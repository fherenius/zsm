use zellij_tile::prelude::{
    print_table_with_coordinates, print_text_with_coordinates, Table, Text,
};

use zsm::list::visible_range;
use zsm::text::{
    display_width, elide_middle, elide_start, remap_indices_after_elide_middle,
    remap_indices_after_elide_start, truncate_columns, wrap_columns,
};

use crate::session::SessionItem;
use crate::state::{ActiveScreen, PluginState};
use crate::ui::Theme;

/// Main renderer for the plugin UI
pub struct PluginRenderer;

impl PluginRenderer {
    /// Render the main plugin interface
    pub fn render(state: &PluginState, rows: usize, cols: usize) {
        if rows == 0 || cols == 0 {
            return;
        }
        let (x, y, width, height) = Self::calculate_main_size(rows, cols);

        match state.active_screen() {
            ActiveScreen::Main => {
                Self::render_main_screen(state, x, y, width, height);
            }
            ActiveScreen::NewSession => {
                Self::render_new_session_screen(state, x, y, width, height);
            }
        }

        // Render overlays
        if let Some(error) = state.error() {
            Self::render_error(error, x, y, width, height);
        } else if let Some(session_name) = state.session_manager().pending_deletion() {
            Self::render_deletion_confirmation(state, session_name, x, y, width, height);
        }
    }

    /// Render the main screen with directory/session list
    fn render_main_screen(state: &PluginState, x: usize, y: usize, width: usize, height: usize) {
        let theme = Theme;

        // Render title
        print_text_with_coordinates(
            theme.title(&truncate_columns("Zoxide Session Manager", width)),
            x,
            y,
            Some(width),
            Some(1),
        );

        // Render search indication
        let search_indication = theme.search_prompt(state.search_engine().search_term(), width);
        print_text_with_coordinates(search_indication, x, y + 2, Some(width), Some(1));

        // Render main content
        let table_rows = height.saturating_sub(6);
        let table = if state.search_engine().is_searching() {
            Self::render_search_results(state, table_rows, width, theme)
        } else {
            Self::render_all_items(state, table_rows, width, theme)
        };

        if state.visible_item_count() == 0 && !state.search_engine().is_searching() {
            for (row, line) in wrap_columns(
                "No project directories found. Visit directories with zoxide, then press Ctrl+r.",
                width,
            )
            .iter()
            .take(table_rows)
            .enumerate()
            {
                print_text_with_coordinates(
                    theme.warning(line),
                    x,
                    y + 4 + row,
                    Some(width),
                    Some(1),
                );
            }
        } else {
            print_table_with_coordinates(table, x, y + 4, Some(width), Some(table_rows));
        }

        // Render help text
        Self::render_help_text(state, x, y + height.saturating_sub(1), width, theme);
    }

    /// Render new session creation screen
    fn render_new_session_screen(
        state: &PluginState,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) {
        crate::ui::components::render_new_session_block(
            state.new_session_info(),
            height.saturating_sub(2),
            width,
            x,
            y,
        );
    }

    /// Render search results table
    fn render_search_results(
        state: &PluginState,
        table_rows: usize,
        table_width: usize,
        theme: Theme,
    ) -> Table {
        let mut table = Table::new().add_row(vec!["Directory/Session"]);
        let results = state.search_engine().results();
        let selected_index = state.search_engine().selected_index();

        let (first_row, last_row) = visible_range(table_rows, results.len(), selected_index);

        for i in first_row..last_row {
            if let Some(result) = results.get(i) {
                let is_selected = Some(i) == selected_index;
                let mut table_cells = vec![Self::render_search_result_item(
                    &result.item,
                    &result.indices,
                    table_width.saturating_sub(4),
                    theme,
                )];

                if is_selected {
                    table_cells = table_cells.drain(..).map(|t| t.selected()).collect();
                }

                table = table.add_styled_row(table_cells);
            }
        }

        table
    }

    /// Render all items table
    fn render_all_items(
        state: &PluginState,
        table_rows: usize,
        table_width: usize,
        theme: Theme,
    ) -> Table {
        let mut table = Table::new().add_row(vec!["Directory/Session"]);
        let items = state.combined_items();
        let selected_index = state.selected_index();

        let (first_row, last_row) = visible_range(table_rows, items.len(), selected_index);

        for i in first_row..last_row {
            if let Some(item) = items.get(i) {
                let is_selected = Some(i) == selected_index;
                let mut table_cells = vec![Self::render_item(
                    item,
                    table_width.saturating_sub(4),
                    theme,
                )];

                if is_selected {
                    table_cells = table_cells.drain(..).map(|t| t.selected()).collect();
                }

                table = table.add_styled_row(table_cells);
            }
        }

        table
    }

    /// Render a search result item, moving the highlight positions onto the
    /// shortened text that is actually drawn.
    fn render_search_result_item(
        item: &SessionItem,
        indices: &[usize],
        max_width: usize,
        theme: Theme,
    ) -> Text {
        let text = Self::render_item(item, max_width, theme);

        if indices.is_empty() {
            return text;
        }

        // `indices` address `SessionItem::display_text`, which `render_item`
        // shortens to fit the pane, so they have to be remapped the same way.
        let display_text = item.display_text();
        let adjusted_indices = match item {
            SessionItem::Directory { pinned: false, .. } => {
                remap_indices_after_elide_start(&display_text, max_width, indices)
            }
            _ => remap_indices_after_elide_middle(&display_text, max_width, indices),
        };

        if adjusted_indices.is_empty() {
            return text;
        }

        theme.highlight(text, adjusted_indices)
    }

    /// Render a session item, shortened to `max_width` terminal columns.
    ///
    /// Directories keep their tail (the project directory is the informative
    /// part); sessions keep both ends, since the name leads and the directory
    /// trails.
    fn render_item(item: &SessionItem, max_width: usize, theme: Theme) -> Text {
        let display_text = item.display_text();

        match item {
            SessionItem::ExistingSession { is_current, .. } => {
                let shortened = elide_middle(&display_text, max_width);
                if *is_current {
                    theme.current_session(&shortened)
                } else {
                    theme.available_session(&shortened)
                }
            }
            SessionItem::ResurrectableSession { .. } => {
                theme.available_session(&elide_middle(&display_text, max_width))
            }
            SessionItem::Directory { pinned, .. } => theme.content(&if *pinned {
                elide_middle(&display_text, max_width)
            } else {
                elide_start(&display_text, max_width)
            }),
        }
    }

    /// Render help text
    fn render_help_text(state: &PluginState, x: usize, y: usize, width: usize, theme: Theme) {
        // The empty list used to advertise "Type session name and press Enter",
        // which does nothing: typing searches, and Enter with no selection is
        // a no-op. Say what the keys actually do instead.
        let help_text = if state.visible_item_count() == 0 {
            if state.search_engine().is_searching() {
                "Backspace: Edit search • Esc: Clear search"
            } else {
                "Ctrl+r: reload directories • Esc: Exit"
            }
        } else if width < 100 {
            "Enter: Open • Ctrl+p: Pin/unpin • Esc: Back"
        } else {
            "↑/↓: Navigate • Enter: Switch/New • Ctrl+Enter: Quick create • Ctrl+p: Pin/unpin • Ctrl+r: Reload • Ctrl+d: Kill • Type: Search • Esc: Exit"
        };

        print_text_with_coordinates(
            theme.help(&truncate_columns(help_text, width)),
            x,
            y,
            Some(width),
            Some(1),
        );
    }

    /// Render error message
    fn render_error(error: &str, x: usize, y: usize, width: usize, height: usize) {
        let lines = wrap_columns(error, width);
        let start = y + height.saturating_sub(lines.len()) / 2;
        for (row, line) in lines.iter().take(height).enumerate() {
            print_text_with_coordinates(Theme.warning(line), x, start + row, Some(width), Some(1));
        }
    }

    fn render_deletion_confirmation(
        state: &PluginState,
        session_name: &str,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) {
        let dialog_width = width.min(64);
        let inner = dialog_width.saturating_sub(2);
        if inner == 0 {
            return;
        }
        let warning = if state.is_current_session(session_name) {
            "This is your current session. Killing it will disconnect you."
        } else {
            "A live session will be killed; a resurrectable session will be deleted."
        };
        let mut lines = vec![elide_middle(
            &format!("Kill session '{session_name}'?"),
            inner,
        )];
        lines.extend(wrap_columns(warning, inner));
        lines.extend(wrap_columns("y: Confirm • n/Esc: Cancel", inner));
        let dialog_x = x + width.saturating_sub(dialog_width) / 2;
        let dialog_y = y + height.saturating_sub(lines.len() + 2) / 2;
        let border = format!("+{}+", "-".repeat(inner));
        let mut framed = vec![border.clone()];
        framed.extend(lines.iter().map(|line| {
            format!(
                "|{}{}|",
                line,
                " ".repeat(inner.saturating_sub(display_width(line)))
            )
        }));
        framed.push(border);
        for (row, line) in framed.iter().take(height).enumerate() {
            print_text_with_coordinates(
                Theme.warning(line),
                dialog_x,
                dialog_y + row,
                Some(dialog_width),
                Some(1),
            );
        }
    }

    /// Calculate main UI size
    fn calculate_main_size(rows: usize, cols: usize) -> (usize, usize, usize, usize) {
        let width = cols;
        let x = 0;
        let y = 0;
        let height = rows.saturating_sub(y);
        (x, y, width, height)
    }
}
