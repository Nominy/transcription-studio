pub(super) const TRIPLE_DASH_REASON: &str = "Triple or more dashes (---) are typically avoided.";
pub(super) const SPACED_DOUBLE_DASH_REASON: &str =
    "Double dash should hug the preceding word (no space before --).";
pub(super) const DOUBLE_DASH_PUNCTUATION_REASON: &str =
    "Punctuation immediately after double dash is typically avoided.";
pub(super) const DOUBLE_DASH_MISSING_SPACE_REASON: &str =
    "Double dash mid-sentence should be followed by a space or closing quote.";
pub(super) const NEWLINE_CHARACTERS_REASON: &str = "Segment contains newline character(s).";
pub(super) const MULTIPLE_ASTERISKS_REASON: &str =
    "Multiple asterisks in a row (**, ***, etc.) are not allowed.";
pub(super) const UNPAIRED_ASTERISKS_REASON: &str = "Asterisks must come in pairs (*word*).";
pub(super) const EMPTY_EMPHASIS_REASON: &str =
    "Empty emphasis (*  *) is not allowed - add text between asterisks.";
pub(super) const PUNCTUATION_INSIDE_EMPHASIS_REASON: &str =
    "Punctuation should be outside emphasis asterisk.";
pub(super) const CONSECUTIVE_PUNCTUATION_REASON: &str =
    "Consecutive terminal punctuation or commas detected.";
pub(super) const EM_DASH_REASON: &str =
    "Em dash (—) is not allowed. Use a standard dash with spaces on either side ( - ) instead.";
pub(super) const PARENTHESES_REASON: &str = "Parentheses are not used for transcription, with rare exceptions. Use [] or <> for tags, or {} for MIS/PRO/comments.";
pub(super) const UNPAIRED_QUOTES_REASON: &str =
    "Quotation marks must come in pairs — this segment has an odd number of quotation marks.";
pub(super) const BANNED_CHARACTERS_REASON: &str = "Banned character(s) detected.";
pub(super) const CONSECUTIVE_CAPITALS_REASON: &str = "Consecutive capital letters detected.";
pub(super) const CONSECUTIVE_CHARS_REASON: &str = "Consecutive repeated character(s) detected.";
pub(super) const EXCESSIVE_DIGITS_REASON: &str = "Excessive digit sequence detected.";
pub(super) const BANNED_WORDS_REASON: &str = "Banned word(s) detected.";
pub(super) const INTRA_SEGMENT_DUPLICATE_SUBSTRING_LENGTH: usize = 50;

const BANNED_CHARACTER_TOKENS: &[&str] = &[
    "!==", "<<=", ">>=", "+=", "-=", "*=", "/=", "<=", ">=", "&&", "||", "::", "->", "=>", "<<",
    ">>", "=", "|", "\\", "^", "±", "×", "÷", "√", "∞", "≈", "≠", "≤", "≥", "∑", "∏", "∫", "Δ",
    "∂",
];

const BANNED_WORDS_RU: &[&str] = &[
    "блядь",
    "blyad",
    "сука",
    "suka",
    "хуй",
    "khuy",
    "пизда",
    "pizda",
    "ёбаный",
    "yobany",
    "мудак",
    "mudak",
    "залупа",
    "zalupa",
    "дерьмо",
    "derymo",
    "пиздец",
    "pizdets",
    "ебать",
    "yebat",
    "хуёвый",
    "khuyovy",
    "пиздабол",
    "pizdabol",
    "ёб твою мать",
    "yob tvoyu mat",
    "долбоёб",
    "dolboyob",
    "шлюха",
    "shlyukha",
    "говно",
    "govno",
    "жопа",
    "zhopa",
    "заебал",
    "zayebal",
    "ублюдок",
    "ublyudok",
    "выблядок",
    "vyblaydok",
    "пидорас",
    "pidoras",
    "ёбанат",
    "yobanat",
    "хуесос",
    "khuyesos",
    "мразь",
    "mraz",
    "падла",
    "padla",
];

pub(super) fn has_spaced_double_dash(text: &str) -> bool {
    text.contains(" --")
}

