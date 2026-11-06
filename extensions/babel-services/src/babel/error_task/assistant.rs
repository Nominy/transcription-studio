//! The error-task assistant flow (Gold `error-task-content.ts`): locate the diff in the
//! target speaker's audio (up to three nearby ≤ 10 s windows, then the whole segment when
//! it is ≤ 10 s), then Initial fill from the aligned clip, comparison and style guide.

use std::sync::Arc;

use bytes::Bytes;
use studio_waveform::DecodedAudio;

use super::broker::{
    self, AlignBackend, CancelToken, ClipWav, DecisionInput, DecisionStore, ErrorTaskProgress,
    Receipt,
};
use super::choices::ErrorLabelDraft;
use super::clef::DecisionComparison;
use super::diff_align::{
    AlignError, AudioInterval, BoundaryNote, ClipDirection, DiffAlignment, NormalizedDiff,
    align_error_diff_context, crop_aligned_error_diff_context, crop_error_diff_context,
    estimate_error_diff_clip, next_error_diff_clip, segment_error_diff_context, valid_alignment,
};
use super::style_guide::StyleGuide;
use super::wav::encode_clip_wav;
use super::{ErrorTaskError, ErrorTaskIdentity};

/// Decoded target-speaker recording the clips are cut from.
pub type ClipSource = Arc<DecodedAudio>;

/// The task inputs one assistant run is bound to (Gold `ErrorTaskSnapshot`). Results of a
/// run apply only while the open task still has this snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorTaskSnapshot {
    pub identity: ErrorTaskIdentity,
    pub language: String,
    pub diff: NormalizedDiff,
    /// The diff's segment (absolute seconds in the target recording).
    pub segment: AudioInterval,
}

/// The located diff plus the receipt of the ASR call that found it.
#[derive(Debug, Clone, PartialEq)]
pub struct AlignedDiff {
    pub alignment: DiffAlignment,
    pub receipt: Receipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantPhase {
    WaitingAudio,
    Aligning,
    Aligned,
    Drafting,
    Filled,
    Error,
    Cancelled,
}

impl AssistantPhase {
    /// Phases with work in flight.
    pub fn is_busy(self) -> bool {
        matches!(self, Self::Aligning | Self::Drafting)
    }
}

/// What the assistant shows next to Initial fill.
#[derive(Debug, Clone, PartialEq)]
pub struct AssistantStatus {
    pub phase: AssistantPhase,
    pub message: String,
    pub receipt: Option<Receipt>,
}

impl AssistantStatus {
    pub fn new(phase: AssistantPhase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
            receipt: None,
        }
    }
}

/// Gold appends the coarse-alignment caveat to every later message.
pub fn status_with_boundary_note(
    phase: AssistantPhase,
    message: &str,
    alignment: Option<&DiffAlignment>,
) -> String {
    match alignment.and_then(|alignment| alignment.boundary_note) {
        Some(_) if phase == AssistantPhase::Aligned => message.to_string(),
        Some(BoundaryNote::OverlappingAsrWords) => {
            format!("{message} Words overlap here; check the audio.")
        }
        Some(BoundaryNote::SegmentOnly) => {
            format!("{message} Using the whole segment; check the audio.")
        }
        None => message.to_string(),
    }
}

fn progress_message(label: &str, progress: ErrorTaskProgress) -> String {
    format!("{label}… {}s", progress.elapsed.as_secs())
}

fn capture(audio: &DecodedAudio, range: AudioInterval) -> Result<ClipWav, ErrorTaskError> {
    let (wav, interval) = encode_clip_wav(audio, range)?;
    let frames = (wav.len() - 44) / 2;
    Ok(ClipWav {
        wav: Bytes::from(wav),
        interval,
        sample_rate: audio.sample_rate(),
        frames,
    })
}

fn align_failure(error: AlignError) -> ErrorTaskError {
    ErrorTaskError::new("alignment-failed", error.to_string())
}

