//! Error-category tasks: Babel payload parsing, questionnaire rules and submission payload,
//! Gold's diff alignment (ported from babel-babel-runtime `error-diff.test.mjs`), the
//! alignment window search, and Initial fill's paid decision semantics.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::extract::State;
use axum::routing::post;
use exaudio_babel_services::babel::BabelRequest;
use exaudio_babel_services::babel::error_task::{
    AlignBackend, AlignError, AssistantStatus, AudioInterval, BoundaryNote, CancelToken,
    ClipDirection, DecisionStore, DiffAlignment, DiffSide, ErrorChoices, ErrorLabelDraft,
    ErrorSource, ErrorTaskIdentity, ErrorTaskSnapshot, InitialFillRequest, NormalizedDiff,
    StyleGuide, TimedWord, align_error_diff_context, apply_draft, crop_aligned_error_diff_context,
    crop_error_diff_context, decisions_url_with, estimate_error_diff_clip, find_alignment,
    initial_fill_decision, next_error_diff_clip, pairing_problem, parse_decision_response,
    parse_error_task, parse_style_guide_document, payload_is_error_task, readiness,
    segment_error_diff_context, select_style_guide_document, submission, valid_categories,
};
use exaudio_gold::prefs::ExecutionLocation;
use serde_json::{Value, json};
use studio_inference::{
    AlignRequest, AlignResponse, BackendStatus, DraftResponse, EnhancedLane, InferenceBackend,
    InferenceResult, LaneAudio, PreservedRow, ProgressSink, TimingResponse, TimingToken,
};
use studio_waveform::DecodedAudio;

// ---------------------------------------------------------------------------------------
// Diff alignment (port of error-diff.test.mjs)

fn u16_index(text: &str, needle: &str) -> usize {
    let byte = text.find(needle).expect("needle in text");
    text[..byte].encode_utf16().count()
}

fn u16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn span_of(text: &str, needle: &str) -> (usize, usize) {
    let start = u16_index(text, needle);
    (start, start + u16_len(needle))
}

fn diff(
    human: &str,
    human_span: (usize, usize),
    machine: &str,
    machine_span: (usize, usize),
) -> NormalizedDiff {
    NormalizedDiff {
        human: DiffSide::new(human, human_span.0, human_span.1),
        machine: DiffSide::new(machine, machine_span.0, machine_span.1),
    }
}

/// `makeDiff(text, needle)` with identical sides.
fn same(text: &str, needle: &str) -> NormalizedDiff {
    let span = span_of(text, needle);
    diff(text, span, text, span)
}

fn same_at(text: &str, span: (usize, usize)) -> NormalizedDiff {
    diff(text, span, text, span)
}

fn selected(side: &DiffSide) -> String {
    side.parts().1
}

const CLIP: AudioInterval = AudioInterval {
    start_seconds: 100.0,
    end_seconds: 110.0,
};

fn words(text: &str) -> Vec<TimedWord> {
    text.split(' ')
        .enumerate()
        .map(|(index, text)| TimedWord {
            text: text.to_string(),
            start_seconds: index as f64,
            end_seconds: index as f64 + 0.5,
        })
        .collect()
}

fn word(text: &str, start: f64, end: f64) -> TimedWord {
    TimedWord {
        text: text.to_string(),
        start_seconds: start,
        end_seconds: end,
    }
}

fn timing(alignment: &DiffAlignment) -> [f64; 4] {
    [
        alignment.diff_start_seconds,
        alignment.diff_end_seconds,
        alignment.context_start_seconds,
        alignment.context_end_seconds,
    ]
}

fn assert_timing(alignment: &DiffAlignment, expected: [f64; 4]) {
    for (actual, expected) in timing(alignment).into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "{:?} != {expected:?}",
            timing(alignment)
        );
    }
}

fn ambiguous(result: Result<DiffAlignment, AlignError>) {
    match result {
        Err(AlignError::Miss(message)) => assert!(message.contains("Ambiguous"), "{message}"),
        other => panic!("expected an ambiguous miss, got {other:?}"),
    }
}

#[test]
fn crop_keeps_ten_words_per_flank() {
    let tokens: Vec<String> = (0..30).map(|index| format!("w{index}")).collect();
    let text = tokens.join(" ");
    let result = crop_error_diff_context(&same(&text, "w15"), 10).unwrap();
    assert_eq!(
        result.human.text,
        format!("… {} …", tokens[5..26].join(" "))
    );
    assert_eq!(selected(&result.human), "w15");
}

#[test]
fn crop_rebases_utf16_spans_including_non_bmp_text() {
    let text = "zero 😀ёжик two three four";
    let start = u16_index(text, "ёжик") + 1;
    let end = u16_index(text, "three") + 2;
    let result = crop_error_diff_context(&same_at(text, (start, end)), 0).unwrap();
    assert_eq!(result.human.text, "… 😀ёжик two three …");
    assert_eq!(result.human.start, 5);
    let units: Vec<u16> = text.encode_utf16().collect();
    assert_eq!(
        selected(&result.human),
        String::from_utf16_lossy(&units[start..end])
    );
}

#[test]
fn crop_handles_insertions_deletions_and_zero_width_edits() {
    let result = crop_error_diff_context(
        &diff(
            "a b extra c d",
            span_of("a b extra c d", "extra"),
            "a b c d",
            (4, 4),
        ),
        1,
    )
    .unwrap();
    assert_eq!(result.human.text, "… b extra c …");
    assert_eq!(result.machine.text, "… b c …");
    assert_eq!(selected(&result.machine), "");
    assert_eq!(result.machine.start, 4);
    let interior = crop_error_diff_context(&same_at("before word after", (9, 9)), 0).unwrap();
    assert_eq!(interior.human.text, "… word …");
    assert_eq!(interior.human.start, 4);
    let text = " \tёж\n word  ";
    assert_eq!(
        crop_error_diff_context(&same(text, "ёж"), 10)
            .unwrap()
            .human
            .text,
        text
    );
    assert!(crop_error_diff_context(&same_at("abc", (0, 4)), 10).is_err());
    assert!(crop_error_diff_context(&same_at("abc", (2, 1)), 10).is_err());
}

#[test]
fn rough_clip_is_proportional_with_four_second_flanks() {
    let segment = AudioInterval::new(20.0, 40.0);
    let clip =
        |span| estimate_error_diff_clip(&same_at("0123456789", span), segment, 10.0).unwrap();
    assert_eq!(clip((4, 6)), AudioInterval::new(25.0, 35.0));
    assert_eq!(clip((5, 5)), AudioInterval::new(26.0, 34.0));
    assert_eq!(clip((0, 0)), AudioInterval::new(20.0, 24.0));
    assert_eq!(clip((10, 10)), AudioInterval::new(36.0, 40.0));
    let one_sided = diff("word", (0, 4), "", (0, 0));
    assert_eq!(
        estimate_error_diff_clip(&one_sided, AudioInterval::new(7.0, 9.0), 10.0).unwrap(),
        AudioInterval::new(7.0, 9.0)
    );
    assert_eq!(
        estimate_error_diff_clip(&one_sided, CLIP, 2.0).unwrap(),
        AudioInterval::new(104.0, 106.0)
    );
    assert!(estimate_error_diff_clip(&same_at("", (0, 0)), CLIP, 10.0).is_err());
}

