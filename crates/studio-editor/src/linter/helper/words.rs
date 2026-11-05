//! Word-level rules: stutters, interjection spellings, unnecessary ё and highlighted words.

use regex::{Regex, RegexSet};
use unicode_normalization::UnicodeNormalization;

use super::text::{
    Text, Token, TokenKind, is_capital, is_js_space, is_letter, is_letter_number_mark,
    is_range_inside_generic_tag, is_word_character,
};
use super::{COMPACTED_MATCH_LIMIT, Row};

/// Case-insensitive literal patterns (`giu` regexes) with a cheap any-pattern prefilter.
pub(super) struct Literals {
    set: RegexSet,
    patterns: Vec<Regex>,
}

impl Literals {
    fn new(sources: Vec<String>) -> Self {
        let sources = sources
            .into_iter()
            .map(|source| format!("(?i){source}"))
            .collect::<Vec<_>>();
        Self {
            set: RegexSet::new(&sources).expect("valid literal patterns"),
            patterns: sources
                .iter()
                .map(|source| Regex::new(source).expect("valid literal pattern"))
                .collect(),
        }
    }

    /// The patterns occurring anywhere in `source`.
    fn occurring<'s>(&'s self, source: &str) -> impl Iterator<Item = &'s Regex> {
        self.set
            .matches(source)
            .into_iter()
            .map(|index| &self.patterns[index])
    }

    /// page.ts `INTERJECTION_CORRECTIONS`: each variant spelled verbatim.
    pub(super) fn interjections<'v>(variants: impl IntoIterator<Item = &'v str>) -> Self {
        Self::new(variants.into_iter().map(regex::escape).collect())
    }

    /// page.ts `getHighlightedWordPattern`: the word with each whitespace run widened to `\s+`.
    pub(super) fn highlighted_words<'w>(words: impl IntoIterator<Item = &'w str>) -> Self {
        Self::new(
            words
                .into_iter()
                .map(|word| {
                    let mut pattern = String::new();
                    let mut in_space = false;
                    for character in word.chars() {
                        if is_js_space(character) {
                            if !in_space {
                                pattern.push_str(JS_SPACE_RUN);
                            }
                            in_space = true;
                        } else {
                            pattern.push_str(&regex::escape(character.encode_utf8(&mut [0; 4])));
                            in_space = false;
                        }
                    }
                    pattern
                })
                .collect(),
        )
    }
}

/// JS `\s+`.
const JS_SPACE_RUN: &str = r"[\t\n\x0B\x0C\r \xA0\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]+";

/// Successive JS `exec` matches of `pattern` bounded on both sides by a character outside
/// `is_inner` (or the text edge), as byte ranges.
fn bounded_matches<'s>(
    source: &'s str,
    pattern: &'s Regex,
    is_inner: fn(char) -> bool,
) -> impl Iterator<Item = (usize, usize)> + 's {
    let mut position = 0;
    std::iter::from_fn(move || {
        while let Some(found) = pattern.find_at(source, position) {
            let before = source[..found.start()].chars().next_back();
            let after = source[found.end()..].chars().next();
            if before.is_none_or(|character| !is_inner(character))
                && after.is_none_or(|character| !is_inner(character))
            {
                position = found.end();
                return Some((found.start(), found.end()));
            }
            position = found.start()
                + source[found.start()..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
        }
        None
    })
}

/// `incorrect-interjection-forms`: a variant spelling standing as a whole word.
pub(super) fn incorrect_interjection_forms(row: &Row<'_>) -> bool {
    let source = row.text.source;
    row.tables.interjections.occurring(source).any(|pattern| {
        bounded_matches(source, pattern, is_letter_number_mark)
            .next()
            .is_some()
    })
}

/// `highlighted-words`: a highlighted word outside tags.
pub(super) fn highlighted_words(row: &Row<'_>) -> bool {
    let text = &row.text;
    row.tables
        .highlighted_words
        .occurring(text.source)
        .any(|pattern| {
            bounded_matches(text.source, pattern, is_word_character)
                .take(COMPACTED_MATCH_LIMIT)
                .any(|(start, end)| {
                    !is_range_inside_generic_tag(text, text.unit_index(start), text.unit_index(end))
                })
        })
}

fn token_text(text: &Text<'_>, token: Token) -> String {
    text.slice(token.start, token.end)
}

fn is_word_token(token: Option<&Token>) -> bool {
    token.is_some_and(|token| token.kind == TokenKind::Word)
}

