//! Soft wrapping of row text, shared by the static row text, the row-height measurement and
//! the ghost-caret line lookup.
//!
//! It reproduces the row editor's wrapping (gpui-component's multi-line input with soft wrap):
//! per hard line, the longest prefix of whole grapheme clusters whose *shaped* width fits is
//! found by exponential then binary search, the break moves back to the last Unicode (UAX #14)
//! line-break opportunity inside it, an unbreakable run falls back to grapheme boundaries, and
//! continuation rows keep the hard line's leading-space indent. gpui's plain text wrapping
//! (used by text elements) picks other break points — it sums per-char advances and breaks
//! only at spaces and before non-word characters (`<`, `>`, `?`, `—`, …), never after a
//! hyphen — so static text is painted with these breaks made explicit and the row keeps its
//! line breaks and height when the editor mounts or unmounts.

use std::ops::Range;

use gpui_kit::Pixels;
use unicode_segmentation::UnicodeSegmentation as _;

/// Longest leading-space indent continuation rows keep (gpui's `LineWrapper::MAX_INDENT`).
const MAX_INDENT: usize = 256;

/// One visual row of a wrapped text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VisualLine {
    /// Bytes of the text on this row (never the hard newline).
    pub range: Range<usize>,
    /// Leading spaces of the hard line this continuation row is indented by (zero for the
    /// first row of a hard line).
    pub indent: usize,
}

/// A text broken into the visual rows the row editor shows at a wrap width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WrappedText {
    lines: Vec<VisualLine>,
}

impl WrappedText {
    /// Wraps `text` at `width`; `measure` returns the shaped width of a single-line string in
    /// the row font (the editor measures with `layout_line` of the same font and size).
    pub fn new(text: &str, width: Pixels, mut measure: impl FnMut(&str) -> Pixels) -> Self {
        let mut lines = Vec::new();
        let mut line_start = 0;
        for line in text.split('\n') {
            let (breaks, indent) = soft_breaks(line, width, &mut measure);
            let mut start = 0;
            for end in breaks.into_iter().chain([line.len()]) {
                lines.push(VisualLine {
                    range: line_start + start..line_start + end,
                    indent: if start == 0 { 0 } else { indent },
                });
                start = end;
            }
            line_start += line.len() + 1;
        }
        Self { lines }
    }

    /// Number of visual rows (at least one: an empty text is one empty row).
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Visual row holding the caret at byte `byte`: a caret on a soft break sits at the start
    /// of the next row, a caret at the end of a hard line on its last row.
    pub fn line_of_byte(&self, byte: usize) -> usize {
        let last = self.lines.len().saturating_sub(1);
        (0..last)
            .find(|&index| {
                let end = self.lines[index].range.end;
                byte < end || (byte == end && self.lines[index + 1].range.start != end)
            })
            .unwrap_or(last)
    }

    /// The text with every soft break made a hard one: a newline plus the row's indent is
    /// inserted where a continuation row starts, so an unwrapped text element paints exactly
    /// the editor's rows.
    pub fn display_text(&self, text: &str) -> String {
        let extra: usize = self.soft_rows().map(|line| 1 + line.indent).sum();
        let mut display = String::with_capacity(text.len() + extra);
        let mut copied = 0;
        for line in self.soft_rows() {
            display.push_str(&text[copied..line.range.start]);
            display.push('\n');
            display.extend(std::iter::repeat_n(' ', line.indent));
            copied = line.range.start;
        }
        display.push_str(&text[copied..]);
        display
    }

    /// Byte in [`Self::display_text`] of text byte `byte`; a byte on a soft break maps to the
    /// start of the next row.
    pub fn to_display(&self, byte: usize) -> usize {
        byte + self
            .soft_rows()
            .take_while(|line| line.range.start <= byte)
            .map(|line| 1 + line.indent)
            .sum::<usize>()
    }

