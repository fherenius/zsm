//! Character-safe text shortening for the plugin UI.
//!
//! These helpers count characters, never bytes. The plugin renders zoxide paths
//! and session names that may contain multi-byte characters, and slicing a
//! `str` (or calling `String::truncate`) at a byte offset that lands inside a
//! codepoint panics — which traps the whole WASM instance and kills the plugin.

/// Marker inserted where characters were dropped.
const ELLIPSIS: &str = "...";
/// Character width of [`ELLIPSIS`].
const ELLIPSIS_WIDTH: usize = 3;
/// Leading characters [`elide_middle`] keeps before the ellipsis.
const HEAD_WIDTH: usize = 10;

/// Shorten `text` to at most `max_chars` by dropping leading characters and
/// prefixing an ellipsis.
///
/// Used for directory paths, where the tail (the project directory itself)
/// carries more information than the leading path components.
pub fn elide_start(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars <= ELLIPSIS_WIDTH {
        return text.chars().take(max_chars).collect();
    }

    let keep = max_chars - ELLIPSIS_WIDTH;
    let mut shortened = String::from(ELLIPSIS);
    shortened.extend(text.chars().skip(char_count - keep));
    shortened
}

/// Shorten `text` to at most `max_chars` by keeping the leading characters and
/// the tail, with an ellipsis between them.
///
/// Used for session rows, where both the name (at the start) and the directory
/// (at the end) matter.
pub fn elide_middle(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars <= ELLIPSIS_WIDTH {
        return text.chars().take(max_chars).collect();
    }

    let budget = max_chars - ELLIPSIS_WIDTH;
    let head = budget.min(HEAD_WIDTH);
    let tail = budget - head;

    let mut shortened: String = text.chars().take(head).collect();
    shortened.push_str(ELLIPSIS);
    shortened.extend(text.chars().skip(char_count - tail));
    shortened
}

/// Map character indices in `text` onto their positions after [`elide_start`].
///
/// Indices that land in the dropped prefix are discarded. This lives next to
/// `elide_start` so the two cannot drift apart.
pub fn remap_indices_after_elide_start(
    text: &str,
    max_chars: usize,
    indices: &[usize],
) -> Vec<usize> {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return indices.to_vec();
    }
    if max_chars <= ELLIPSIS_WIDTH {
        return indices.iter().copied().filter(|&i| i < max_chars).collect();
    }

    let dropped = char_count - (max_chars - ELLIPSIS_WIDTH);
    indices
        .iter()
        .filter(|&&i| i >= dropped)
        .map(|&i| i - dropped + ELLIPSIS_WIDTH)
        .collect()
}

/// Map character indices in `text` onto their positions after [`elide_middle`].
///
/// Indices that land in the dropped middle are discarded.
pub fn remap_indices_after_elide_middle(
    text: &str,
    max_chars: usize,
    indices: &[usize],
) -> Vec<usize> {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return indices.to_vec();
    }
    if max_chars <= ELLIPSIS_WIDTH {
        return indices.iter().copied().filter(|&i| i < max_chars).collect();
    }

    let budget = max_chars - ELLIPSIS_WIDTH;
    let head = budget.min(HEAD_WIDTH);
    let tail = budget - head;
    let tail_start = char_count - tail;

    indices
        .iter()
        .filter_map(|&i| {
            if i < head {
                Some(i)
            } else if i >= tail_start {
                Some(head + ELLIPSIS_WIDTH + (i - tail_start))
            } else {
                None
            }
        })
        .collect()
}