#[test]
fn alignment_normalizes_case_punctuation_and_decomposed_yo() {
    let source = same("Он сказал: «ЁЖ», потом ушёл.", "ЁЖ");
    let result =
        align_error_diff_context(&source, &words("ОН сказал е\u{308}ж потом УШЕЛ"), CLIP).unwrap();
    assert_timing(&result, [102.0, 102.5, 100.0, 104.5]);
    assert_eq!(
        selected(
            &crop_aligned_error_diff_context(&source, &result)
                .unwrap()
                .human
        ),
        "ЁЖ"
    );
}

#[test]
fn matching_ignores_symbols_and_unattached_marks() {
    let text = "[шум] «Тест_номер/42», — + €!";
    assert_timing(
        &align_error_diff_context(&same(text, "Тест_номер/42"), &words("тест номер 42"), CLIP)
            .unwrap(),
        [100.0, 102.5, 100.0, 102.5],
    );
    let result = align_error_diff_context(
        &same("тест", "тест"),
        &[
            word("_", 0.0, 0.1),
            word("\u{301}", 0.2, 0.3),
            word("тест", 0.5, 1.0),
            word("?!", 1.2, 1.3),
        ],
        CLIP,
    )
    .unwrap();
    assert_timing(&result, [100.5, 101.0, 100.5, 101.0]);
    assert_eq!(result.asr_text, "тест");
}

#[test]
fn replacement_is_enclosed_by_shared_anchors() {
    let human = "we saw a red car today";
    let machine = "we saw two blue vans today";
    let source = diff(
        human,
        span_of(human, "a red car"),
        machine,
        span_of(machine, "two blue vans"),
    );
    let result = align_error_diff_context(&source, &words(machine), CLIP).unwrap();
    assert_timing(&result, [102.0, 104.5, 100.0, 105.5]);
    let cropped = crop_aligned_error_diff_context(&source, &result).unwrap();
    assert_eq!(cropped.human.text, human);
    assert_eq!(cropped.machine.text, machine);
}

#[test]
fn hyphenated_provider_items_keep_their_whole_interval() {
    let human = "это общий тест тест пример он подходит";
    let machine = "это общий теста теста пример он подходит";
    let source = diff(
        human,
        span_of(human, "тест тест"),
        machine,
        span_of(machine, "теста теста"),
    );
    let result = align_error_diff_context(
        &source,
        &[
            word("Это", 0.0, 0.2),
            word("общий", 0.3, 0.6),
            word("тест-", 0.8, 1.1),
            word("тест-пример,", 1.2, 1.8),
            word("он", 1.9, 2.1),
            word("подходит.", 2.2, 2.6),
        ],
        CLIP,
    )
    .unwrap();
    assert_timing(&result, [100.8, 101.8, 100.0, 102.6]);
    assert_eq!(result.asr_text, "Это общий тест- тест-пример, он подходит.");
    for text in [
        "это тест пример дальше",
        "это тест-пример дальше",
        "это тест‑пример дальше",
    ] {
        let source = same(text, "тест");
        let result = align_error_diff_context(
            &source,
            &[
                word("это", 0.0, 0.2),
                word("тест-пример", 0.5, 2.0),
                word("дальше", 2.3, 2.6),
            ],
            CLIP,
        )
        .unwrap();
        assert_eq!(result.diff_start_seconds, 100.5);
        assert_eq!(
            result.diff_end_seconds, 102.0,
            "no fabricated subword endpoint"
        );
        assert_eq!(result.asr_text, "это тест-пример дальше");
    }
    assert_timing(
        &align_error_diff_context(
            &same("по-новому", "-"),
            &[word("по-новому", 1.0, 3.0)],
            CLIP,
        )
        .unwrap(),
        [101.0, 103.0, 101.0, 103.0],
    );
}

#[test]
fn repeated_words_need_unambiguous_context() {
    let text = "red cat then blue cat left";
    let last = text.rfind("cat").unwrap();
    assert_timing(
        &align_error_diff_context(&same_at(text, (last, last + 3)), &words(text), CLIP).unwrap(),
        [104.0, 104.5, 101.0, 105.5],
    );
    ambiguous(align_error_diff_context(
        &same_at("cat then cat", (9, 12)),
        &words("cat"),
        CLIP,
    ));
    ambiguous(align_error_diff_context(
        &same("red cat left", "cat"),
        &words("red cat left red cat left"),
        CLIP,
    ));
    ambiguous(align_error_diff_context(
        &diff("cat", (0, 3), "dog", (0, 3)),
        &words("cat dog"),
        CLIP,
    ));
}

#[test]
fn dp_tolerates_omissions_and_asr_insertions() {
    let human = "we really saw red cars go straight home";
    let machine = "we really saw blue vans go straight home";
    let source = diff(
        human,
        span_of(human, "red cars"),
        machine,
        span_of(machine, "blue vans"),
    );
    let result =
        align_error_diff_context(&source, &words("we saw red um cars go home"), CLIP).unwrap();
    assert_timing(&result, [102.0, 104.5, 100.0, 106.5]);
    assert_eq!(result.asr_text, "we saw red um cars go home");
}

#[test]
fn deletions_keep_the_precise_difference_inside_the_padded_window() {
    let h = "начало не буду меняться и подстраиваться и у нас с ней было потом";
    let m = "не буду меняться подстраиваться и у нас было потом";
    let at = u16_index(m, "подстраиваться");
    let human_at = u16_index(h, " и подстраиваться") + 1;
    let source = diff(h, (human_at, human_at + 1), m, (at, at));
    let result = align_error_diff_context(
        &source,
        &words("буду меняться подстраиваться и у нас было"),
        CLIP,
    )
    .unwrap();
    assert_timing(&result, [101.5, 102.0, 100.0, 105.5]);
    let cropped = crop_aligned_error_diff_context(&source, &result).unwrap();
    assert_eq!(
        cropped.human.text,
        "… буду меняться и подстраиваться и у нас …"
    );
    assert_eq!(
        cropped.machine.text,
        "… буду меняться подстраиваться и у нас …"
    );
    assert_eq!(selected(&cropped.human), "и");
    assert_eq!(selected(&cropped.machine), "");
    assert_eq!(result.asr_text, "буду меняться подстраиваться и у нас");
    let spoken =
        align_error_diff_context(&source, &words("буду меняться и подстраиваться"), CLIP).unwrap();
    assert_eq!(
        (spoken.diff_start_seconds, spoken.diff_end_seconds),
        (102.0, 102.5)
    );
    let reversed = diff(m, (at, at), h, (human_at, human_at + 1));
    assert_eq!(
        align_error_diff_context(&reversed, &words("буду меняться и подстраиваться"), CLIP)
            .unwrap()
            .diff_start_seconds,
        102.0
    );
}