    /// Display range of the text range `range` (an end on a soft break stays on the earlier
    /// row).
    pub fn to_display_range(&self, range: Range<usize>) -> Range<usize> {
        let end = range.end
            + self
                .soft_rows()
                .take_while(|line| line.range.start < range.end)
                .map(|line| 1 + line.indent)
                .sum::<usize>();
        self.to_display(range.start).min(end)..end
    }

    /// Text byte of display byte `display`; the inserted newline and indent map to the start
    /// of the row they introduce.
    pub fn to_source(&self, display: usize) -> usize {
        let mut shift = 0;
        for line in self.soft_rows() {
            let inserted_at = line.range.start + shift;
            if display < inserted_at {
                break;
            }
            let inserted = 1 + line.indent;
            if display < inserted_at + inserted {
                return line.range.start;
            }
            shift += inserted;
        }
        display - shift
    }

    /// Continuation rows (each starts at a soft break).
    fn soft_rows(&self) -> impl Iterator<Item = &VisualLine> {
        self.lines
            .windows(2)
            .filter(|pair| pair[0].range.end == pair[1].range.start)
            .map(|pair| &pair[1])
    }
}

/// Soft-break byte offsets of one hard line (no newline) and its leading-space indent.
fn soft_breaks(
    line: &str,
    width: Pixels,
    measure: &mut impl FnMut(&str) -> Pixels,
) -> (Vec<usize>, usize) {
    let indent = line
        .bytes()
        .take_while(|&byte| byte == b' ')
        .count()
        .min(MAX_INDENT);
    let indent_width = measure(&line[..indent]);
    let ends: Vec<usize> = line
        .grapheme_indices(true)
        .map(|(index, grapheme)| index + grapheme.len())
        .collect();
    let opportunities: Vec<usize> = unicode_linebreak::linebreaks(line)
        .map(|(index, _)| index)
        .filter(|index| ends.binary_search(index).is_ok())
        .collect();
    let mut breaks = Vec::new();
    let mut first = 0;
    let mut start = 0;
    while first < ends.len() {
        let available = if start == 0 {
            width
        } else {
            width - indent_width
        };
        // Exponential probing, then binary search, over whole graphemes (the editor's search;
        // the same probes keep the result identical even for non-monotonic shaping).
        let remaining = ends.len() - first;
        let mut low = 0;
        let mut high = 1;
        while measure(&line[start..ends[first + high - 1]]) <= available {
            low = high;
            if high == remaining {
                break;
            }
            high = (high * 2).min(remaining);
        }
        while low + 1 < high {
            let mid = (low + high) / 2;
            if measure(&line[start..ends[first + mid - 1]]) <= available {
                low = mid;
            } else {
                high = mid;
            }
        }
        // A grapheme wider than the row still takes a row of its own.
        let fitting_end = ends[first + low.max(1) - 1];
        if fitting_end == line.len() {
            break;
        }
        let candidate = opportunities.partition_point(|&index| index <= fitting_end);
        let end = candidate
            .checked_sub(1)
            .map(|index| opportunities[index])
            .filter(|&index| index > start && (start != 0 || index > indent))
            .unwrap_or(fitting_end);
        breaks.push(end);
        first = ends.partition_point(|&index| index <= end);
        start = end;
    }
    (breaks, indent)
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;
    use unicode_segmentation::UnicodeSegmentation as _;

    use super::WrappedText;

    /// One unit per grapheme (the editor's own wrapping tests use this measure).
    fn graphemes(text: &str) -> gpui_kit::Pixels {
        px(text.graphemes(true).count() as f32)
    }

    /// A proportional font: narrow spaces and punctuation, wide CJK.
    fn proportional(text: &str) -> gpui_kit::Pixels {
        px(text
            .chars()
            .map(|c| match c {
                ' ' | '.' | ',' | ':' | ';' | '!' | 'i' | 'l' | '\'' => 3.,
                c if c > '\u{2E80}' => 14.,
                _ => 7.,
            })
            .sum())
    }

    fn rows(text: &str, width: f32) -> Vec<String> {
        let wrap = WrappedText::new(text, px(width), graphemes);
        wrap.display_text(text)
            .split('\n')
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn breaks_where_the_row_editor_breaks() {
        assert_eq!(rows("hello world", 8.), ["hello ", "world"]);
        assert_eq!(rows("a👩‍💻b", 2.), ["a👩‍💻", "b"]);
        // Continuation rows keep the leading-space indent and wrap in the narrower space.
        assert_eq!(rows("  abcdefgh", 5.), ["  abc", "  def", "  gh"]);
        // A row narrower than one grapheme still shows one grapheme per row.
        assert_eq!(rows("abc", 0.), ["a", "b", "c"]);
        // Closing punctuation stays with the preceding character.
        assert_eq!(rows("你好，世界", 2.), ["你", "好，", "世界"]);
        assert_eq!(rows("", 10.), [""]);
    }

    #[test]
    fn breaks_after_hyphens_and_inside_tags_like_the_editor() {
        // gpui's text wrapping keeps `как-то` and `смех-в-речи` whole; the editor (UAX #14)
        // may break after a hyphen.
        assert_eq!(rows("ab как-то", 7.), ["ab как-", "то"]);
        assert_eq!(rows("ab <смех-в-речи>", 12.), ["ab <смех-в-", "речи>"]);
        // No break after an opening brace; the tag moves as a whole word.
        assert_eq!(rows("ab {СКАЗ: десять}", 8.), ["ab ", "{СКАЗ: ", "десять}"]);
        // Double spaces stay at the end of the row they follow.
        assert_eq!(rows("ab  cd  ef", 5.), ["ab  ", "cd  ", "ef"]);
        // An unbreakable token falls back to grapheme boundaries.
        assert_eq!(rows("ab абвгдежзий", 5.), ["ab ", "абвгд", "ежзий"]);
        // Hard line breaks (and a trailing one) are rows of their own.
        assert_eq!(rows("ab\n\ncd\n", 10.), ["ab", "", "cd", ""]);
    }

    /// Static text (the display text), the measured height (row count) and the caret line all
    /// come from one wrap: on the demo transcript and edge cases, at many widths, the display
    /// text paints exactly the wrapped rows, every row fits, and offsets map both ways.
    #[test]
    fn static_text_rows_height_and_caret_lines_agree() {
        let project = studio_editor::load_sample_project().expect("demo project");
        let mut corpus: Vec<String> = project
            .segments
            .iter()
            .map(|segment| segment.text.clone())
            .collect();
        corpus.extend(
            [
                "a  b  c  d  e  f",
                "line one\nline two\n",
                "\n",
                "   indented text that wraps onto continuation rows",
                "{СКАЗ: десять процентов} <смех-в-речи> т-- п-- как-то, что-то",
                "щ👩‍💻щ 你好，世界 word",
            ]
            .map(str::to_string),
        );
        corpus.push("ы".repeat(300));
        assert!(corpus.len() > 10, "demo transcript should load");

        for text in &corpus {
            for width in [1., 20., 41., 77., 150., 333., 512., 900.] {
                let wrap = WrappedText::new(text, px(width), proportional);
                let display = wrap.display_text(text);
                let rows: Vec<&str> = display.split('\n').collect();
                assert_eq!(rows.len(), wrap.line_count(), "{text:?} at {width}");
                for row in &rows {
                    assert!(
                        proportional(row) <= px(width)
                            || row.trim_start_matches(' ').graphemes(true).count() <= 1,
                        "row {row:?} overflows {width}"
                    );
                }
                for (byte, char) in text.char_indices().chain([(text.len(), '\0')]) {
                    let shown = wrap.to_display(byte);
                    assert_eq!(wrap.to_source(shown), byte, "{text:?} at {width}");
                    if char != '\0' {
                        assert!(display[shown..].starts_with(char));
                    }
                    assert_eq!(
                        wrap.line_of_byte(byte),
                        display[..shown].matches('\n').count(),
                        "caret line of byte {byte} in {text:?} at {width}"
                    );
                }
            }
        }
    }
}
