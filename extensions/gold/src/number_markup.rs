//! Babel's number markup: `4 {СКАЗ: четыре}` keeps the digits for readers and the spoken
//! words for the model. Documents without the markup policy get the spoken words only.

use std::borrow::Cow;

const OPEN: &str = "{СКАЗ:";

/// Rewrites every `N {СКАЗ: words}` (and bare `{СКАЗ: words}`) to `words`. `N` is the
/// digit form Babel writes before the tag: grouped digits, decimal comma, fraction, percent,
/// range or digit sequence (`1 234`, `3,5`, `3/4`, `15 %`, `10-20`).
pub fn strip_number_markup(text: &str) -> Cow<'_, str> {
    if !text.contains(OPEN) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(OPEN) {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let words = rest[open + OPEN.len()..open + close].trim();
        let kept = &rest[..number_start(&rest[..open])];
        out.push_str(kept);
        out.push_str(words);
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Byte offset where the digit form before the tag begins (`prefix.len()` when there is
/// none): trailing whitespace-separated words made of digits and number punctuation.
fn number_start(prefix: &str) -> usize {
    let trimmed = prefix.trim_end();
    let mut start = trimmed.len();
    let mut end = trimmed.len();
    loop {
        let word_start = trimmed[..end]
            .trim_end_matches(|c: char| !c.is_whitespace())
            .len();
        let word = &trimmed[word_start..end];
        if word.is_empty() || !is_number_word(word) {
            break;
        }
        start = word_start;
        end = trimmed[..word_start].trim_end().len();
    }
    if start == trimmed.len() {
        prefix.len()
    } else {
        start
    }
}

fn is_number_word(word: &str) -> bool {
    word == "%"
        || (word.chars().any(|c| c.is_ascii_digit())
            && word
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '/' | '-' | '%')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_before_the_tag_goes_with_it() {
        assert_eq!(
            strip_number_markup("4 {СКАЗ: четыре} человека"),
            "четыре человека"
        );
        assert_eq!(strip_number_markup("2000 {СКАЗ: две тысячи}"), "две тысячи");
        assert_eq!(
            strip_number_markup("нас пока 4 {СКАЗ: четыре} человека, 3 {СКАЗ: три} дня"),
            "нас пока четыре человека, три дня"
        );
    }

    #[test]
    fn grouped_decimal_percent_and_range_forms() {
        assert_eq!(
            strip_number_markup("1 234 {СКАЗ: одна тысяча двести тридцать четыре}"),
            "одна тысяча двести тридцать четыре"
        );
        assert_eq!(
            strip_number_markup("это 3,5 {СКАЗ: три с половиной} кг"),
            "это три с половиной кг"
        );
        assert_eq!(
            strip_number_markup("рост 15 % {СКАЗ: пятнадцать процентов}"),
            "рост пятнадцать процентов"
        );
        assert_eq!(
            strip_number_markup("где-то 10-20 {СКАЗ: десять двадцать} штук"),
            "где-то десять двадцать штук"
        );
    }

    #[test]
    fn bare_tag_and_plain_text() {
        assert_eq!(
            strip_number_markup("{СКАЗ: четыре} человека"),
            "четыре человека"
        );
        assert_eq!(
            strip_number_markup("текст без разметки, 4 человека"),
            "текст без разметки, 4 человека"
        );
        assert!(matches!(
            strip_number_markup("текст без разметки"),
            Cow::Borrowed(_)
        ));
        assert_eq!(strip_number_markup("а - {СКАЗ: два}"), "а - два");
    }
}