#[test]
fn padding_adds_three_words_and_keeps_multiword_items_whole() {
    let h = "w0 w1 w2 w3 left old right w7 w8 w9 w10";
    let m = h.replace("old", "new");
    let source = diff(h, span_of(h, "old"), &m, span_of(&m, "new"));
    let timed: Vec<TimedWord> = h
        .split(' ')
        .enumerate()
        .map(|(index, text)| word(text, index as f64 / 4.0, index as f64 / 4.0 + 0.2))
        .collect();
    let result = align_error_diff_context(&source, &timed, CLIP).unwrap();
    assert_timing(&result, [101.25, 101.45, 100.25, 102.45]);
    let cropped = crop_aligned_error_diff_context(&source, &result).unwrap();
    assert_eq!(cropped.human.text, "… w1 w2 w3 left old right w7 w8 w9 …");
    assert_eq!(cropped.machine.text, "… w1 w2 w3 left new right w7 w8 w9 …");

    let source = same("aa bb cc dd ee ff target gg hh ii jj", "target");
    let result = align_error_diff_context(
        &source,
        &[
            word("aa bb cc dd", 0.2, 1.0),
            word("ee", 1.1, 1.25),
            word("ff", 1.3, 1.45),
            word("target", 1.5, 1.7),
            word("gg", 1.8, 2.0),
            word("hh", 2.1, 2.3),
            word("ii", 2.4, 2.6),
            word("jj", 2.7, 2.9),
        ],
        CLIP,
    )
    .unwrap();
    assert_timing(&result, [101.5, 101.7, 100.2, 102.6]);
    assert_eq!(result.asr_text, "aa bb cc dd ee ff target gg hh ii");
    assert_eq!(
        crop_aligned_error_diff_context(&source, &result)
            .unwrap()
            .human
            .text,
        "aa bb cc dd ee ff target gg hh ii …"
    );
}

#[test]
fn gaps_insertions_and_edges() {
    assert_timing(
        &align_error_diff_context(
            &diff("hello, world", (5, 6), "hello world", (5, 5)),
            &words("hello world"),
            CLIP,
        )
        .unwrap(),
        [100.5, 101.0, 100.0, 101.5],
    );
    let insertion = diff("red car", (0, 3), "car", (0, 0));
    let result = align_error_diff_context(&insertion, &words("red"), CLIP).unwrap();
    assert_timing(&result, [100.0, 100.5, 100.0, 100.5]);
    assert_timing(
        &align_error_diff_context(&diff("car", (0, 0), "red car", (0, 3)), &words("red"), CLIP)
            .unwrap(),
        timing(&result),
    );
    ambiguous(align_error_diff_context(
        &insertion,
        &words("red red"),
        CLIP,
    ));
    ambiguous(align_error_diff_context(
        &diff("red car", (0, 3), "car", (3, 3)),
        &words("red car"),
        CLIP,
    ));
    let interior =
        align_error_diff_context(&same_at("hello world", (2, 2)), &words("hello world"), CLIP)
            .unwrap();
    assert_eq!(
        (interior.diff_start_seconds, interior.diff_end_seconds),
        (100.0, 100.5)
    );
    for at in [0, 11] {
        assert!(matches!(
            align_error_diff_context(&same_at("hello world", (at, at)), &words("hello world"), CLIP),
            Err(AlignError::Miss(message)) if message.contains("Unable to align")
        ));
    }
    let overlapping = align_error_diff_context(
        &same_at("hello world", (5, 5)),
        &[word("hello", 0.0, 2.0), word("world", 1.0, 3.0)],
        CLIP,
    )
    .unwrap();
    assert_timing(&overlapping, [100.0, 103.0, 100.0, 103.0]);
    assert_eq!(
        overlapping.boundary_note,
        Some(BoundaryNote::OverlappingAsrWords)
    );
}

#[test]
fn unsupported_evidence_and_invalid_timestamps_are_rejected() {
    let source = same("we saw red cars go home", "red cars");
    ambiguous(align_error_diff_context(
        &source,
        &words("we saw green go home we saw green go home"),
        CLIP,
    ));
    assert!(matches!(
        align_error_diff_context(&source, &words("unrelated text"), CLIP),
        Err(AlignError::Miss(message)) if message.contains("Unable to align")
    ));
    let target = same("target", "target");
    for timed in [
        vec![],
        vec![word("target", 0.0, f64::NAN)],
        vec![word("target", -1.0, 0.5)],
        vec![word("target", 0.0, 11.0)],
        vec![word("target", 2.0, 1.0)],
        vec![word("target", 1.0, 1.0)],
        vec![word("target", 2.0, 3.0), word("next", 1.0, 2.0)],
        words("..."),
    ] {
        assert!(matches!(
            align_error_diff_context(&target, &timed, CLIP),
            Err(AlignError::Invalid(_))
        ));
    }
    // A word ending exactly at an absolute clip end survives float subtraction.
    let absolute = AudioInterval::new(20.1, 20.2);
    let result = align_error_diff_context(&target, &[word("target", 0.0, 0.1)], absolute).unwrap();
    assert_eq!(result.diff_end_seconds, absolute.end_seconds);
    assert!(align_error_diff_context(&target, &[word("target", 0.0, 0.1002)], absolute).is_err());
}

#[test]
fn segment_fallback_discloses_coarse_audio() {
    let source = same("we saw red cars go home", "red cars");
    let result = segment_error_diff_context(&source, &words("unrelated text"), CLIP, CLIP).unwrap();
    assert_timing(&result, [100.0, 110.0, 100.0, 110.0]);
    assert_eq!(result.boundary_note, Some(BoundaryNote::SegmentOnly));
    assert_eq!(result.asr_text, "unrelated text");
    assert_eq!(
        selected(
            &crop_aligned_error_diff_context(&source, &result)
                .unwrap()
                .human
        ),
        "red cars"
    );
    assert!(
        segment_error_diff_context(&source, &words("x"), AudioInterval::new(101.0, 110.0), CLIP)
            .is_err()
    );
    assert!(
        segment_error_diff_context(&source, &words("x"), CLIP, AudioInterval::new(100.0, 111.0))
            .is_err()
    );
}

