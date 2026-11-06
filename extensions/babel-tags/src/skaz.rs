//! Babel's Russian numeric annotation conversions.
const MAX_SUPPORTED_DIGITS: usize = 12;
/// SKAZ replacement for a selected number (`1234` → `1 234 {СКАЗ: одна тысяча ...}`), decimal
/// comma, `n/d` fraction, percent, integer range, or a spoken digit sequence.
pub fn skaz_for_selection(selected_text: &str) -> Option<String> {
    build_auto_converted_number_text(selected_text)
}

/// Replacement when a digit is typed over selected spoken digit text: `"{digit} {СКАЗ: sel}"`.
/// The caret belongs one char after the start of the replacement (right after the digit).
pub fn skaz_with_digit(digit: char, selected_text: &str) -> Option<String> {
    (!selected_text.trim().is_empty()).then(|| format!("{digit} {{СКАЗ: {selected_text}}}"))
}
const HUNDREDS: [&str; 9] = [
    "сто",
    "двести",
    "триста",
    "четыреста",
    "пятьсот",
    "шестьсот",
    "семьсот",
    "восемьсот",
    "девятьсот",
];

const TENS: [&str; 8] = [
    "двадцать",
    "тридцать",
    "сорок",
    "пятьдесят",
    "шестьдесят",
    "семьдесят",
    "восемьдесят",
    "девяносто",
];

const TEENS: [&str; 10] = [
    "десять",
    "одиннадцать",
    "двенадцать",
    "тринадцать",
    "четырнадцать",
    "пятнадцать",
    "шестнадцать",
    "семнадцать",
    "восемнадцать",
    "девятнадцать",
];

const UNITS_MALE: [&str; 9] = [
    "один",
    "два",
    "три",
    "четыре",
    "пять",
    "шесть",
    "семь",
    "восемь",
    "девять",
];

const UNITS_FEMALE: [&str; 9] = [
    "одна",
    "две",
    "три",
    "четыре",
    "пять",
    "шесть",
    "семь",
    "восемь",
    "девять",
];

const SCALE_FORMS: [(&str, &str, &str); 4] = [
    ("", "", ""),
    ("тысяча", "тысячи", "тысяч"),
    ("миллион", "миллиона", "миллионов"),
    ("миллиард", "миллиарда", "миллиардов"),
];

const DIGIT_WORDS: [&str; 10] = [
    "ноль",
    "один",
    "два",
    "три",
    "четыре",
    "пять",
    "шесть",
    "семь",
    "восемь",
    "девять",
];

fn build_auto_converted_number_text(selected_text: &str) -> Option<String> {
    build_expanded_skaz_text(selected_text)
        .or_else(|| build_skaz_from_slash_fraction(selected_text))
        .or_else(|| build_expanded_skaz_for_numeric_pattern(selected_text))
        .or_else(|| build_skaz_from_spoken_digit_sequence(selected_text))
}

fn build_expanded_skaz_text(selected_text: &str) -> Option<String> {
    let trimmed = selected_text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if is_decimal_comma_text(trimmed) {
        let words = decimal_comma_text_to_russian_words(trimmed)?;
        return Some(format!(
            "{} {{СКАЗ: {words}}}",
            format_grouped_decimal_text(trimmed)
        ));
    }

    if !is_integer_text(trimmed) {
        return None;
    }

    let words = number_text_to_russian_words(trimmed)?;
    Some(format!(
        "{} {{СКАЗ: {words}}}",
        format_grouped_integer_text(trimmed)
    ))
}

fn build_skaz_from_slash_fraction(selected_text: &str) -> Option<String> {
    let trimmed = selected_text.trim();
    let (numerator_text, denominator_text) = split_slash_fraction(trimmed)?;
    let denominator = denominator_text.parse::<i64>().ok()?;
    if !(2..=20).contains(&denominator) {
        return None;
    }
    let numerator = numerator_text.parse::<i64>().ok()?;
    let numerator_words = integer_text_to_russian_words(numerator_text, true)?;
    let denominator_word = choose_fraction_denominator_word(denominator, numerator)?;
    Some(format!(
        "{trimmed} {{СКАЗ: {numerator_words} {denominator_word}}}"
    ))
}

fn build_expanded_skaz_for_numeric_pattern(selected_text: &str) -> Option<String> {
    let trimmed = selected_text.trim();
    if let Some(digits) = trimmed.strip_suffix('%').map(str::trim_end)
        && is_integer_text(digits)
    {
        let words = number_text_to_russian_words(digits)?;
        let percent_word = pick_plural_form(
            digits.parse::<i64>().ok()?,
            "процент",
            "процента",
            "процентов",
        );
        return Some(format!(
            "{} % {{СКАЗ: {words} {percent_word}}}",
            format_grouped_integer_text(digits)
        ));
    }

    let parts = parse_integer_range_parts(trimmed)?;
    if parts.len() < 2 {
        return None;
    }
    let words = parts
        .iter()
        .map(|part| number_text_to_russian_words(part))
        .collect::<Option<Vec<_>>>()?;
    let grouped = parts
        .iter()
        .map(|part| format_grouped_integer_text(part))
        .collect::<Vec<_>>()
        .join("-");
    Some(format!("{grouped} {{СКАЗ: {}}}", words.join(" ")))
}