/// Accepts an alignment only inside the audio it came from (Gold native `alignment` ack).
fn acknowledge(
    diff: &NormalizedDiff,
    alignment: DiffAlignment,
    clip: &ClipWav,
) -> Result<DiffAlignment, ErrorTaskError> {
    if !valid_alignment(&alignment)
        || alignment.context_start_seconds < clip.interval.start_seconds - 0.001
        || alignment.context_end_seconds > clip.interval.end_seconds + 0.001
        || crop_aligned_error_diff_context(diff, &alignment).is_err()
    {
        return Err(ErrorTaskError::new(
            "invalid-alignment",
            "The words fall outside the captured audio.",
        ));
    }
    Ok(alignment)
}

/// Locates the diff in `audio` (Gold `ensureAlignment`): rough crop, ASR, joint word
/// alignment; a proven clip miss continues from the observed words (three windows at most);
/// any miss on a segment of at most ten seconds falls back to the whole segment.
pub async fn find_alignment(
    snapshot: &ErrorTaskSnapshot,
    audio: ClipSource,
    backend: &AlignBackend,
    cancel: &CancelToken,
    status: &(dyn Fn(AssistantStatus) + Send + Sync),
) -> Result<AlignedDiff, ErrorTaskError> {
    let diff = &snapshot.diff;
    let segment = snapshot.segment;
    // Validate the native UTF-16 spans before any provider request.
    crop_error_diff_context(diff, 10).map_err(align_failure)?;
    let sample = 1.0 / f64::from(audio.sample_rate());
    let mut audio_range: Option<AudioInterval> = None;
    for window in 1..=3 {
        let range = match audio_range {
            Some(range) => {
                if range.start_seconds < segment.start_seconds - sample
                    || range.end_seconds > segment.end_seconds + sample
                {
                    return Err(ErrorTaskError::new(
                        "invalid-audio",
                        "The clip is outside this segment.",
                    ));
                }
                range
            }
            None => estimate_error_diff_clip(diff, segment, 10.0).map_err(align_failure)?,
        };
        let clip = capture(&audio, range)?;
        let label = format!("Finding the words {window}/3");
        status(AssistantStatus::new(
            AssistantPhase::Aligning,
            format!("{label}…"),
        ));
        let report = |progress| {
            status(AssistantStatus::new(
                AssistantPhase::Aligning,
                progress_message(&label, progress),
            ))
        };
        let result = broker::align_clip(
            backend,
            &snapshot.identity,
            &snapshot.language,
            &clip,
            cancel,
            &report,
        )
        .await?;
        let error = match align_error_diff_context(diff, &result.words, clip.interval) {
            Ok(alignment) => {
                let alignment = acknowledge(diff, alignment, &clip)?;
                status(AssistantStatus {
                    phase: AssistantPhase::Aligned,
                    message:
                        if alignment.boundary_note == Some(BoundaryNote::OverlappingAsrWords) {
                            "Words overlap here. Check the audio and choices before Submit."
                        } else {
                            "Audio found. Check it and the choices before Submit."
                        }
                        .to_string(),
                    receipt: Some(result.receipt.clone()),
                });
                return Ok(AlignedDiff {
                    alignment,
                    receipt: result.receipt,
                });
            }
            Err(error) => error,
        };
        if error.is_miss() && segment.duration() <= 10.0 {
            let whole_segment = (clip.interval.start_seconds - segment.start_seconds).abs()
                <= 0.000125
                && (clip.interval.end_seconds - segment.end_seconds).abs() <= 0.000125;
            let alignment;
            let (full_clip, full_result) = if whole_segment {
                alignment = segment_error_diff_context(diff, &result.words, clip.interval, segment)
                    .map_err(align_failure)?;
                (clip, result)
            } else {
                let full_clip = capture(&audio, segment)?;
                let report = |progress| {
                    status(AssistantStatus::new(
                        AssistantPhase::Aligning,
                        progress_message("Checking the whole segment", progress),
                    ))
                };
                let full_result = broker::align_clip(
                    backend,
                    &snapshot.identity,
                    &snapshot.language,
                    &full_clip,
                    cancel,
                    &report,
                )
                .await?;
                alignment =
                    match align_error_diff_context(diff, &full_result.words, full_clip.interval) {
                        Ok(alignment) => alignment,
                        Err(error) if error.is_miss() => segment_error_diff_context(
                            diff,
                            &full_result.words,
                            full_clip.interval,
                            segment,
                        )
                        .map_err(align_failure)?,
                        Err(error) => return Err(align_failure(error)),
                    };
                (full_clip, full_result)
            };
            let alignment = acknowledge(diff, alignment, &full_clip)?;
            status(AssistantStatus {
                phase: AssistantPhase::Aligned,
                message: if alignment.boundary_note == Some(BoundaryNote::SegmentOnly) {
                    "Using the whole segment. Check the audio and choices before Submit."
                } else {
                    "Audio found. Check it and the choices before Submit."
                }
                .to_string(),
                receipt: Some(full_result.receipt.clone()),
            });
            return Ok(AlignedDiff {
                alignment,
                receipt: full_result.receipt,
            });
        }
        let AlignError::ClipMiss { direction, .. } = error else {
            return Err(align_failure(error));
        };
        if window == 3 {
            break;
        }
        audio_range =
            Some(next_error_diff_clip(&error, clip.interval, segment).map_err(align_failure)?);
        status(AssistantStatus {
            phase: AssistantPhase::Aligning,
            message: match direction {
                ClipDirection::After => "Looking later in the audio…",
                ClipDirection::Before => "Looking earlier in the audio…",
            }
            .to_string(),
            receipt: Some(result.receipt),
        });
    }
    Err(ErrorTaskError::new(
        "alignment-failed",
        "Couldn’t find these words in the audio.",
    ))
}

