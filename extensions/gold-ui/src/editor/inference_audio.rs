//! Original lane audio for inference backends (Gold timing, draft and enhancement): the exact
//! WAV bytes each loaded lane was decoded from, never playback or enhanced PCM.

use std::pin::pin;
use std::task::Poll;

use anyhow::Context as _;
use bytes::Bytes;
use exaudio_shell::editor::model::LaneSource;
use studio_inference::LaneAudio;

/// The headline of a failed read: the track's name only; the path and OS error follow in
/// the chain.
#[derive(Debug)]
pub struct AudioReadError {
    pub label: String,
}

impl std::fmt::Display for AudioReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Couldn't read the audio for {}.", self.label)
    }
}

/// Reads the original WAV bytes of `sources`, in order. Blocking file IO: call it from a
/// background executor. A failure carries [`AudioReadError`] as its outermost context.
pub fn read_lane_audio(sources: &[LaneSource]) -> anyhow::Result<Vec<LaneAudio>> {
    sources
        .iter()
        .map(|source| {
            let wav = std::fs::read(&source.path)
                .with_context(|| source.path.display().to_string())
                .with_context(|| AudioReadError {
                    label: source.lane.label.clone(),
                })?;
            Ok(LaneAudio {
                lane_id: source.lane.id.clone(),
                speaker_key: source.lane.speaker_id.clone(),
                label: source.lane.label.clone(),
                wav: Bytes::from(wav),
            })
        })
        .collect()
}

/// Runs `work` until it finishes (`Some`) or every sender of `cancelled` is dropped (`None`,
/// and `work` is dropped).
pub async fn until_cancelled<F: Future>(
    work: F,
    cancelled: async_channel::Receiver<()>,
) -> Option<F::Output> {
    let mut work = pin!(work);
    let mut cancel = pin!(cancelled.recv());
    std::future::poll_fn(|cx| {
        if let Poll::Ready(output) = work.as_mut().poll(cx) {
            return Poll::Ready(Some(output));
        }
        if cancel.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}
