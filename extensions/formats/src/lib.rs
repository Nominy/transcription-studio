//! Optional subtitle/text contributions. Native project persistence is deliberately absent.
use std::borrow::Cow;
use std::fmt::Write as _;
use std::sync::Arc;

use exaudio_extension::ExtensionManifest;
use exaudio_extension::format::{
    ExportOptions, FormatDescriptor, FormatError, FormatProvider, FormatRegistry, ImportContent,
    ImportedCue,
};
use studio_core::Project;

pub const EXTENSION_ID: &str = "exaudio.formats";
pub const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CUES: usize = 100_000;
pub const MAX_CUE_BYTES: usize = 256 * 1024;
const MAX_MILLIS: u64 = 3_599_999_999_999;

pub fn manifest() -> ExtensionManifest {
    serde_json::from_str(include_str!("../manifest.json")).expect("bundled format manifest")
}

/// Host calls only while this extension is enabled; disable with
/// `registry.unregister_extension(EXTENSION_ID)`.
pub fn register(registry: &mut FormatRegistry) -> Result<(), String> {
    registry.register_many(
        EXTENSION_ID,
        vec![
            Arc::new(SubtitleFormat::Srt),
            Arc::new(SubtitleFormat::WebVtt),
            Arc::new(SubtitleFormat::Text),
        ],
    )
}

#[derive(Clone, Copy, Debug)]
pub enum SubtitleFormat {
    Srt,
    WebVtt,
    Text,
}

impl FormatProvider for SubtitleFormat {
    fn descriptor(&self) -> FormatDescriptor {
        let (id, name, extension, media_type) = match self {
            Self::Srt => (
                "exaudio.formats.srt",
                "SubRip (.srt)",
                "srt",
                "application/x-subrip",
            ),
            Self::WebVtt => ("exaudio.formats.vtt", "WebVTT (.vtt)", "vtt", "text/vtt"),
            Self::Text => (
                "exaudio.formats.txt",
                "Plain text (.txt)",
                "txt",
                "text/plain",
            ),
        };
        FormatDescriptor {
            id: id.into(),
            name: name.into(),
            extensions: vec![extension.into()],
            media_type: media_type.into(),
            can_import: true,
            can_export: true,
        }
    }

    fn import(&self, bytes: &[u8]) -> Result<ImportContent, FormatError> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(error(None, None, "input exceeds 16 MiB limit"));
        }
        let text = std::str::from_utf8(bytes).map_err(|e| {
            error(
                None,
                None,
                format!("invalid UTF-8 at byte {}", e.valid_up_to()),
            )
        })?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let normalized = normalize_newlines(text);
        if normalized.contains('\0') {
            return Err(error(None, None, "NUL is not supported in transcript text"));
        }
        match self {
            Self::Text => Ok(ImportContent::Untimed(normalized.into_owned())),
            Self::Srt => parse_subtitles(&normalized, false),
            Self::WebVtt => parse_subtitles(&normalized, true),
        }
    }

    fn export(&self, project: &Project, options: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        if project.segments.len() > MAX_CUES {
            return Err(error(None, None, "project exceeds 100000 cue limit"));
        }
        let mut cues = Vec::with_capacity(project.segments.len());
        for (index, segment) in project.segments.iter().enumerate() {
            let start = milliseconds(segment.start_seconds)
                .map_err(|message| error(None, Some(index + 1), message))?;
            let end = milliseconds(segment.end_seconds)
                .map_err(|message| error(None, Some(index + 1), message))?;
            if segment.end_seconds <= segment.start_seconds || end <= start {
                return Err(error(
                    None,
                    Some(index + 1),
                    "cue must have positive duration after millisecond rounding",
                ));
            }
            cues.push((segment, start, end, index));
        }
        // Explicit original index tie-break keeps overlapping equal-start cues stable.
        cues.sort_unstable_by(|a, b| {
            a.0.start_seconds
                .total_cmp(&b.0.start_seconds)
                .then(a.3.cmp(&b.3))
        });
        let mut output = String::new();
        if matches!(self, Self::WebVtt) {
            output.push_str("WEBVTT\n\n");
        }
        for (ordinal, (segment, start, end, source_index)) in cues.into_iter().enumerate() {
            let text = normalize_newlines(&segment.text);
            if text.len() > MAX_CUE_BYTES || text.contains('\0') {
                return Err(error(
                    None,
                    Some(source_index + 1),
                    "cue text exceeds 256 KiB or contains NUL",
                ));
            }
            if !matches!(self, Self::Text)
                && (text.is_empty() || text.split('\n').any(|line| line.trim().is_empty()))
            {
                return Err(error(
                    None,
                    Some(source_index + 1),
                    "subtitle cue text must not be empty or contain blank lines",
                ));
            }
            if matches!(self, Self::Srt) && text.lines().any(is_srt_timing_line) {
                return Err(error(
                    None,
                    Some(source_index + 1),
                    "SRT payload cannot contain a second timing line",
                ));
            }
            let speaker = if options.speaker_labels {
                Some(
                    project
                        .speakers
                        .iter()
                        .find(|speaker| speaker.id == segment.speaker_id)
                        .ok_or_else(|| {
                            error(
                                None,
                                Some(source_index + 1),
                                format!("unknown speaker {}", segment.speaker_id),
                            )
                        })?
                        .label
                        .as_str(),
                )
            } else {
                None
            };
            if speaker.is_some_and(|s| s.contains(['\n', '\r', '\0']) || s.len() > MAX_CUE_BYTES) {
                return Err(error(
                    None,
                    Some(source_index + 1),
                    "speaker label must be a single bounded line",
                ));
            }
            let payload_start = output.len();
            match self {
                Self::Srt => {
                    writeln!(output, "{}", ordinal + 1).unwrap();
                    write_timing(&mut output, start, end, ',');
                    if let Some(label) = speaker {
                        write!(output, "{label}: ").unwrap();
                    }
                    writeln!(output, "{text}\n").unwrap();
                }
                Self::WebVtt => {
                    write_timing(&mut output, start, end, '.');
                    // Labels are literal visible text, not executable cue markup or metadata.
                    if let Some(label) = speaker {
                        escape_vtt_into(label, &mut output);
                        output.push_str(": ");
                    }
                    escape_vtt_into(&text, &mut output);
                    output.push_str("\n\n");
                }
                Self::Text => {
                    if ordinal != 0 {
                        output.push('\n');
                    }
                    if let Some(label) = speaker {
                        write!(output, "{label}: ").unwrap();
                    }
                    output.push_str(&text);
                }
            }
            if !matches!(self, Self::Text) && output.len() - payload_start > MAX_CUE_BYTES {
                return Err(error(
                    None,
                    Some(source_index + 1),
                    "encoded subtitle cue exceeds 256 KiB",
                ));
            }
            if output.len() > MAX_INPUT_BYTES {
                return Err(error(None, None, "export exceeds 16 MiB limit"));
            }
        }
        if !matches!(self, Self::Text)
            && output.bytes().filter(|byte| *byte == b'\n').count() >= 1_000_000
        {
            return Err(error(
                None,
                None,
                "subtitle export exceeds 1000000 line limit",
            ));
        }
        Ok(output.into_bytes())
    }
}

