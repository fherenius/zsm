//! Terminal-column-aware text fitting with grapheme-safe cuts. Highlight
//! indices remain Unicode scalar positions, as required by Zellij Text.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const ELLIPSIS: &str = "...";
const ELLIPSIS_WIDTH: usize = 3;
const HEAD_WIDTH: usize = 10;

pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

// All offsets below come from grapheme_indices, so slices cannot split either
// a UTF-8 codepoint or a combining/emoji sequence.
fn prefix_end(text: &str, columns: usize) -> usize {
    if columns == 0 {
        return 0;
    }
    let mut width = 0;
    let mut end = 0;
    for (offset, grapheme) in text.grapheme_indices(true) {
        width += display_width(grapheme);
        if width > columns {
            break;
        }
        end = offset + grapheme.len();
    }
    end
}

pub fn truncate_columns(text: &str, columns: usize) -> String {
    text[..prefix_end(text, columns)].to_owned()
}

struct Elision<'a> {
    source: &'a str,
    head_end: usize,
    tail_start: usize,
    marker: &'static str,
}

impl<'a> Elision<'a> {
    fn new(text: &'a str, columns: usize, keep_head: bool) -> Self {
        if columns == 0 {
            return Self {
                source: text,
                head_end: 0,
                tail_start: text.len(),
                marker: "",
            };
        }
        if display_width(text) <= columns {
            return Self {
                source: text,
                head_end: text.len(),
                tail_start: text.len(),
                marker: "",
            };
        }
        if columns <= ELLIPSIS_WIDTH {
            return Self {
                source: text,
                head_end: prefix_end(text, columns),
                tail_start: text.len(),
                marker: "",
            };
        }
        let budget = columns - ELLIPSIS_WIDTH;
        let head_end = if keep_head {
            prefix_end(text, budget.min(HEAD_WIDTH))
        } else {
            0
        };
        let tail_budget = budget - display_width(&text[..head_end]);
        let mut tail_start = text.len();
        let mut tail_width = 0;
        for (offset, grapheme) in text.grapheme_indices(true).rev() {
            if offset < head_end {
                break;
            }
            tail_width += display_width(grapheme);
            if tail_width > tail_budget {
                break;
            }
            tail_start = offset;
        }
        Self {
            source: text,
            head_end,
            tail_start,
            marker: ELLIPSIS,
        }
    }

    fn render(&self) -> String {
        format!(
            "{}{}{}",
            &self.source[..self.head_end],
            self.marker,
            &self.source[self.tail_start..]
        )
    }

    fn remap(&self, indices: &[usize]) -> Vec<usize> {
        let head_chars = self.source[..self.head_end].chars().count();
        let tail_char_start = self.source[..self.tail_start].chars().count();
        let source_chars = self.source.chars().count();
        indices
            .iter()
            .filter_map(|&index| {
                if index >= source_chars {
                    None
                } else if index < head_chars {
                    Some(index)
                } else if index >= tail_char_start {
                    Some(head_chars + self.marker.chars().count() + index - tail_char_start)
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Keep the tail of a directory path within a terminal-column budget.
pub fn elide_start(text: &str, columns: usize) -> String {
    Elision::new(text, columns, false).render()
}

/// Keep both the leading session name/marker and the tail of its path.
pub fn elide_middle(text: &str, columns: usize) -> String {
    Elision::new(text, columns, true).render()
}

pub fn remap_indices_after_elide_start(
    text: &str,
    columns: usize,
    indices: &[usize],
) -> Vec<usize> {
    Elision::new(text, columns, false).remap(indices)
}

pub fn remap_indices_after_elide_middle(
    text: &str,
    columns: usize,
    indices: &[usize],
) -> Vec<usize> {
    Elision::new(text, columns, true).remap(indices)
}

/// Wrap messages without splitting a grapheme. An individual cluster wider
/// than the whole pane is omitted because it cannot be displayed intact.
pub fn wrap_columns(text: &str, columns: usize) -> Vec<String> {
    if columns == 0 {
        return vec![];
    }
    let mut lines = vec![];
    let mut line = String::new();
    let mut width = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = display_width(grapheme);
        if grapheme == "\n" || width + grapheme_width > columns {
            lines.push(std::mem::take(&mut line));
            width = 0;
        }
        if grapheme != "\n" && grapheme_width <= columns {
            line.push_str(grapheme);
            width += grapheme_width;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
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
    fn wide_and_combining_graphemes_fit_and_highlights_stay_on_original_characters() {
        let samples = [
            "/work/日本語/設定",
            "e\u{301}e\u{301}e\u{301}",
            "★ 👩🏽‍💻 project /🇳🇱/👨‍👩‍👧‍👦/a",
            "x/❤️/1️⃣/end",
        ];
        for sample in samples {
            let source: Vec<_> = sample.chars().collect();
            let clusters: Vec<_> = sample.graphemes(true).collect();
            for width in 0..50 {
                for middle in [false, true] {
                    let plan = Elision::new(sample, width, middle);
                    let rendered = plan.render();
                    assert!(
                        display_width(&rendered) <= width,
                        "{rendered:?} exceeds {width}"
                    );
                    // Every retained piece consists only of whole source clusters.
                    for part in [&sample[..plan.head_end], &sample[plan.tail_start..]] {
                        assert!(part
                            .graphemes(true)
                            .all(|cluster| clusters.contains(&cluster)));
                    }
                    let chars: Vec<_> = rendered.chars().collect();
                    for (index, expected) in source.iter().enumerate() {
                        if let Some(&mapped) = plan.remap(&[index]).first() {
                            assert_eq!(chars[mapped], *expected);
                        }
                    }
                }
                assert!(wrap_columns(sample, width)
                    .iter()
                    .all(|line| display_width(line) <= width));
            }
        }
        assert_eq!(truncate_columns("日本", 3), "日");
        assert_eq!(truncate_columns("e\u{301}x", 1), "e\u{301}");
        assert_eq!(truncate_columns("👩🏽‍💻x", 2), "👩🏽‍💻");
        assert_eq!(truncate_columns("👩🏽‍💻x", 1), "");
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