/// Inputs of one Initial fill decision.
#[derive(Debug, Clone)]
pub struct InitialFillRequest {
    pub snapshot: ErrorTaskSnapshot,
    pub alignment: DiffAlignment,
    pub style_guide: StyleGuide,
    /// Gold OpenRouter key (Settings → AI).
    pub api_key: String,
    pub store: DecisionStore,
    /// `https://openrouter.ai/api/alpha/decisions` (tests point it at a local server).
    pub endpoint: String,
}

/// The paid decision for an aligned diff: the aligned context clip, the aligned comparison
/// crop and its ASR, and the full style guide. Returns the proposed choices; applying them
/// (unanswered choices only) is the caller's job.
pub async fn initial_fill_decision(
    request: InitialFillRequest,
    audio: ClipSource,
    cancel: &CancelToken,
    status: &(dyn Fn(AssistantStatus) + Send + Sync),
) -> Result<(ErrorLabelDraft, Receipt), ErrorTaskError> {
    let alignment = &request.alignment;
    let clip = capture(
        &audio,
        AudioInterval::new(
            alignment.context_start_seconds,
            alignment.context_end_seconds,
        ),
    )?;
    if (clip.interval.start_seconds - alignment.context_start_seconds).abs() > 0.001
        || (clip.interval.end_seconds - alignment.context_end_seconds).abs() > 0.001
    {
        return Err(ErrorTaskError::new(
            "stale-source",
            "The audio changed. Try again.",
        ));
    }
    let cropped = crop_aligned_error_diff_context(&request.snapshot.diff, alignment)
        .map_err(align_failure)?;
    let comparison = DecisionComparison {
        language: request.snapshot.language.clone(),
        human: cropped.human,
        machine: cropped.machine,
        asr_text: alignment.asr_text.clone(),
        style_guide: request.style_guide,
    };
    let message = |text: String| {
        status(AssistantStatus::new(
            AssistantPhase::Drafting,
            status_with_boundary_note(AssistantPhase::Drafting, &text, Some(alignment)),
        ))
    };
    message("Filling choices…".to_string());
    let report = |progress| message(progress_message("Filling choices", progress));
    let decision = broker::decide(
        DecisionInput {
            identity: request.snapshot.identity.clone(),
            comparison,
            clip,
            api_key: request.api_key,
            store: request.store,
            endpoint: request.endpoint,
        },
        cancel,
        &report,
    )
    .await?;
    Ok((decision.draft, decision.receipt))
}
