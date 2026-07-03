use studio_core::Segment;

pub(super) fn stable_display_id(segment: &Segment) -> String {
    if let Some(source_annotation_id) = segment.source_annotation_id.as_deref() {
        return trailing_number(source_annotation_id)
            .map(format_stable_number)
            .unwrap_or_else(|| source_annotation_id.to_string());
    }

    if let Some((base_id, derived_index)) = segment.id.rsplit_once("__studio_") {
        let base = trailing_number(base_id)
            .map(format_stable_number)
            .unwrap_or_else(|| base_id.to_string());
        return trailing_number(derived_index)
            .map(|index| format!("{base}.{index}"))
            .unwrap_or_else(|| format!("{base}.new"));
    }

    trailing_number(&segment.id)
        .map(format_stable_number)
        .unwrap_or_else(|| segment.id.clone())
}

fn trailing_number(value: &str) -> Option<u64> {
    let mut multiplier = 1;
    let mut number = 0;
    let mut found_digit = false;
    for byte in value.bytes().rev() {
        if !byte.is_ascii_digit() {
            break;
        }
        found_digit = true;
        number += u64::from(byte - b'0') * multiplier;
        multiplier *= 10;
    }

    found_digit.then_some(number)
}

fn format_stable_number(number: u64) -> String {
    if number < 100 {
        format!("{number:02}")
    } else {
        number.to_string()
    }
}
