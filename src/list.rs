//! Shared scrolling maths for the plugin's two scrollable lists.

/// Pick the half-open range of rows to draw for a list of `len` items in a
/// table `rows` high, keeping `selected` visible.
///
/// One row is reserved for the table header. The window is centred on the
/// selection and then slid back inside the list, so scrolling to either end
/// still fills the pane instead of trailing off into blank rows.
pub fn visible_range(rows: usize, len: usize, selected: Option<usize>) -> (usize, usize) {
    let row_count = rows.saturating_sub(1); // 1 for the header
    if row_count >= len {
        return (0, len);
    }

    let first = selected
        .unwrap_or(0)
        .min(len.saturating_sub(1))
        .saturating_sub(row_count / 2)
        .min(len - row_count);

    (first, first + row_count)
}

/// Move `selected` one row down in a list of `len` items, wrapping at the end.
///
/// A selection that is already past the end (the list shrank underneath it)
/// wraps to the top rather than drifting further out of range.
pub fn select_next(selected: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }

    Some(match selected {
        Some(selected) if selected >= len - 1 => 0,
        Some(selected) => selected + 1,
        None => 0,
    })
}

/// Move `selected` one row up in a list of `len` items, wrapping at the top.
///
/// An out-of-range selection is pulled back to the last row.
pub fn select_previous(selected: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }

    Some(match selected {
        Some(0) | None => len - 1,
        Some(selected) => (selected - 1).min(len - 1),
    })
}

/// Pull `selected` back inside a list of `len` items.
pub fn clamp_selection(selected: Option<usize>, len: usize) -> Option<usize> {
    match (selected, len) {
        (_, 0) => None,
        (Some(selected), len) => Some(selected.min(len - 1)),
        (None, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lists_render_whole() {
        assert_eq!(visible_range(10, 0, None), (0, 0));
        assert_eq!(visible_range(10, 5, Some(2)), (0, 5));
        // rows includes the header, so 10 rows shows 9 items.
        assert_eq!(visible_range(10, 9, Some(8)), (0, 9));
    }

    #[test]
    fn window_is_centred_on_the_selection() {
        assert_eq!(visible_range(11, 100, Some(50)), (45, 55));
        assert_eq!(visible_range(11, 100, Some(0)), (0, 10));
    }

    /// The old implementation let the window run past the end, so scrolling to
    /// the bottom of a long list left most of the pane blank.
    #[test]
    fn window_stays_inside_the_list() {
        let (first, last) = visible_range(11, 100, Some(99));
        assert_eq!((first, last), (90, 100));

        for len in 1..40 {
            for rows in 1..20 {
                for selected in 0..len {
                    let (first, last) = visible_range(rows, len, Some(selected));
                    assert!(last <= len, "rows {rows} len {len} sel {selected}");
                    assert!(first <= last);
                    if rows.saturating_sub(1) < len {
                        assert_eq!(last - first, rows - 1, "window should fill the pane");
                    }
                }
            }
        }
    }

    /// A selection left over from a longer list must not push the window past
    /// the end, which used to render an entirely empty table.
    #[test]
    fn stale_selection_does_not_blank_the_window() {
        let (first, last) = visible_range(11, 30, Some(400));
        assert!(last <= 30);
        assert_eq!(last - first, 10);
    }

    #[test]
    fn selection_wraps_at_both_ends() {
        assert_eq!(select_next(None, 5), Some(0));
        assert_eq!(select_next(Some(3), 5), Some(4));
        assert_eq!(select_next(Some(4), 5), Some(0));
        assert_eq!(select_previous(None, 5), Some(4));
        assert_eq!(select_previous(Some(0), 5), Some(4));
        assert_eq!(select_previous(Some(3), 5), Some(2));
        assert_eq!(select_next(Some(0), 0), None);
        assert_eq!(select_previous(Some(0), 0), None);
    }

    /// Regression: `move_selection_down` compared `== len - 1`, so a stale
    /// index incremented forever instead of wrapping, and the list stayed
    /// unusable until the user pressed Up enough times to walk back into range.
    #[test]
    fn stale_selection_wraps_instead_of_running_away() {
        assert_eq!(select_next(Some(40), 10), Some(0));
        assert_eq!(select_previous(Some(40), 10), Some(9));
        assert_eq!(clamp_selection(Some(40), 10), Some(9));
        assert_eq!(clamp_selection(Some(3), 10), Some(3));
        assert_eq!(clamp_selection(Some(3), 0), None);
        assert_eq!(clamp_selection(None, 10), None);
    }
}