fn error(line: Option<usize>, cue: Option<usize>, message: impl Into<String>) -> FormatError {
    FormatError {
        line,
        cue,
        message: message.into(),
    }
}

fn milliseconds(seconds: f64) -> Result<u64, &'static str> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("time must be finite and nonnegative");
    }
    let millis = (seconds * 1000.0).round();
    if millis > MAX_MILLIS as f64 {
        return Err("time exceeds 999999:59:59.999");
    }
    Ok(millis as u64)
}

fn write_timing(output: &mut String, start: u64, end: u64, separator: char) {
    for (index, ms) in [start, end].into_iter().enumerate() {
        if index == 1 {
            output.push_str(" --> ");
        }
        write!(
            output,
            "{:02}:{:02}:{:02}{separator}{:03}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms % 1000
        )
        .unwrap();
    }
    output.push('\n');
}

fn normalize_newlines(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            output.push('\n');
        } else {
            output.push(ch);
        }
    }
    Cow::Owned(output)
}

fn is_srt_timing_line(line: &str) -> bool {
    line.split_once("-->").is_some_and(|(left, right)| {
        parse_time(left.trim(), false).is_ok()
            && right
                .split_whitespace()
                .next()
                .is_some_and(|time| parse_time(time, false).is_ok())
    })
}

fn parse_time(value: &str, vtt: bool) -> Result<u64, &'static str> {
    let separator = if vtt { '.' } else { ',' };
    let (whole, fraction) = value
        .split_once(separator)
        .ok_or("timestamp needs millisecond fraction")?;
    if fraction.len() != 3 || !fraction.bytes().all(|c| c.is_ascii_digit()) {
        return Err("timestamp fraction must be three digits");
    }
    let mut parts = whole.split(':');
    let first = parts.next().unwrap_or_default();
    let second = parts.next().ok_or("timestamp needs minutes and seconds")?;
    let third = parts.next();
    if parts.next().is_some() || (!vtt && third.is_none()) {
        return Err("expected HH:MM:SS,mmm (SRT) or [HH:]MM:SS.mmm (VTT)");
    }
    let parse = |s: &str| -> Result<u64, &'static str> {
        if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
            return Err("timestamp fields must be unsigned digits");
        }
        s.parse().map_err(|_| "timestamp field overflows")
    };
    let (hours, minutes, seconds) = if let Some(third) = third {
        if first.len() < 2 || first.len() > 6 {
            return Err("hours must contain two to six digits");
        }
        (parse(first)?, second, third)
    } else {
        (0, first, second)
    };
    if minutes.len() != 2 || seconds.len() != 2 {
        return Err("minutes and seconds must have two digits");
    }
    let (minutes, seconds) = (parse(minutes)?, parse(seconds)?);
    if minutes > 59 || seconds > 59 {
        return Err("minutes and seconds must be below 60");
    }
    Ok(hours * 3_600_000 + minutes * 60_000 + seconds * 1000 + parse(fraction)?)
}