#[test]
fn clip_miss_continues_from_observed_anchors() {
    let human = "first words before the target after final words";
    let machine = "first words before the changed after final words";
    let source = diff(
        human,
        span_of(human, "target"),
        machine,
        span_of(machine, "changed"),
    );
    let short = AudioInterval::new(100.0, 108.0);
    let later =
        align_error_diff_context(&source, &words("first words before the"), short).unwrap_err();
    assert!(matches!(
        later,
        AlignError::ClipMiss {
            direction: ClipDirection::After,
            ..
        }
    ));
    assert_eq!(
        next_error_diff_clip(&later, short, AudioInterval::new(90.0, 130.0)).unwrap(),
        AudioInterval::new(102.0, 112.0)
    );
    let earlier = align_error_diff_context(&source, &words("after final words"), CLIP).unwrap_err();
    assert!(matches!(
        earlier,
        AlignError::ClipMiss {
            direction: ClipDirection::Before,
            ..
        }
    ));
    assert_eq!(
        next_error_diff_clip(&earlier, CLIP, AudioInterval::new(90.0, 130.0)).unwrap(),
        AudioInterval::new(91.5, 101.5)
    );
    assert!(next_error_diff_clip(&later, short, AudioInterval::new(90.0, 108.0)).is_err());
}

// ---------------------------------------------------------------------------------------
// Payload parsing

fn error_payload(source: &str, focus_row: &str) -> Value {
    json!([{ "result": { "data": { "json": {
        "actionId": "11111111-2222-4333-8444-555555555555",
        "actionLevel": 1,
        "actionWorkerId": "worker-1",
        "actionDecision": "in-progress",
        "transcriptionChunkProcessedRecordings": [
            { "transcriptionChunkId": "chunk", "processedRecordingId": "pr-1", "chunkedProcessedRecordingId": "cpr-1", "speaker": 1 },
            { "transcriptionChunkId": "chunk", "processedRecordingId": "pr-2", "chunkedProcessedRecordingId": "cpr-2", "speaker": 2 }
        ],
        "processedRecordingUriMap": { "cpr-1": "audio/speaker-1.wav", "cpr-2": "audio/speaker-2.wav" },
        "annotations": [
            { "id": "row-1", "processedRecordingId": "pr-1", "startTimeInSeconds": 0.5, "endTimeInSeconds": 3.0, "content": "a" },
            { "id": "row-2", "processedRecordingId": "pr-2", "startTimeInSeconds": 3.5, "endTimeInSeconds": 9.0, "content": "b" }
        ],
        "transcriptionError": {
            "id": "diff-1", "segmentId": focus_row, "language": "ru", "source": source,
            "diffType": "substitution",
            "humanNormalizedText": "😀 он был недовольный", "humanNormalizedSpanStart": 10, "humanNormalizedSpanEnd": 21,
            "machineNormalizedText": "😀 он был недовольны", "machineNormalizedSpanStart": 10, "machineNormalizedSpanEnd": 20,
            "priorLabels": [
                { "level": 1, "capturesWholeError": true, "meaningChanged": false, "humanTranscriptWrong": false, "categoryKeys": ["affix"], "notes": null, "otherIssues": null }
            ],
            "submittedLabel": null,
            "isQaReview": true
        }
    } } } }])
}

#[test]
fn real_task_focuses_the_diff_segment_and_its_own_recording() {
    let payload = error_payload("real", "row-2").to_string();
    assert!(payload_is_error_task(&payload));
    let task = parse_error_task(&payload).unwrap();
    assert_eq!(
        task.review_action_id,
        "11111111-2222-4333-8444-555555555555"
    );
    assert_eq!(task.error.source, ErrorSource::Real);
    assert!(task.error.is_qa_review);
    assert_eq!(task.error.prior_labels[0].category_keys, vec!["affix"]);
    assert_eq!(selected(&task.error.diff.human), "недовольный");
    assert_eq!(selected(&task.error.diff.machine), "недовольны");
    let focus = task.focus.expect("real tasks have a focus segment");
    assert_eq!(focus.annotation_id, "row-2");
    assert_eq!(focus.processed_recording_id, "pr-2");
    assert_eq!(focus.audio_path.as_deref(), Some("audio/speaker-2.wav"));
    assert_eq!((focus.start_seconds, focus.end_seconds), (3.5, 9.0));
    assert_eq!(focus.speaker_label(), "Speaker 2");
}

#[test]
fn synthetic_tasks_are_text_only_and_bad_payloads_fail() {
    let task = parse_error_task(&error_payload("synthetic", "missing-row").to_string()).unwrap();
    assert_eq!(task.error.source, ErrorSource::Synthetic);
    assert!(task.focus.is_none());
    assert!(parse_error_task(&error_payload("real", "missing-row").to_string()).is_err());
    let mut bad_span = error_payload("real", "row-1");
    bad_span[0]["result"]["data"]["json"]["transcriptionError"]["humanNormalizedSpanEnd"] =
        json!(99);
    assert!(parse_error_task(&bad_span.to_string()).is_err());
    let failed =
        json!({ "json": { "success": false, "error": "No tasks available in this queue" } });
    assert_eq!(
        parse_error_task(&failed.to_string()).unwrap_err().message,
        "No tasks available in this queue"
    );
    assert!(!payload_is_error_task(
        r#"{"json":{"actionId":"a","transcriptionError":null}}"#
    ));
}

#[test]
fn viewed_tasks_are_editable_only_while_this_workers_in_progress_action() {
    let task = parse_error_task(&error_payload("real", "row-1").to_string()).unwrap();
    assert!(!task.viewed_read_only(Some("worker-1")));
    assert!(task.viewed_read_only(Some("worker-2")));
    assert!(task.viewed_read_only(None));
}

// ---------------------------------------------------------------------------------------
// Questionnaire