pub(super) fn has_multiple_asterisks(text: &str) -> bool {
    text.contains("**")
}

pub(super) fn has_unpaired_asterisk(text: &str) -> bool {
    !has_multiple_asterisks(text) && text.matches('*').count() % 2 == 1
}

pub(super) fn has_empty_emphasis(text: &str) -> bool {
    if has_multiple_asterisks(text) || has_unpaired_asterisk(text) {
        return false;
    }

    emphasis_ranges(text).any(|emphasis| !emphasis.chars().any(char_is_letter_or_number))
}

pub(super) fn has_punctuation_inside_emphasis(text: &str) -> bool {
    if has_multiple_asterisks(text) || has_unpaired_asterisk(text) {
        return false;
    }

    emphasis_ranges(text).any(|emphasis| {
        emphasis
            .chars()
            .last()
            .is_some_and(|character| matches!(character, '.' | ',' | '!' | '?' | ';' | ':'))
    })
}

pub(super) fn has_double_dash_punctuation(text: &str) -> bool {
    [
        "--.", "--,", "--;", "--:", "--!", "--?", "--。", "--、", "--；", "--：", "--！", "--？",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

pub(super) fn has_double_dash_missing_space(text: &str) -> bool {
    let has_triple_dash = text.contains("---");
    let mut search_start = 0;
    while let Some(offset) = text[search_start..].find("--") {
        let dash_start = search_start + offset;
        let after_dashes = dash_start + 2;
        if let Some(next) = text[after_dashes..].chars().next() {
            if (has_triple_dash && next == '-') || matches!(next, '.' | ',' | ';' | ':' | '!' | '?')
            {
                search_start = after_dashes;
                continue;
            }
            if !is_allowed_after_double_dash(next) {
                return true;
            }
        }
        search_start = after_dashes;
    }

    false
}

pub(super) fn has_newline(text: &str) -> bool {
    text.contains('\n') || text.contains('\r')
}

pub(super) fn has_consecutive_punctuation(text: &str) -> bool {
    let mut count = 0;
    for character in text.chars() {
        if matches!(character, '.' | ',' | '!' | '?' | ';' | ':') {
            count += 1;
            if count >= 2 {
                return true;
            }
        } else if !character.is_whitespace() {
            count = 0;
        }
    }

    false
}

pub(super) fn emoji_in_text(text: &str) -> Option<String> {
    unique_matching_characters(text, is_emoji)
}

pub(super) fn has_em_dash(text: &str) -> bool {
    text.contains('—') || text.contains('–')
}

pub(super) fn unsupported_unicode_in_text(text: &str) -> Option<String> {
    unique_matching_characters(text, is_unsupported_unicode)
}

pub(super) fn has_parentheses(text: &str) -> bool {
    text.contains('(') || text.contains(')')
}

pub(super) fn has_unpaired_quotes(text: &str) -> bool {
    let quote_count = text
        .chars()
        .filter(|character| matches!(character, '"' | '“' | '”' | '„' | '«' | '»'))
        .count();
    quote_count % 2 == 1
}

pub(super) fn banned_characters_in_text(text: &str) -> Option<String> {
    let searchable = strip_angle_tags(&text.replace("\\\"", "").replace("\\'", ""));
    let mut matches = Vec::new();
    let mut index = 0;
    while index < searchable.len() {
        let remaining = &searchable[index..];
        if let Some(token) = BANNED_CHARACTER_TOKENS
            .iter()
            .find(|token| remaining.starts_with(**token))
        {
            if !matches.contains(token) {
                matches.push(*token);
            }
            index += token.len();
            continue;
        }

        let Some(character) = remaining.chars().next() else {
            break;
        };
        index += character.len_utf8();
    }

    quoted_list(matches)
}

pub(super) fn consecutive_capitals_in_text(text: &str) -> Option<String> {
    let mut run = String::new();
    for character in text.chars() {
        if character.is_uppercase() {
            run.push(character);
            if run.chars().count() >= 5 {
                return Some(format!("\"{}\"", truncate_chars(&run, 20)));
            }
        } else {
            run.clear();
        }
    }

    None
}

pub(super) fn consecutive_chars_in_text(text: &str) -> Option<String> {
    let mut previous = None;
    let mut run = String::new();
    for character in text.chars() {
        if character.is_numeric() {
            previous = None;
            run.clear();
            continue;
        }

        if Some(character) == previous {
            run.push(character);
            if run.chars().count() >= 5 {
                return Some(format!("\"{}\"", truncate_chars(&run, 10)));
            }
        } else {
            previous = Some(character);
            run.clear();
            run.push(character);
        }
    }

    None
}

pub(super) fn excessive_digits_in_text(text: &str) -> Option<String> {
    let mut run = String::new();
    for character in text.chars() {
        if character.is_numeric() {
            run.push(character);
            if run.chars().count() >= 13 {
                return Some(format!("\"{}\"", truncate_chars(&run, 20)));
            }
        } else {
            run.clear();
        }
    }

    None
}

pub(super) fn banned_words_in_text(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let words = lower
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let mut matches = Vec::new();

    for banned_word in BANNED_WORDS_RU {
        let is_phrase = banned_word.contains(' ');
        let found = if is_phrase {
            lower.contains(banned_word)
        } else {
            words.contains(banned_word)
        };
        if found && !matches.contains(banned_word) {
            matches.push(*banned_word);
        }
    }

    quoted_list(matches)
}

pub(super) fn intra_segment_duplicate_in_text(text: &str) -> Option<String> {
    if text.len() < 100 {
        return None;
    }

    let mut seen = Vec::<&str>::new();
    let mut duplicates = Vec::<&str>::new();
    for index in text
        .char_indices()
        .map(|(index, _)| index)
        .filter(|index| text.len() - index >= INTRA_SEGMENT_DUPLICATE_SUBSTRING_LENGTH)
    {
        let Some(substring) = text.get(index..index + INTRA_SEGMENT_DUPLICATE_SUBSTRING_LENGTH)
        else {
            continue;
        };
        if substring.trim().is_empty() {
            continue;
        }

        if seen.contains(&substring) {
            if !duplicates.contains(&substring) {
                duplicates.push(substring);
            }
        } else {
            seen.push(substring);
        }
    }

    let duplicate = duplicates.first()?;
    let more = if duplicates.len() > 1 {
        format!(" (+{} more)", duplicates.len() - 1)
    } else {
        String::new()
    };
    Some(format!("\"{}\"{}", truncate_chars(duplicate, 40), more))
}

fn emphasis_ranges(text: &str) -> impl Iterator<Item = &str> {
    text.split('*').enumerate().filter_map(
        |(index, part)| {
            if index % 2 == 1 { Some(part) } else { None }
        },
    )
}

fn char_is_letter_or_number(character: char) -> bool {
    character.is_alphanumeric()
}

fn is_allowed_after_double_dash(character: char) -> bool {
    matches!(
        character,
        ' ' | '\u{3000}' | '"' | '\'' | '\u{201d}' | '\u{201c}' | '\u{2019}'
    )
}

fn unique_matching_characters(text: &str, predicate: impl Fn(char) -> bool) -> Option<String> {
    let mut characters = Vec::new();
    for character in text.chars().filter(|character| predicate(*character)) {
        if !characters.contains(&character) {
            characters.push(character);
        }
    }

    (!characters.is_empty()).then(|| {
        characters
            .into_iter()
            .map(|character| character.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    })
}

fn strip_angle_tags(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut in_tag = false;
    for character in text.chars() {
        match character {
            '<' => {
                in_tag = true;
                stripped.push(' ');
            }
            '>' => {
                in_tag = false;
                stripped.push(' ');
            }
            _ if !in_tag => stripped.push(character),
            _ => {}
        }
    }
    stripped
}

fn quoted_list(matches: Vec<&str>) -> Option<String> {
    (!matches.is_empty()).then(|| {
        matches
            .into_iter()
            .map(|item| format!("\"{item}\""))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

fn is_emoji(character: char) -> bool {
    matches!(
        character as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF
    )
}

fn is_unsupported_unicode(character: char) -> bool {
    matches!(character, '“' | '”' | '‘' | '’' | '…')
}