fn parse_subtitles(text: &str, vtt: bool) -> Result<ImportContent, FormatError> {
    if text.bytes().filter(|byte| *byte == b'\n').count() >= 1_000_000 {
        return Err(error(
            None,
            None,
            "subtitle input exceeds 1000000 line limit",
        ));
    }
    let lines: Vec<_> = text.split('\n').collect();
    let mut cursor = 0;
    if vtt {
        let header = lines[0];
        if !(header == "WEBVTT" || header.starts_with("WEBVTT ") || header.starts_with("WEBVTT\t"))
            || header.contains("-->")
        {
            return Err(error(Some(1), None, "expected WEBVTT header"));
        }
        cursor = 1;
        while cursor < lines.len() && !lines[cursor].trim().is_empty() {
            if lines[cursor].contains("-->") {
                return Err(error(
                    Some(cursor + 1),
                    None,
                    "missing blank line after WEBVTT header",
                ));
            }
            cursor += 1;
        }
    }
    let mut cues = Vec::new();
    while cursor < lines.len() {
        if lines[cursor].trim().is_empty() {
            cursor += 1;
            continue;
        }
        let begin = cursor;
        while cursor < lines.len() && !lines[cursor].trim().is_empty() {
            cursor += 1;
        }
        let block = &lines[begin..cursor];
        if vtt
            && (block[0] == "NOTE"
                || block[0].starts_with("NOTE ")
                || block[0].starts_with("NOTE\t"))
        {
            continue;
        }
        if vtt && matches!(block[0], "STYLE" | "REGION") {
            if !cues.is_empty() {
                return Err(error(
                    Some(begin + 1),
                    None,
                    "STYLE/REGION blocks must precede cues",
                ));
            }
            if block.iter().any(|line| line.contains("-->")) {
                return Err(error(Some(begin + 1), None, "invalid STYLE/REGION block"));
            }
            continue; // Presentation-only metadata: not transcript content.
        }
        let ordinal = cues.len() + 1;
        if cues.len() == MAX_CUES {
            return Err(error(
                Some(begin + 1),
                Some(ordinal),
                "input exceeds 100000 cues",
            ));
        }
        let timing_index = usize::from(!block[0].contains("-->"));
        if timing_index == 1 && !vtt && !block[0].bytes().all(|c| c.is_ascii_digit()) {
            return Err(error(
                Some(begin + 1),
                Some(ordinal),
                "SRT cue identifier must be numeric",
            ));
        }
        let timing = block
            .get(timing_index)
            .ok_or_else(|| error(Some(begin + 1), Some(ordinal), "missing cue timing line"))?;
        let timing_line = begin + timing_index + 1;
        let fail = |message: &str| error(Some(timing_line), Some(ordinal), message);
        let (start, rest) = timing
            .split_once("-->")
            .ok_or_else(|| fail("missing --> timing separator"))?;
        let mut fields = rest.split_whitespace();
        let end = fields
            .next()
            .ok_or_else(|| fail("missing cue end timestamp"))?;
        let start = parse_time(start.trim(), vtt).map_err(fail)?;
        let end = parse_time(end, vtt).map_err(fail)?;
        if end <= start {
            return Err(fail("cue end must be greater than start"));
        }
        if vtt {
            validate_settings(fields).map_err(fail)?;
        } else if fields.next().is_some() {
            return Err(fail("unexpected SRT timing suffix"));
        }
        let payload = &block[timing_index + 1..];
        if payload.is_empty() {
            return Err(error(
                Some(timing_line + 1),
                Some(ordinal),
                "missing cue text",
            ));
        }
        if payload.iter().map(|s| s.len()).sum::<usize>() + payload.len() - 1 > MAX_CUE_BYTES {
            return Err(error(
                Some(timing_line + 1),
                Some(ordinal),
                "cue text exceeds 256 KiB",
            ));
        }
        if let Some(offset) = payload
            .iter()
            .position(|line| (vtt && line.contains("-->")) || is_srt_timing_line(line))
        {
            return Err(error(
                Some(timing_line + 1 + offset),
                Some(ordinal),
                "unexpected timing separator in cue text (possibly missing blank separator)",
            ));
        }
        let raw = payload.join("\n");
        let (text, speaker) = if vtt {
            decode_vtt(&raw).map_err(|(offset, message)| {
                error(Some(timing_line + 1 + offset), Some(ordinal), message)
            })?
        } else {
            (raw, None)
        };
        cues.push(ImportedCue {
            start_seconds: start as f64 / 1000.0,
            end_seconds: end as f64 / 1000.0,
            text,
            speaker,
        });
    }
    Ok(ImportContent::Timed(cues))
}