fn keys(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

#[test]
fn pairing_rules_follow_native_priority_order() {
    assert_eq!(pairing_problem(&keys(&["affix"])), None);
    assert_eq!(
        pairing_problem(&keys(&["hard_to_hear", "spelling"])),
        Some("hard_to_hear can't be combined with another category")
    );
    // Order of selection does not matter: pairs are evaluated in native order.
    assert_eq!(
        pairing_problem(&keys(&["different_word", "spelling"])),
        Some("a second category can't be from Missing or extra words or Wrong words, except affix")
    );
    assert_eq!(
        pairing_problem(&keys(&["spelling", "affix"])),
        Some("a category from the first four groups only pairs with another from them")
    );
    assert_eq!(pairing_problem(&keys(&["named_entities", "affix"])), None);
    assert_eq!(pairing_problem(&keys(&["spelling", "numbers"])), None);
    assert_eq!(pairing_problem(&keys(&["not_sure", "typo"])), None);
    assert_eq!(
        pairing_problem(&keys(&["a", "b", "c"])),
        Some("pick at most two categories")
    );
    assert!(valid_categories(&keys(&["named_entities", "affix"])));
    assert!(!valid_categories(&keys(&["affix", "affix"])));
    assert!(!valid_categories(&keys(&["unknown"])));
    assert!(!valid_categories(&[]));
    let choices = ErrorChoices {
        category_keys: keys(&["spelling"]),
        ..ErrorChoices::default()
    };
    assert!(choices.category_blocked("different_word"));
    assert!(!choices.category_blocked("numbers"));
    assert!(!choices.category_blocked("spelling"));
}

#[test]
fn submission_payload_matches_babel_web() {
    let id = "11111111-2222-4333-8444-555555555555";
    let whole = ErrorChoices {
        captures_whole_error: Some(true),
        meaning_changed: Some(true),
        human_transcript_wrong: Some(false),
        category_keys: keys(&["affix"]),
        other_issues: "  ".to_string(),
        notes: " Keep my manual notes ".to_string(),
    };
    let envelope =
        BabelRequest::SubmitTranscriptionErrorLabel(submission(id, &whole, ErrorSource::Real))
            .envelope();
    assert_eq!(
        envelope,
        json!({
            "operation": "transcriptions.submitTranscriptionErrorLabel",
            "payload": {
                "reviewActionId": id, "capturesWholeError": true, "meaningChanged": true,
                "humanTranscriptWrong": false, "categoryKeys": ["affix"], "notes": "Keep my manual notes"
            }
        })
    );
    // Synthetic diffs have no human-transcript question.
    assert_eq!(
        serde_json::to_value(submission(id, &whole, ErrorSource::Synthetic)).unwrap(),
        json!({ "reviewActionId": id, "capturesWholeError": true, "meaningChanged": true,
            "categoryKeys": ["affix"], "notes": "Keep my manual notes" })
    );
    // "No" discards the diff: no categories or answers, only the optional notes.
    let discard = ErrorChoices {
        captures_whole_error: Some(false),
        ..whole
    };
    assert_eq!(
        serde_json::to_value(submission(id, &discard, ErrorSource::Real)).unwrap(),
        json!({ "reviewActionId": id, "capturesWholeError": false, "categoryKeys": [],
            "notes": "Keep my manual notes" })
    );
    assert_eq!(
        BabelRequest::ClaimTranscriptionErrorQaTask {
            transcription_error_diff_id: "diff-1".to_string(),
            queue_id: "queue-1".to_string(),
        }
        .envelope(),
        json!({ "operation": "transcriptions.claimTranscriptionErrorQaTask",
            "payload": { "transcriptionErrorDiffId": "diff-1", "queueId": "queue-1" } })
    );
}

#[test]
fn readiness_reports_missing_items_then_rule_violations() {
    let mut choices = ErrorChoices::default();
    assert_eq!(
        readiness(&choices, ErrorSource::Real),
        Err("Missing: first question".to_string())
    );
    choices.captures_whole_error = Some(false);
    assert_eq!(readiness(&choices, ErrorSource::Real), Ok(()));
    choices.captures_whole_error = Some(true);
    assert_eq!(
        readiness(&choices, ErrorSource::Real),
        Err("Missing: category, 2 questions".to_string())
    );
    assert_eq!(
        readiness(&choices, ErrorSource::Synthetic),
        Err("Missing: category, 1 question".to_string())
    );
    choices.meaning_changed = Some(false);
    choices.human_transcript_wrong = Some(true);
    choices.category_keys = keys(&["spelling", "different_word"]);
    assert_eq!(
        readiness(&choices, ErrorSource::Real),
        Err(
            "A second category can't be from Missing or extra words or Wrong words, except affix."
                .to_string()
        )
    );
    choices.category_keys = keys(&["hard_to_hear"]);
    assert_eq!(readiness(&choices, ErrorSource::Real), Ok(()));
    assert_eq!(
        readiness(&choices, ErrorSource::Synthetic),
        Err("A synthetic diff has no audio, so hard_to_hear doesn't apply.".to_string())
    );
}

#[test]
fn initial_fill_fills_only_unanswered_choices() {
    let draft = ErrorLabelDraft {
        captures_whole_error: true,
        meaning_changed: false,
        human_transcript_wrong: false,
        category_keys: keys(&["affix"]),
    };
    let baseline = ErrorChoices {
        meaning_changed: Some(true),
        notes: "manual".to_string(),
        other_issues: "issue".to_string(),
        ..ErrorChoices::default()
    };
    let filled = apply_draft(&baseline, &baseline, &draft).unwrap().unwrap();
    assert_eq!(filled.captures_whole_error, Some(true));
    assert_eq!(
        filled.meaning_changed,
        Some(true),
        "a manual answer is kept"
    );
    assert_eq!(filled.human_transcript_wrong, Some(false));
    assert_eq!(filled.category_keys, keys(&["affix"]));
    assert_eq!(
        (filled.notes.as_str(), filled.other_issues.as_str()),
        ("manual", "issue")
    );
    // Everything answered: nothing to apply.
    assert_eq!(apply_draft(&filled, &filled, &draft).unwrap(), None);
    // The user edited while the provider ran: nothing is overwritten.
    let edited = ErrorChoices {
        notes: "edited".to_string(),
        ..baseline.clone()
    };
    assert!(apply_draft(&baseline, &edited, &draft).is_err());
    // A manual "No" keeps the diff discarded.
    let discard = ErrorChoices {
        captures_whole_error: Some(false),
        ..ErrorChoices::default()
    };
    let kept = apply_draft(&discard, &discard, &draft).unwrap();
    assert_eq!(kept, None);
    // Draft categories that clash with a manual first category are refused.
    let manual = ErrorChoices {
        captures_whole_error: Some(true),
        category_keys: keys(&["hard_to_hear", "affix"]),
        ..ErrorChoices::default()
    };
    assert!(apply_draft(&manual, &manual, &draft).is_err());
}

// ---------------------------------------------------------------------------------------
// Decisions response and style guide

fn answers(pairs: &[(&str, &str)]) -> Value {
    let mut answers = serde_json::Map::new();
    for (key, choice) in pairs {
        answers.insert(
            key.to_string(),
            json!({ "type": "choice", "choice": choice }),
        );
    }
    json!({ "id": "gen-1", "model": "cloudflare/clef-omni", "answers": answers })
}

const AFFIX: [(&str, &str); 6] = [
    ("errorOutsideHighlight", "no"),
    ("onlyNonSemanticDifference", "yes"),
    ("humanTranscriptWrong", "no"),
    ("primaryCategory", "affix"),
    ("hasSecondaryCategory", "no"),
    ("secondaryCategory", "none"),
];

#[test]
fn decision_answers_are_validated_exactly() {
    let draft = parse_decision_response(&answers(&AFFIX)).unwrap();
    assert_eq!(
        draft,
        ErrorLabelDraft {
            captures_whole_error: true,
            meaning_changed: false,
            human_transcript_wrong: false,
            category_keys: keys(&["affix"]),
        }
    );
    let mut outside = AFFIX;
    outside[0].1 = "yes";
    assert!(
        parse_decision_response(&answers(&outside))
            .unwrap()
            .category_keys
            .is_empty()
    );
    let mut secondary = AFFIX;
    secondary[4].1 = "yes";
    assert!(
        parse_decision_response(&answers(&secondary)).is_err(),
        "yes + none"
    );
    secondary[5].1 = "numbers";
    secondary[3].1 = "spelling";
    assert_eq!(
        parse_decision_response(&answers(&secondary))
            .unwrap()
            .category_keys,
        keys(&["spelling", "numbers"])
    );
    // Valid choices that break the pairing rules are rejected (paid output retained).
    secondary[3].1 = "named_entities";
    secondary[5].1 = "spelling";
    assert!(parse_decision_response(&answers(&secondary)).is_err());
    secondary[5].1 = "different_word";
    assert!(
        parse_decision_response(&answers(&secondary)).is_err(),
        "not a secondary choice"
    );
    let mut bad = AFFIX;
    bad[3].1 = "made_up";
    assert!(parse_decision_response(&answers(&bad)).is_err());
    assert!(parse_decision_response(&answers(&AFFIX[..5])).is_err());
    let mut wrong_model = answers(&AFFIX);
    wrong_model["model"] = json!("other/model");
    assert!(parse_decision_response(&wrong_model).is_err());
}

#[test]
fn style_guide_is_the_single_task_tool_markdown_guide() {
    let doc = |id: &str, label: &str, shown: bool| json!({ "documentId": id, "label": label, "name": "doc", "kind": "markdown", "showInTaskTool": shown });
    assert_eq!(
        select_style_guide_document(&json!([
            doc("a", "Glossary", true),
            doc("b", " style  GUIDE ", true),
            doc("c", "Style Guide", false)
        ]))
        .unwrap(),
        "b"
    );
    assert!(
        select_style_guide_document(&json!([
            doc("a", "Style Guide", true),
            doc("b", "Style guide", true)
        ]))
        .is_err()
    );
    assert!(select_style_guide_document(&json!([])).is_err());
    let document = json!({ "documentId": "b", "kind": "markdown",
        "latest": { "version": 3, "createdAt": "2026-10-01T12:00:00.000Z" },
        "body": { "kind": "markdown", "content": "# Policy\nJudge only the highlight." } });
    let guide = parse_style_guide_document("project-1", "b", &document).unwrap();
    assert_eq!(
        (guide.version, guide.content.as_str()),
        (3, "# Policy\nJudge only the highlight.")
    );
    let mut empty = document.clone();
    empty["body"]["content"] = json!("   ");
    assert!(parse_style_guide_document("project-1", "b", &empty).is_err());
    assert!(parse_style_guide_document("project-1", "other", &document).is_err());
}

// ---------------------------------------------------------------------------------------
// Assistant: alignment windows and paid decisions

/// One scripted ASR answer: `(text, start, end)` clip-relative words.
type ScriptedWords = Vec<(&'static str, f64, f64)>;

/// Scripted clip ASR: answers each align call with the next word list (clip-relative).
struct ScriptedAsr {
    responses: Mutex<VecDeque<ScriptedWords>>,
    durations: Mutex<Vec<f64>>,
}

impl ScriptedAsr {
    fn new(responses: Vec<ScriptedWords>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into()),
            durations: Mutex::new(Vec::new()),
        })
    }
}

