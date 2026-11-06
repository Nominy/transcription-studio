//! JavaScript string semantics the Helper linter relies on: UTF-16 code-unit indexing,
//! JS `\s`, Unicode general categories, generic `<…>`/`{…}`/`[…]` tag ranges and
//! `text-context.ts` tokenization.

use std::sync::LazyLock;

use regex::Regex;

/// A row's text as JavaScript sees it: indexes are UTF-16 code units.
pub(super) struct Text<'a> {
    pub(super) source: &'a str,
    pub(super) units: Vec<u16>,
}

impl<'a> Text<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        Self {
            source,
            units: source.encode_utf16().collect(),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.units.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// `text[index] === character` for a BMP `character`.
    pub(super) fn is(&self, index: usize, character: char) -> bool {
        self.units
            .get(index)
            .is_some_and(|&unit| u32::from(unit) == u32::from(character))
    }

    /// `text[index]` tested by a one-character pattern: a lone surrogate half reads as
    /// U+FFFD, which no pattern the linter uses matches.
    pub(super) fn char_at(&self, index: usize) -> Option<char> {
        self.units.get(index).map(|&unit| unit_char(unit))
    }

    /// The code point starting at `index` (`codePointAt`), as `u`-flag regexes read it.
    pub(super) fn code_point_at(&self, index: usize) -> Option<char> {
        let unit = *self.units.get(index)?;
        let pair = self
            .units
            .get(index..index + 2)
            .unwrap_or(&self.units[index..]);
        Some(
            char::decode_utf16(pair.iter().copied())
                .next()
                .and_then(Result::ok)
                .unwrap_or(unit_char(unit)),
        )
    }

    /// `text.startsWith(needle, index)`.
    pub(super) fn starts_with_at(&self, index: usize, needle: &str) -> bool {
        (index..)
            .zip(needle.encode_utf16())
            .all(|(position, unit)| self.units.get(position) == Some(&unit))
    }

    /// `text.indexOf(needle, from)`.
    pub(super) fn index_of(&self, needle: &str, from: usize) -> Option<usize> {
        (from..self.len()).find(|&index| self.starts_with_at(index, needle))
    }

    /// `text.indexOf(character, from)` for a BMP `character`.
    pub(super) fn index_of_char(&self, character: char, from: usize) -> Option<usize> {
        (from..self.len()).find(|&index| self.is(index, character))
    }

    /// `text.lastIndexOf(character, at_or_before)` for a BMP `character`.
    pub(super) fn last_index_of_char(&self, character: char, at_or_before: usize) -> Option<usize> {
        let end = at_or_before.saturating_add(1).min(self.len());
        (0..end).rev().find(|&index| self.is(index, character))
    }

    /// `text.slice(start, end)`; split surrogate pairs decode to U+FFFD.
    pub(super) fn slice(&self, start: usize, end: usize) -> String {
        String::from_utf16_lossy(&self.units[start..end])
    }

    /// The UTF-16 index of a byte offset into `source`.
    pub(super) fn unit_index(&self, byte: usize) -> usize {
        self.source[..byte].encode_utf16().count()
    }
}