/// Truncate `text` to at most `max_chars` characters, dropping the tail.
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Truncate `text` to at most `max_bytes` bytes, cutting on a character
/// boundary so the result stays valid UTF-8.
///
/// Session names are limited in *bytes* by the socket path, so a character
/// budget alone is not enough for multi-byte names.
pub fn truncate_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_count(text: &str) -> usize {
        text.chars().count()
    }

    #[test]
    fn short_text_is_returned_unchanged() {
        assert_eq!(elide_start("/tmp/foo", 20), "/tmp/foo");
        assert_eq!(elide_middle("/tmp/foo", 20), "/tmp/foo");
        assert_eq!(elide_start("exact", 5), "exact");
    }

    #[test]
    fn elide_start_keeps_the_tail_at_the_requested_width() {
        assert_eq!(elide_start("/home/user/projects/zsm", 10), "...cts/zsm");
        // The result is exactly max_chars wide.
        assert_eq!(char_count(&elide_start("/home/user/projects/zsm", 12)), 12);
    }

    #[test]
    fn elide_middle_keeps_head_and_tail_at_the_requested_width() {
        assert_eq!(char_count(&elide_middle("a".repeat(80).as_str(), 20)), 20);
        assert_eq!(elide_middle("0123456789abcdefghij", 16), "0123456789...hij");
    }

    /// Regression: the previous implementations sliced at byte offsets, so any
    /// multi-byte character near the cut point panicked and took the plugin
    /// down with it.
    #[test]
    fn multi_byte_text_does_not_panic_and_stays_character_aligned() {
        let cyrillic = "/home/пользователь/проекты/сайт";
        let cjk = "/Users/fester/文書/プロジェクト/設定";
        let emoji = "● сессия (/home/u/🚀🚀🚀/app)";

        for width in 0..40 {
            for text in [cyrillic, cjk, emoji] {
                let start = elide_start(text, width);
                let middle = elide_middle(text, width);
                assert!(char_count(&start) <= width, "{text} @ {width}");
                assert!(char_count(&middle) <= width, "{text} @ {width}");
                assert!(truncate_chars(text, width).chars().count() <= width);
            }
        }
    }

    /// The old code guarded on `max_width > 10` but computed `max_width - 13`,
    /// so widths 11..=13 underflowed.
    #[test]
    fn narrow_widths_do_not_underflow() {
        for width in 0..=14 {
            let out = elide_middle("/home/user/projects/zsm", width);
            assert!(
                out.chars().count() <= width,
                "width {width} produced {out:?}"
            );
        }
        assert_eq!(elide_start("abcdefgh", 0), "");
        assert_eq!(elide_start("abcdefgh", 3), "abc");
        assert_eq!(elide_middle("abcdefgh", 3), "abc");
        assert_eq!(elide_start("abcdefgh", 4), "...h");
    }

    /// Highlight positions must survive elision, otherwise search highlights
    /// land on the wrong characters.
    #[test]
    fn remapped_indices_point_at_the_same_characters() {
        let path = "/home/user/projects/zsm";

        for width in 0..40 {
            let shortened = elide_start(path, width);
            let all: Vec<usize> = (0..path.chars().count()).collect();
            let remapped = remap_indices_after_elide_start(path, width, &all);
            let source: Vec<char> = path.chars().collect();
            let rendered: Vec<char> = shortened.chars().collect();

            // Every surviving index must still address a character in range,
            // and it must be the very same character.
            let kept: Vec<usize> = all
                .iter()
                .copied()
                .filter(|&i| remap_indices_after_elide_start(path, width, &[i]).len() == 1)
                .collect();
            for (slot, original) in remapped.iter().zip(kept.iter()) {
                assert!(*slot < rendered.len(), "width {width}: {slot} out of range");
                assert_eq!(
                    rendered[*slot], source[*original],
                    "width {width}: index {original} mapped to {slot}"
                );
            }
        }
    }

    #[test]
    fn remapped_middle_indices_point_at_the_same_characters() {
        let row = "\u{25cf} session (/home/user/projects/zsm)";
        let source: Vec<char> = row.chars().collect();

        for width in 0..50 {
            let shortened = elide_middle(row, width);
            let rendered: Vec<char> = shortened.chars().collect();
            for (original, expected) in source.iter().enumerate() {
                let mapped = remap_indices_after_elide_middle(row, width, &[original]);
                if let Some(&slot) = mapped.first() {
                    assert!(slot < rendered.len(), "width {width}: {slot} out of range");
                    assert_eq!(
                        rendered[slot], *expected,
                        "width {width}: index {original} mapped to {slot}"
                    );
                }
            }
        }
    }

    #[test]
    fn byte_truncation_cuts_on_a_character_boundary() {
        assert_eq!(truncate_bytes("abcdef", 3), "abc");
        assert_eq!(truncate_bytes("abc", 10), "abc");

        // Each of these is 2 bytes, so a 3-byte budget fits only one.
        let two_byte = "\u{00e9}\u{00e9}\u{00e9}";
        assert_eq!(truncate_bytes(two_byte, 3), "\u{00e9}");
        assert_eq!(truncate_bytes(two_byte, 4), "\u{00e9}\u{00e9}");

        // A budget smaller than the first character yields nothing rather
        // than splitting it.
        assert_eq!(truncate_bytes("\u{1f680}", 3), "");

        for budget in 0..40 {
            let out = truncate_bytes("/home/\u{043f}\u{0440}/\u{1f680}app", budget);
            assert!(out.len() <= budget);
        }
    }
}