fn wav_duration(wav: &[u8]) -> f64 {
    let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
    let data = u32::from_le_bytes(wav[40..44].try_into().unwrap());
    f64::from(data) / 2.0 / f64::from(rate)
}

#[async_trait]
impl InferenceBackend for ScriptedAsr {
    async fn status(&self) -> BackendStatus {
        unreachable!()
    }
    async fn timing(
        &self,
        _: &str,
        _: &[LaneAudio],
        _: ProgressSink,
    ) -> InferenceResult<TimingResponse> {
        unreachable!()
    }
    async fn draft(
        &self,
        _: &str,
        _: &[LaneAudio],
        _: &TimingResponse,
        _: Option<Vec<PreservedRow>>,
        _: ProgressSink,
    ) -> InferenceResult<DraftResponse> {
        unreachable!()
    }
    async fn enhance(
        &self,
        _: &str,
        _: &[LaneAudio],
        _: ProgressSink,
    ) -> InferenceResult<Vec<EnhancedLane>> {
        unreachable!()
    }
    async fn align(
        &self,
        request: AlignRequest,
        _: ProgressSink,
    ) -> InferenceResult<AlignResponse> {
        let duration = wav_duration(&request.wav);
        assert!(
            duration <= 10.0,
            "error-task clips stay at most ten seconds"
        );
        assert_eq!(request.lane, "pr-2");
        assert!(request.task_id.starts_with("error-task-clip:"));
        self.durations.lock().unwrap().push(duration);
        let words = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted response");
        Ok(AlignResponse {
            tokens: words
                .into_iter()
                .enumerate()
                .map(|(index, (text, start, end))| TimingToken {
                    id: index.to_string(),
                    text: text.to_string(),
                    start_seconds: start,
                    end_seconds: end,
                })
                .collect(),
            duration_seconds: Some(duration),
            model: "gold-test-release".to_string(),
        })
    }
}

fn silence(seconds: usize) -> Arc<DecodedAudio> {
    Arc::new(DecodedAudio::from_f32_channels(16_000, vec![vec![0.0; 16_000 * seconds]]).unwrap())
}

fn identity() -> ErrorTaskIdentity {
    ErrorTaskIdentity {
        review_action_id: "ra-1".to_string(),
        diff_id: "diff-1".to_string(),
        annotation_id: "row-2".to_string(),
        track_id: "pr-2".to_string(),
    }
}

fn snapshot(source: NormalizedDiff) -> ErrorTaskSnapshot {
    ErrorTaskSnapshot {
        identity: identity(),
        language: "ru".to_string(),
        diff: source,
        segment: AudioInterval::new(0.5, 11.5),
    }
}

fn backend(asr: Arc<ScriptedAsr>, location: ExecutionLocation) -> AlignBackend {
    AlignBackend {
        backend: asr,
        location,
        remote_base_url: "https://coordinator.invalid".to_string(),
        secrets: vec!["sk-or-test".to_string()],
    }
}

fn spaced(
    items: &[&'static str],
    start: f64,
    step: f64,
    length: f64,
) -> Vec<(&'static str, f64, f64)> {
    items
        .iter()
        .enumerate()
        .map(|(index, text)| {
            (
                *text,
                start + index as f64 * step,
                start + index as f64 * step + length,
            )
        })
        .collect()
}

const PREFIX: &str =
    "сначала мы решили обсудить новый фильм после вечера потом он сказал что этот актёр всегда был";
const SUFFIX: &str =
    "из-за своих проблем но при этом очень хорошо сыграл роль и нам всем понравилась эта история";

fn substitution() -> NormalizedDiff {
    let human = format!("{PREFIX} недовольный {SUFFIX}");
    let machine = format!("{PREFIX} недовольны {SUFFIX}");
    diff(
        &human,
        span_of(&human, "недовольный"),
        &machine,
        span_of(&machine, "недовольны"),
    )
}

fn guide() -> StyleGuide {
    StyleGuide {
        project_id: "project-1".to_string(),
        id: "guide-1".to_string(),
        version: 2,
        created_at: "2026-10-01T00:00:00.000Z".to_string(),
        content: "# Synthetic error-labeling policy\nJudge only the highlighted error.".to_string(),
    }
}

fn collect() -> (
    Arc<Mutex<Vec<AssistantStatus>>>,
    impl Fn(AssistantStatus) + Send + Sync,
) {
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&statuses);
    (statuses, move |status| sink.lock().unwrap().push(status))
}