fn build_skaz_from_spoken_digit_sequence(selected_text: &str) -> Option<String> {
    let trimmed = selected_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tokens = trimmed
        .to_lowercase()
        .split_whitespace()
        .map(spoken_digit_word)
        .collect::<Option<Vec<_>>>()?;
    if tokens.len() < 2 {
        return None;
    }
    Some(format!("{} {{СКАЗ: {trimmed}}}", tokens.join("-")))
}

fn number_text_to_russian_words(number_text: &str) -> Option<String> {
    integer_text_to_russian_words(number_text, false)
}

fn decimal_comma_text_to_russian_words(number_text: &str) -> Option<String> {
    if !is_decimal_comma_text(number_text) {
        return None;
    }
    let (sign_prefix, unsigned) = unsigned_with_sign_prefix(number_text);
    let (integer_part, fraction_part) = unsigned.split_once(',')?;
    let integer_words = number_text_to_russian_words(integer_part)?;
    let fraction_words = fraction_part
        .chars()
        .map(|character| {
            character
                .to_digit(10)
                .and_then(|digit| DIGIT_WORDS.get(digit as usize).copied())
        })
        .collect::<Option<Vec<_>>>()?;
    if fraction_words.len() != fraction_part.len() || fraction_words.is_empty() {
        return None;
    }
    Some(format!(
        "{sign_prefix}{integer_words} и {}",
        fraction_words.join(" ")
    ))
}

fn integer_text_to_russian_words(number_text: &str, last_triad_feminine: bool) -> Option<String> {
    if !is_integer_text(number_text) {
        return None;
    }

    let (sign_prefix, mut digits) = unsigned_with_sign_prefix(number_text);
    digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some(format!("{sign_prefix}ноль"));
    }
    if digits.len() > MAX_SUPPORTED_DIGITS {
        return None;
    }

    let mut triads = Vec::new();
    let mut end = digits.len();
    while end > 0 {
        let start = end.saturating_sub(3);
        triads.insert(0, &digits[start..end]);
        end = start;
    }

    let mut parts = Vec::new();
    for (index, triad) in triads.iter().enumerate() {
        let triad_value = triad.parse::<i64>().ok()?;
        if triad_value == 0 {
            continue;
        }
        let scale_index = triads.len().saturating_sub(index + 1);
        let is_feminine = scale_index == 1 || (scale_index == 0 && last_triad_feminine);
        let triad_words = three_digit_to_words(triad_value, is_feminine)?;
        if scale_index > 0 {
            let (one, two_to_four, many) = SCALE_FORMS.get(scale_index).copied()?;
            parts.push(format!(
                "{triad_words} {}",
                pick_plural_form(triad_value, one, two_to_four, many)
            ));
        } else {
            parts.push(triad_words);
        }
    }

    if parts.is_empty() {
        Some(format!("{sign_prefix}ноль"))
    } else {
        Some(format!("{sign_prefix}{}", parts.join(" ")))
    }
}

fn three_digit_to_words(value: i64, is_feminine: bool) -> Option<String> {
    let mut parts = Vec::new();
    let hundreds = value / 100;
    let remainder = value % 100;
    let tens = remainder / 10;
    let units = remainder % 10;

    if hundreds > 0 {
        parts.push(*HUNDREDS.get(hundreds as usize - 1)?);
    }
    if (10..=19).contains(&remainder) {
        parts.push(*TEENS.get(remainder as usize - 10)?);
        return Some(parts.join(" "));
    }
    if tens >= 2 {
        parts.push(*TENS.get(tens as usize - 2)?);
    }
    if units > 0 {
        let units_table = if is_feminine {
            &UNITS_FEMALE
        } else {
            &UNITS_MALE
        };
        parts.push(*units_table.get(units as usize - 1)?);
    }
    Some(parts.join(" "))
}