/// `/^\p{L}+(?:-\p{L}+)*$/u` over a word token.
fn is_letter_only_word_token(text: &Text<'_>, token: Option<&Token>) -> bool {
    token.is_some_and(|&token| {
        token.kind == TokenKind::Word
            && token_text(text, token)
                .split('-')
                .all(|part| !part.is_empty() && part.chars().all(is_letter))
    })
}

fn is_token(
    text: &Text<'_>,
    token: Option<&Token>,
    kind: TokenKind,
    content: Option<char>,
) -> bool {
    token.is_some_and(|token| {
        token.kind == kind
            && content.is_none_or(|content| token.len() == 1 && text.is(token.start, content))
    })
}

/// `normalized-stutters` (`getNormalizedStutterMatches`): in a `fragment- word` chain each
/// fragment must start the final word and be no longer than the word after it.
pub(super) fn normalized_stutters(row: &Row<'_>) -> bool {
    let (text, tokens) = (&row.text, &row.tokens);
    for index in 0..tokens.len() {
        let mut cursor = index;
        let mut fragments = Vec::new();
        let terminal = loop {
            let fragment = tokens.get(cursor);
            let following = tokens.get(cursor + 3);
            if !is_letter_only_word_token(text, fragment)
                || !is_token(
                    text,
                    tokens.get(cursor + 1),
                    TokenKind::Punctuation,
                    Some('-'),
                )
                || !is_token(text, tokens.get(cursor + 2), TokenKind::Space, None)
                || !is_word_token(following)
            {
                break None;
            }
            let (fragment, following) = (tokens[cursor], tokens[cursor + 3]);
            fragments.push(fragment);
            if fragment.len() > following.len() {
                return true;
            }
            if is_letter_only_word_token(text, Some(&following))
                && is_token(
                    text,
                    tokens.get(cursor + 4),
                    TokenKind::Punctuation,
                    Some('-'),
                )
                && is_token(text, tokens.get(cursor + 5), TokenKind::Space, None)
            {
                cursor += 3;
                continue;
            }
            break Some(following);
        };
        let Some(terminal) = terminal else {
            continue;
        };
        let terminal = token_text(text, terminal).to_lowercase();
        if fragments
            .iter()
            .any(|&fragment| !terminal.starts_with(&token_text(text, fragment).to_lowercase()))
        {
            return true;
        }
    }
    false
}

/// yo-orthography.ts `hasYo`: `/[ёЁ]|[еЕ]\u0308/u`.
fn has_yo(word: &str) -> bool {
    let mut previous = None;
    word.chars().any(|character| {
        let found = matches!(character, 'ё' | 'Ё')
            || (character == '\u{308}' && matches!(previous, Some('е' | 'Е')));
        previous = Some(character);
        found
    })
}

fn starts_with_capital(word: &str) -> bool {
    word.chars().next().is_some_and(is_capital)
}

fn exception_key(word: &str) -> String {
    word.nfc().collect::<String>().to_lowercase()
}

fn is_yo_hyphen(character: char) -> bool {
    matches!(character, '\u{2010}' | '\u{2011}')
}

/// `unnecessary-yo` (`getUnnecessaryYoMatches`): a lowercase word (or hyphenated component)
/// written with ё that the exception dictionary does not list.
pub(super) fn unnecessary_yo(row: &Row<'_>) -> bool {
    let (text, tokens, exceptions) = (&row.text, &row.tokens, &row.tables.yo_exceptions);
    if !has_yo(text.source) {
        return false;
    }
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        index += 1;
        if token.kind != TokenKind::Word {
            continue;
        }
        let mut end = token.end;
        // Typographic hyphens are not joined by the tokenizer.
        while tokens.get(index).is_some_and(|hyphen| {
            hyphen.len() == 1 && text.char_at(hyphen.start).is_some_and(is_yo_hyphen)
        }) && is_word_token(tokens.get(index + 1))
        {
            end = tokens[index + 1].end;
            index += 2;
        }
        let word = text.slice(token.start, end);
        if !has_yo(&word)
            || starts_with_capital(&word)
            || exceptions.contains(&exception_key(&word).replace(is_yo_hyphen, "-"))
        {
            continue;
        }
        if word
            .split(|character| character == '-' || is_yo_hyphen(character))
            .any(|part| {
                !part.is_empty()
                    && has_yo(part)
                    && !starts_with_capital(part)
                    && !exceptions.contains(&exception_key(part))
            })
        {
            return true;
        }
    }
    false
}