#[tokio::test]
async fn alignment_finds_the_substitution_in_the_first_window() {
    let speech = [
        "что",
        "этот",
        "актёр",
        "всегда",
        "был",
        "недовольный",
        "из-за",
        "своих",
        "проблем",
        "но",
        "при",
    ];
    let asr = ScriptedAsr::new(vec![spaced(&speech, 0.3, 0.55, 0.4)]);
    let (statuses, status) = collect();
    let aligned = find_alignment(
        &snapshot(substitution()),
        silence(12),
        &backend(Arc::clone(&asr), ExecutionLocation::Local),
        &CancelToken::new(),
        &status,
    )
    .await
    .unwrap();
    let alignment = aligned.alignment;
    assert!((alignment.diff_end_seconds - alignment.diff_start_seconds - 0.4).abs() < 1e-3);
    assert!(alignment.diff_start_seconds > 3.0);
    assert_eq!(
        alignment.asr_text,
        "этот актёр всегда был недовольный из-за своих проблем но"
    );
    let cropped = crop_aligned_error_diff_context(&substitution(), &alignment).unwrap();
    assert_eq!(
        cropped.human.text.replace('…', "").trim(),
        "этот актёр всегда был недовольный из-за своих проблем но"
    );
    assert_eq!(asr.durations.lock().unwrap().len(), 1);
    assert_eq!(aligned.receipt.provider, "local");
    let statuses = statuses.lock().unwrap();
    assert_eq!(statuses[0].message, "Finding the words 1/3…");
    assert_eq!(
        statuses.last().unwrap().message,
        "Audio found. Check it and the choices before Submit."
    );
}

fn deletion() -> NormalizedDiff {
    let h = "до старта я не буду меняться и подстраиваться и потом продолжу обсуждение сегодня";
    let at = u16_index(h, " и подстраиваться") + 1;
    let units: Vec<u16> = h.encode_utf16().collect();
    let m = String::from_utf16(&[&units[..at], &units[at + 2..]].concat()).unwrap();
    diff(h, (at, at + 1), &m, (at, at))
}

#[tokio::test]
async fn a_clip_miss_moves_the_window_and_initial_fill_sends_the_aligned_crop() {
    let before = spaced(&["до", "старта", "я", "не", "буду"], 0.1, 0.35, 0.2);
    let containing = vec![
        ("буду", 0.2, 0.45),
        ("меняться", 0.7, 1.1),
        ("подстраиваться", 1.4, 1.8),
        ("и", 2.0, 2.15),
        ("потом", 2.3, 2.6),
    ];
    let asr = ScriptedAsr::new(vec![before, containing]);
    let snapshot = snapshot(deletion());
    let audio = silence(12);
    let (statuses, status) = collect();
    let aligned = find_alignment(
        &snapshot,
        Arc::clone(&audio),
        &backend(Arc::clone(&asr), ExecutionLocation::Local),
        &CancelToken::new(),
        &status,
    )
    .await
    .unwrap();
    assert_eq!(asr.durations.lock().unwrap().len(), 2);
    assert!(
        statuses
            .lock()
            .unwrap()
            .iter()
            .any(|status| status.message == "Looking later in the audio…")
    );
    let alignment = aligned.alignment;
    assert!((alignment.context_end_seconds - alignment.context_start_seconds - 2.4).abs() < 1e-3);
    assert_eq!(alignment.asr_text, "буду меняться подстраиваться и потом");

    let server = DecisionServer::start(false).await;
    let store = tempfile::tempdir().unwrap();
    let request = InitialFillRequest {
        snapshot: snapshot.clone(),
        alignment: alignment.clone(),
        style_guide: guide(),
        api_key: "sk-or-test".to_string(),
        store: DecisionStore::new(store.path()),
        endpoint: server.url.clone(),
    };
    let (status_log, status) = collect();
    let (draft, receipt) = initial_fill_decision(request, audio, &CancelToken::new(), &status)
        .await
        .unwrap();
    assert_eq!(draft.category_keys, keys(&["affix"]));
    assert_eq!(receipt.id.as_deref(), Some("gen-dec-1"));
    assert_eq!(receipt.cost, Some(0.000015));
    assert_eq!(status_log.lock().unwrap()[0].message, "Filling choices…");
    let bodies = server.bodies();
    assert_eq!(bodies.len(), 1);
    let body = &bodies[0];
    assert_eq!(server.authorization(), "Bearer sk-or-test");
    assert_eq!(body["model"], "cloudflare/clef-omni");
    assert_eq!(body["provider"], json!({ "allow_fallbacks": false }));
    let questions: Vec<&String> = body["questions"].as_object().unwrap().keys().collect();
    assert_eq!(questions.len(), 6);
    let prompt = &body["state"][0];
    assert_eq!(
        prompt["selectedComparison"],
        json!({ "human": "и", "machine": "" })
    );
    assert_eq!(
        prompt["surroundingContext"],
        json!({ "before": "… буду меняться ", "after": " подстраиваться и потом …" })
    );
    assert_eq!(
        prompt["asrTranscript"],
        "буду меняться подстраиваться и потом"
    );
    let audio_part = &body["state"][1];
    assert_eq!(audio_part["type"], "input_audio");
    let wav = base64_decode(audio_part["input_audio"]["data"].as_str().unwrap());
    assert!(
        (wav_duration(&wav) - (alignment.context_end_seconds - alignment.context_start_seconds))
            .abs()
            < 1e-3
    );
    assert!(
        body["state"][2]
            .as_str()
            .unwrap()
            .starts_with("Current project style guide:\n# Synthetic")
    );
}