fn choose_fraction_denominator_word(denominator: i64, numerator: i64) -> Option<&'static str> {
    let (singular, plural, plural_few) = match denominator {
        2 => ("вторая", "вторых", None),
        3 => ("третья", "третьих", None),
        4 => ("четвертая", "четвертых", Some("четверти")),
        5 => ("пятая", "пятых", None),
        6 => ("шестая", "шестых", None),
        7 => ("седьмая", "седьмых", None),
        8 => ("восьмая", "восьмых", None),
        9 => ("девятая", "девятых", None),
        10 => ("десятая", "десятых", None),
        11 => ("одиннадцатая", "одиннадцатых", None),
        12 => ("двенадцатая", "двенадцатых", None),
        13 => ("тринадцатая", "тринадцатых", None),
        14 => ("четырнадцатая", "четырнадцатых", None),
        15 => ("пятнадцатая", "пятнадцатых", None),
        16 => ("шестнадцатая", "шестнадцатых", None),
        17 => ("семнадцатая", "семнадцатых", None),
        18 => ("восемнадцатая", "восемнадцатых", None),
        19 => ("девятнадцатая", "девятнадцатых", None),
        20 => ("двадцатая", "двадцатых", None),
        _ => return None,
    };

    let absolute = numerator.abs();
    let mod10 = absolute % 10;
    let mod100 = absolute % 100;
    if mod10 == 1 && mod100 != 11 {
        return Some(singular);
    }
    if let Some(plural_few) = plural_few
        && (2..=4).contains(&mod10)
        && !(12..=14).contains(&mod100)
    {
        return Some(plural_few);
    }
    Some(plural)
}

fn pick_plural_form<'a>(value: i64, one: &'a str, two_to_four: &'a str, many: &'a str) -> &'a str {
    let absolute = value.abs();
    let mod10 = absolute % 10;
    let mod100 = absolute % 100;
    if mod10 == 1 && mod100 != 11 {
        one
    } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
        two_to_four
    } else {
        many
    }
}

fn format_grouped_integer_text(number_text: &str) -> String {
    if !is_integer_text(number_text) {
        return number_text.to_string();
    }
    let sign = number_text.strip_prefix('-').map(|_| "-").unwrap_or("");
    let digits = number_text.strip_prefix('-').unwrap_or(number_text);
    if digits.len() <= 3 || (digits.len() > 1 && digits.starts_with('0')) {
        return number_text.to_string();
    }

    let mut grouped = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(character);
    }
    format!("{sign}{grouped}")
}

fn format_grouped_decimal_text(number_text: &str) -> String {
    if !is_decimal_comma_text(number_text) {
        return number_text.to_string();
    }
    let sign = number_text.strip_prefix('-').map(|_| "-").unwrap_or("");
    let unsigned = number_text.strip_prefix('-').unwrap_or(number_text);
    let Some((integer_part, fraction_part)) = unsigned.split_once(',') else {
        return number_text.to_string();
    };
    let grouped_integer_part = format_grouped_integer_text(&format!("{sign}{integer_part}"));
    let normalized_integer_part = grouped_integer_part
        .strip_prefix('-')
        .unwrap_or(&grouped_integer_part);
    format!("{sign}{normalized_integer_part},{fraction_part}")
}

fn split_slash_fraction(trimmed: &str) -> Option<(&str, &str)> {
    let (numerator, denominator) = trimmed.split_once('/')?;
    let numerator = numerator.trim();
    let denominator = denominator.trim();
    if !is_integer_text(numerator)
        || denominator.starts_with('-')
        || !is_unsigned_integer(denominator)
    {
        return None;
    }
    Some((numerator, denominator))
}

fn parse_integer_range_parts(trimmed: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut index = 0;
    let bytes = trimmed.as_bytes();

    loop {
        skip_ascii_whitespace(trimmed, &mut index);
        let start = index;
        if bytes.get(index) == Some(&b'-') {
            index += 1;
        }
        let digit_start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if digit_start == index {
            return None;
        }
        parts.push(&trimmed[start..index]);
        skip_ascii_whitespace(trimmed, &mut index);
        if index == trimmed.len() {
            break;
        }
        if bytes.get(index) != Some(&b'-') {
            return None;
        }
        index += 1;
    }

    (parts.len() >= 2).then_some(parts)
}

fn skip_ascii_whitespace(text: &str, index: &mut usize) {
    while text
        .as_bytes()
        .get(*index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        *index += 1;
    }
}

fn is_integer_text(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    !unsigned.is_empty() && unsigned.chars().all(|character| character.is_ascii_digit())
}

fn is_unsigned_integer(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|character| character.is_ascii_digit())
}

fn is_decimal_comma_text(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let Some((integer_part, fraction_part)) = unsigned.split_once(',') else {
        return false;
    };
    !integer_part.is_empty()
        && !fraction_part.is_empty()
        && is_unsigned_integer(integer_part)
        && is_unsigned_integer(fraction_part)
}

fn unsigned_with_sign_prefix(text: &str) -> (&'static str, &str) {
    if let Some(unsigned) = text.strip_prefix('-') {
        ("минус ", unsigned)
    } else {
        ("", text)
    }
}

fn spoken_digit_word(token: &str) -> Option<&'static str> {
    match token {
        "ноль" => Some("0"),
        "один" | "одна" => Some("1"),
        "два" | "две" => Some("2"),
        "три" => Some("3"),
        "четыре" => Some("4"),
        "пять" => Some("5"),
        "шесть" => Some("6"),
        "семь" => Some("7"),
        "восемь" => Some("8"),
        "девять" => Some("9"),
        _ => None,
    }
}
