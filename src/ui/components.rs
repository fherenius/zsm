use crate::new_session_info::NewSessionInfo;
use crate::ui::Theme;
use zellij_tile::prelude::*;
use zsm::text::truncate_columns;

pub fn render_new_session_block(
    form: &NewSessionInfo,
    rows: usize,
    cols: usize,
    x: usize,
    y: usize,
) {
    let (name, hint) = if form.entering_new_session_name() {
        (
            format!("{}_", form.name()),
            "Enter: Continue • Blank: Random",
        )
    } else {
        (
            if form.name().is_empty() {
                "<RANDOM>".into()
            } else {
                form.name().to_owned()
            },
            "Ctrl+r: Edit name",
        )
    };
    print_text_with_coordinates(
        Theme.field("New session name:", &name, hint, cols),
        x,
        y + 1,
        Some(cols),
        Some(1),
    );

    if form.entering_layout_search_term() {
        let search = format!("{}_", form.layout_search_term());
        print_text_with_coordinates(
            Theme.field("New session layout:", &search, "Enter: Create", cols),
            x,
            y + 3,
            Some(cols),
            Some(1),
        );
        let table_rows = rows.saturating_sub(8);
        if form.is_searching() && form.selected_layout_info().is_none() {
            print_text_with_coordinates(
                Theme.warning(&truncate_columns(
                    "No matching layouts. Edit or clear the search.",
                    cols,
                )),
                x,
                y + 5,
                Some(cols),
                Some(1),
            );
        } else {
            let mut table = Table::new().add_row(vec!["Layout"]);
            for (layout, indices, selected) in form.layouts_to_render(table_rows) {
                let cell = Theme.layout(
                    layout.name(),
                    layout.is_builtin(),
                    &indices,
                    cols.saturating_sub(4),
                );
                table = table.add_styled_row(vec![if selected { cell.selected() } else { cell }]);
            }
            print_table_with_coordinates(table, x, y + 5, Some(cols), Some(table_rows));
        }
    }

    let folder = form
        .new_session_folder()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<default>".into());
    print_text_with_coordinates(
        Theme.field(
            "New session folder:",
            &folder,
            "Ctrl+f: Choose • Ctrl+c: Clear",
            cols,
        ),
        x,
        y + rows.saturating_sub(2),
        Some(cols),
        Some(1),
    );
}