#[tokio::test]
async fn local_alignment_is_russian_only_without_fallback() {
    let asr = ScriptedAsr::new(Vec::new());
    let mut english = snapshot(substitution());
    english.language = "en".to_string();
    let (_, status) = collect();
    let error = find_alignment(
        &english,
        silence(12),
        &backend(Arc::clone(&asr), ExecutionLocation::Local),
        &CancelToken::new(),
        &status,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "unsupported-language");
    assert!(asr.durations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_short_segment_falls_back_to_the_whole_segment() {
    let asr = ScriptedAsr::new(vec![vec![("совсем", 0.1, 0.5), ("другое", 0.6, 1.0)]]);
    let mut short = snapshot(same("мы видели красные машины", "красные"));
    short.segment = AudioInterval::new(1.0, 5.0);
    let (_, status) = collect();
    let aligned = find_alignment(
        &short,
        silence(12),
        &backend(asr, ExecutionLocation::Remote),
        &CancelToken::new(),
        &status,
    )
    .await
    .unwrap();
    assert_eq!(
        aligned.alignment.boundary_note,
        Some(BoundaryNote::SegmentOnly)
    );
    assert_eq!(aligned.receipt.provider, "remote");
    assert!((aligned.alignment.context_start_seconds - 1.0).abs() < 1e-3);
    assert!((aligned.alignment.context_end_seconds - 5.0).abs() < 1e-3);
}

fn base64_decode(text: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .unwrap()
}

/// Loopback OpenRouter decisions endpoint: records bodies, answers `AFFIX`, optionally
/// holds every reply until released, or fails with HTTP 500.
#[derive(Clone)]
struct DecisionServer {
    url: String,
    state: Arc<ServerState>,
}

struct ServerState {
    bodies: Mutex<Vec<Value>>,
    authorization: Mutex<String>,
    hold: tokio::sync::Semaphore,
    fail: bool,
}

impl DecisionServer {
    async fn start(hold: bool) -> Self {
        Self::launch(hold, false).await
    }

    async fn launch(hold: bool, fail: bool) -> Self {
        let state = Arc::new(ServerState {
            bodies: Mutex::new(Vec::new()),
            authorization: Mutex::new(String::new()),
            hold: tokio::sync::Semaphore::new(if hold { 0 } else { 1_000 }),
            fail,
        });
        let app = Router::new()
            .route("/api/alpha/decisions", post(handle))
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/alpha/decisions",
            listener.local_addr().unwrap()
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, state }
    }

    fn bodies(&self) -> Vec<Value> {
        self.state.bodies.lock().unwrap().clone()
    }

    fn authorization(&self) -> String {
        self.state.authorization.lock().unwrap().clone()
    }

    fn release(&self) {
        self.state.hold.add_permits(1);
    }
}

async fn handle(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    body: String,
) -> (axum::http::StatusCode, String) {
    *state.authorization.lock().unwrap() = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    state
        .bodies
        .lock()
        .unwrap()
        .push(serde_json::from_str(&body).unwrap());
    state.hold.acquire().await.unwrap().forget();
    if state.fail {
        return (axum::http::StatusCode::BAD_GATEWAY, "{}".to_string());
    }
    let mut reply = answers(&AFFIX);
    reply["id"] = json!("gen-dec-1");
    reply["usage"] = json!({ "input_tokens": 100, "output_tokens": 0, "cost": 0.000015 });
    (axum::http::StatusCode::OK, reply.to_string())
}

fn fill_request(
    server: &DecisionServer,
    store: &std::path::Path,
    source: NormalizedDiff,
) -> InitialFillRequest {
    let alignment = DiffAlignment {
        diff_start_seconds: 1.0,
        diff_end_seconds: 1.5,
        context_start_seconds: 0.5,
        context_end_seconds: 2.5,
        text_window: exaudio_babel_services::babel::error_task::TextWindow {
            human_start: 0,
            human_end: u16_len(&source.human.text),
            machine_start: 0,
            machine_end: u16_len(&source.machine.text),
        },
        boundary_note: None,
        asr_text: "мы видели красные машины".to_string(),
    };
    InitialFillRequest {
        snapshot: ErrorTaskSnapshot {
            segment: AudioInterval::new(0.0, 3.0),
            ..snapshot(source)
        },
        alignment,
        style_guide: guide(),
        api_key: "sk-or-test".to_string(),
        store: DecisionStore::new(store),
        endpoint: server.url.clone(),
    }
}

#[tokio::test]
async fn a_paid_result_is_cached_and_cancel_keeps_it_for_the_next_fill() {
    let server = DecisionServer::start(true).await;
    let store = tempfile::tempdir().unwrap();
    let request = || {
        fill_request(
            &server,
            store.path(),
            same("мы видели красные машины", "красные"),
        )
    };
    let (_, status) = collect();
    let cancel = CancelToken::new();
    let first = tokio::spawn({
        let request = request();
        let cancel = cancel.clone();
        async move { initial_fill_decision(request, silence(4), &cancel, &|_| {}).await }
    });
    // Cancel once the paid request is on the wire.
    for _ in 0..200 {
        if !server.bodies().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(server.bodies().len(), 1);
    cancel.cancel();
    let error = first.await.unwrap().unwrap_err();
    assert!(error.is_cancelled());
    assert!(error.message.contains("retained"), "{}", error.message);
    // The dispatched request finishes into the durable cache.
    server.release();
    let mut reused = None;
    for _ in 0..200 {
        if std::fs::read_dir(store.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| {
                std::fs::read_to_string(entry.path())
                    .unwrap_or_default()
                    .contains("completed")
            })
        {
            reused = Some(
                initial_fill_decision(request(), silence(4), &CancelToken::new(), &status)
                    .await
                    .unwrap(),
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (draft, _) = reused.expect("cached paid result");
    assert_eq!(draft.category_keys, keys(&["affix"]));
    assert_eq!(server.bodies().len(), 1, "no second paid dispatch");
}

#[tokio::test]
async fn an_uncertain_paid_outcome_is_never_silently_reissued() {
    let server = DecisionServer::launch(false, true).await;
    let store = tempfile::tempdir().unwrap();
    let source = same("мы видели синие машины", "синие");
    let first = initial_fill_decision(
        fill_request(&server, store.path(), source.clone()),
        silence(4),
        &CancelToken::new(),
        &|_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(first.code, "outcome-uncertain");
    let second = initial_fill_decision(
        fill_request(&server, store.path(), source),
        silence(4),
        &CancelToken::new(),
        &|_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(second.code, "outcome-uncertain");
    assert_eq!(server.bodies().len(), 1);
}

#[tokio::test]
async fn initial_fill_requires_a_key_and_never_sends_one_elsewhere() {
    let server = DecisionServer::start(false).await;
    let store = tempfile::tempdir().unwrap();
    let mut request = fill_request(
        &server,
        store.path(),
        same("мы видели белые машины", "белые"),
    );
    request.api_key = " ".to_string();
    let error = initial_fill_decision(request, silence(4), &CancelToken::new(), &|_| {})
        .await
        .unwrap_err();
    assert_eq!(error.code, "missing-key");
    assert!(server.bodies().is_empty());
}

#[test]
fn the_decisions_endpoint_override_is_honored_only_on_loopback() {
    const OPENROUTER: &str = "https://openrouter.ai/api/alpha/decisions";
    assert_eq!(decisions_url_with(None), OPENROUTER);
    assert_eq!(
        decisions_url_with(Some(" http://127.0.0.1:8790/api/alpha/decisions ")),
        "http://127.0.0.1:8790/api/alpha/decisions"
    );
    assert_eq!(
        decisions_url_with(Some("http://localhost:8790/decisions")),
        "http://localhost:8790/decisions"
    );
    for elsewhere in [
        "https://evil.example/decisions",
        "http://127.0.0.1.evil.example/decisions",
        "http://10.0.0.2:8790/decisions",
        "file:///C:/decisions",
        "not a url",
    ] {
        assert_eq!(
            decisions_url_with(Some(elsewhere)),
            OPENROUTER,
            "{elsewhere}"
        );
    }
}