fn validate_settings<'a>(fields: impl Iterator<Item = &'a str>) -> Result<(), &'static str> {
    let mut seen = std::collections::BTreeSet::new();
    for field in fields {
        let (key, value) = field.split_once(':').ok_or("invalid VTT cue setting")?;
        if !seen.insert(key) {
            return Err("duplicate VTT cue setting");
        }
        let percent = |v: &str| {
            v.strip_suffix('%').is_some_and(|n| {
                !n.is_empty()
                    && n.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                    && n.parse::<f64>().is_ok_and(|n| (0.0..=100.0).contains(&n))
            })
        };
        let valid = match key {
            "vertical" => matches!(value, "rl" | "lr"),
            "align" => matches!(value, "start" | "center" | "end" | "left" | "right"),
            "size" => percent(value),
            "region" => !value.is_empty(),
            "position" | "line" => {
                let (number, alignment) = value
                    .split_once(',')
                    .map_or((value, None), |(a, b)| (a, Some(b)));
                let valid_number = percent(number)
                    || (key == "line" && (number == "auto" || number.parse::<i32>().is_ok()));
                let valid_alignment = alignment.is_none_or(|a| {
                    if key == "line" {
                        matches!(a, "start" | "center" | "end")
                    } else {
                        matches!(a, "line-left" | "center" | "line-right" | "auto")
                    }
                });
                valid_number && valid_alignment
            }
            _ => false,
        };
        if !valid {
            return Err("invalid or unsupported VTT cue setting");
        }
    }
    Ok(())
}

fn escape_vtt_into(text: &str, output: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(ch),
        }
    }
}

/// VTT cue markup is presentation, never injected into transcript text. Unknown
/// markup/entities fail explicitly rather than silently discarding user content.
fn decode_vtt(raw: &str) -> Result<(String, Option<String>), (usize, String)> {
    let mut output = String::with_capacity(raw.len());
    let mut speaker = None;
    let mut rest = raw;
    let mut line = 0;
    while !rest.is_empty() {
        let boundary = rest.find(['<', '&']).unwrap_or(rest.len());
        output.push_str(&rest[..boundary]);
        line += rest[..boundary].bytes().filter(|b| *b == b'\n').count();
        rest = &rest[boundary..];
        if rest.is_empty() {
            break;
        }
        let fail = |message: &str| (line, message.to_owned());
        if rest.starts_with('&') {
            let end = rest
                .find(';')
                .ok_or_else(|| fail("unterminated VTT entity; literal ampersands must be &amp;"))?;
            let entity = &rest[1..end];
            output.push_str(match entity {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "nbsp" => "\u{a0}",
                "lrm" => "\u{200e}",
                "rlm" => "\u{200f}",
                _ => return Err(fail(&format!("unsupported VTT entity &{entity};"))),
            });
            rest = &rest[end + 1..];
        } else {
            let end = rest.find('>').ok_or_else(|| {
                fail("unterminated VTT tag; literal less-than signs must be &lt;")
            })?;
            let tag = &rest[1..end];
            if tag.contains('\n') {
                return Err(fail("VTT tags must not span lines"));
            }
            if let Some(voice) = tag.strip_prefix("v ").or_else(|| tag.strip_prefix("v\t")) {
                let voice = voice.trim();
                if voice.is_empty() {
                    return Err(fail("empty VTT voice annotation"));
                }
                let (voice, nested) = decode_vtt(voice).map_err(|(_, message)| (line, message))?;
                if nested.is_some() {
                    return Err(fail("nested voice annotation"));
                }
                if speaker.as_ref().is_some_and(|existing| existing != &voice) {
                    return Err(fail(
                        "multiple voices in one VTT cue; split into separate cues",
                    ));
                }
                speaker = Some(voice);
            } else if !(matches!(
                tag,
                "b" | "/b"
                    | "i"
                    | "/i"
                    | "u"
                    | "/u"
                    | "c"
                    | "/c"
                    | "/v"
                    | "ruby"
                    | "/ruby"
                    | "rt"
                    | "/rt"
                    | "/lang"
            ) || tag.starts_with("c.")
                || tag.starts_with("lang ")
                || parse_time(tag, true).is_ok())
            {
                return Err(fail(&format!("unsupported VTT tag <{tag}>")));
            }
            rest = &rest[end + 1..];
        }
    }
    Ok((output, speaker))
}