fn unit_char(unit: u16) -> char {
    char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// JS `\s` and the `trim()` set (Rust's `char::is_whitespace` adds U+0085 and lacks U+FEFF).
pub(super) fn is_js_space(character: char) -> bool {
    matches!(
        character,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `[ \t]`.
pub(super) fn is_blank(character: Option<char>) -> bool {
    matches!(character, Some(' ' | '\t'))
}

static LETTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{L}").expect("valid regex"));
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{N}").expect("valid regex"));
static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{M}").expect("valid regex"));
static CAPITAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{Lu}\p{Lt}]").expect("valid regex"));

fn in_class(class: &Regex, character: char) -> bool {
    class.is_match(character.encode_utf8(&mut [0; 4]))
}

/// `\p{L}`.
pub(super) fn is_letter(character: char) -> bool {
    if character.is_ascii() {
        character.is_ascii_alphabetic()
    } else {
        in_class(&LETTER, character)
    }
}

/// `\p{N}`.
pub(super) fn is_number(character: char) -> bool {
    if character.is_ascii() {
        character.is_ascii_digit()
    } else {
        in_class(&NUMBER, character)
    }
}

/// `[\p{L}\p{N}\p{M}]`.
pub(super) fn is_letter_number_mark(character: char) -> bool {
    is_letter(character)
        || is_number(character)
        || (!character.is_ascii() && in_class(&MARK, character))
}

/// `[\p{L}\p{N}\p{M}_]`.
pub(super) fn is_word_character(character: char) -> bool {
    character == '_' || is_letter_number_mark(character)
}

/// `[\p{Lu}\p{Lt}]`.
pub(super) fn is_capital(character: char) -> bool {
    if character.is_ascii() {
        character.is_ascii_uppercase()
    } else {
        in_class(&CAPITAL, character)
    }
}

/// `char.toLocaleLowerCase()` / `toLocaleUpperCase()` of a one-character string.
fn lowercase_is_identity(character: char) -> bool {
    let mut lower = character.to_lowercase();
    lower.next() == Some(character) && lower.next().is_none()
}

fn uppercase_is_identity(character: char) -> bool {
    let mut upper = character.to_uppercase();
    upper.next() == Some(character) && upper.next().is_none()
}

/// page.ts `isLowercaseLetter`.
pub(super) fn is_lowercase_letter(character: Option<char>) -> bool {
    character.is_some_and(|character| {
        is_letter(character)
            && lowercase_is_identity(character)
            && !uppercase_is_identity(character)
    })
}

/// The double-quote variants page.ts normalizes to `"` (`UNICODE_DOUBLE_QUOTE_PATTERN`).
pub(super) fn is_unicode_double_quote(character: char) -> bool {
    matches!(
        character,
        '\u{00AB}'
            | '\u{00BB}'
            | '\u{201C}'
            | '\u{201D}'
            | '\u{201E}'
            | '\u{201F}'
            | '\u{2039}'
            | '\u{203A}'
            | '\u{275D}'
            | '\u{275E}'
            | '\u{300C}'
            | '\u{300D}'
            | '\u{300E}'
            | '\u{300F}'
            | '\u{301D}'
            | '\u{301E}'
            | '\u{301F}'
            | '\u{FF02}'
    )
}

/// `"` after `normalizeUnicodeDoubleQuoteVariants`.
pub(super) fn is_double_quote(character: Option<char>) -> bool {
    character.is_some_and(|character| character == '"' || is_unicode_double_quote(character))
}

/// `isInsideOpenQuoteAt`: an odd number of double quotes precedes `index`.
pub(super) fn is_inside_open_quote(text: &Text<'_>, index: usize) -> bool {
    (0..index.min(text.len()))
        .filter(|&pointer| is_double_quote(text.char_at(pointer)))
        .count()
        % 2
        == 1
}

const GENERIC_TAG_DELIMITERS: [(char, char); 3] = [('<', '>'), ('{', '}'), ('[', ']')];

/// text-context.ts `getEnclosingGenericTagRange`: the `[start, end)` of the generic tag
/// around `index`.
pub(super) fn enclosing_generic_tag(text: &Text<'_>, index: usize) -> Option<(usize, usize)> {
    if index >= text.len() {
        return None;
    }
    GENERIC_TAG_DELIMITERS.iter().find_map(|&(open, close)| {
        let open_index = text.last_index_of_char(open, index)?;
        let close_before = index
            .checked_sub(1)
            .and_then(|before| text.last_index_of_char(close, before));
        if close_before.is_some_and(|close_before| close_before > open_index) {
            return None;
        }
        let close_index = text.index_of_char(close, index)?;
        Some((open_index, close_index + 1))
    })
}

/// text-context.ts `isRangeInsideGenericTag`.
pub(super) fn is_range_inside_generic_tag(text: &Text<'_>, start: usize, end: usize) -> bool {
    enclosing_generic_tag(text, start).is_some_and(|(_, tag_end)| end <= tag_end)
}

/// page.ts `skipLeadingCapitalizationTokens`: skips whitespace and whole `[…]`, `{…}`,
/// `<…>` tokens.
pub(super) fn skip_leading_capitalization_tokens(text: &Text<'_>, start: usize) -> usize {
    let mut index = start;
    while let Some(character) = text.char_at(index) {
        if is_js_space(character) {
            index += 1;
            continue;
        }
        let close = match character {
            '[' => ']',
            '{' => '}',
            '<' => '>',
            _ => break,
        };
        let Some(close_index) = text.index_of_char(close, index + 1) else {
            break;
        };
        index = close_index + 1;
    }
    index
}

/// page.ts `findFirstLetterIndex`: the first letter from `start`, unless a number comes first.
pub(super) fn first_letter_index(text: &Text<'_>, start: usize) -> Option<usize> {
    for index in start..text.len() {
        let character = text.char_at(index)?;
        if is_letter(character) {
            return Some(index);
        }
        if is_number(character) {
            return None;
        }
    }
    None
}

/// The length of `text.slice(0, end).trimEnd()`.
pub(super) fn trim_end(text: &Text<'_>, end: usize) -> usize {
    let mut end = end;
    while end > 0 && text.char_at(end - 1).is_some_and(is_js_space) {
        end -= 1;
    }
    end
}

/// page.ts `stripTrailingTagTokens`: the length of the text before its trailing tags.
pub(super) fn strip_trailing_tag_tokens(text: &Text<'_>) -> usize {
    let mut end = trim_end(text, text.len());
    while end > 0 {
        let opener = match text.char_at(end - 1) {
            Some(']') => '[',
            Some('}') => '{',
            Some('>') => '<',
            _ => break,
        };
        let Some(open_index) = text.last_index_of_char(opener, end - 1) else {
            break;
        };
        end = trim_end(text, open_index);
    }
    end
}

/// `skipBlanks`: the first index from `index` that is not `[ \t]`.
pub(super) fn skip_blanks(text: &Text<'_>, index: usize) -> usize {
    let mut index = index;
    while is_blank(text.char_at(index)) {
        index += 1;
    }
    index
}

/// `/<pattern>/gu` over `open [^open close \r\n]* close`: every non-overlapping delimited
/// tag, as `[start, end)`.
pub(super) fn delimited_tags(text: &Text<'_>, open: char, close: char) -> Vec<(usize, usize)> {
    let mut tags = Vec::new();
    let mut search = 0;
    while let Some(start) = text.index_of_char(open, search) {
        let stop = (start + 1..text.len()).find(|&index| {
            matches!(text.char_at(index), Some(character) if character == open || character == close || character == '\r' || character == '\n')
        });
        match stop {
            Some(stop) if text.is(stop, close) => {
                tags.push((start, stop + 1));
                search = stop + 1;
            }
            _ => search = start + 1,
        }
    }
    tags
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TokenKind {
    Word,
    Space,
    Tag,
    Punctuation,
    Symbol,
}

#[derive(Clone, Copy)]
pub(super) struct Token {
    pub(super) kind: TokenKind,
    pub(super) start: usize,
    pub(super) end: usize,
}

impl Token {
    pub(super) fn len(self) -> usize {
        self.end - self.start
    }
}

fn is_token_word_character(character: Option<char>) -> bool {
    character.is_some_and(is_word_character)
}

/// text-context.ts `tokenizeTranscriptText`.
pub(super) fn tokenize(text: &Text<'_>) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while let Some(character) = text.char_at(index) {
        let start = index;
        let generic_close = GENERIC_TAG_DELIMITERS
            .iter()
            .find(|&&(open, _)| open == character)
            .and_then(|&(_, close)| text.index_of_char(close, index + 1));
        let kind = if let Some(close_index) = generic_close {
            index = close_index + 1;
            TokenKind::Tag
        } else if is_js_space(character) {
            while text.char_at(index).is_some_and(is_js_space) {
                index += 1;
            }
            TokenKind::Space
        } else if is_word_character(character) {
            while is_word_token_continuation(text, index) {
                index += 1;
            }
            TokenKind::Word
        } else {
            index += 1;
            if matches!(
                character,
                '.' | ',' | '?' | '!' | ':' | ';' | '"' | '\'' | '(' | ')' | '-'
            ) {
                TokenKind::Punctuation
            } else {
                TokenKind::Symbol
            }
        };
        tokens.push(Token {
            kind,
            start,
            end: index,
        });
    }
    tokens
}

fn is_word_token_continuation(text: &Text<'_>, index: usize) -> bool {
    is_token_word_character(text.char_at(index))
        || (text.is(index, '-')
            && index > 0
            && is_token_word_character(text.char_at(index - 1))
            && is_token_word_character(text.char_at(index + 1)))
}
