use std::collections::HashMap;

use exaudio_gold::polish::{
    DraftAudioMode, broker_audio_tracks, draft_audio_cue_status, draft_audio_guard,
    encode_pcm16_wav, resolve_draft_audio,
};
use studio_waveform::DecodedAudio;

use super::sample_project;

fn audio() -> DecodedAudio {
    DecodedAudio::new(4, vec![vec![0.0, 0.25, -0.25, 0.5]]).unwrap()
}

#[test]
fn audio_modes_choose_tracks_or_the_guard() {
    let project = sample_project();
    let none = HashMap::<String, DecodedAudio>::new();
    let partial = HashMap::from([("lane-spk-1".to_string(), audio())]);
    let status_none = draft_audio_cue_status(&project, |lane| none.get(lane));
    let status_partial = draft_audio_cue_status(&project, |lane| partial.get(lane));
    let partial_tracks = || broker_audio_tracks(&project, |lane| partial.get(lane));

    let guard =
        resolve_draft_audio(DraftAudioMode::Guard, &status_partial, partial_tracks()).unwrap_err();
    assert!(guard.can_use_partial_audio);
    assert_eq!(
        resolve_draft_audio(
            DraftAudioMode::UseAvailableAudio,
            &status_partial,
            partial_tracks()
        )
        .unwrap()
        .len(),
        1
    );
    assert!(
        resolve_draft_audio(DraftAudioMode::UseAvailableAudio, &status_none, Vec::new()).is_err()
    );
    assert!(
        resolve_draft_audio(DraftAudioMode::TextOnly, &status_partial, partial_tracks())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cue_status_and_guard_track_loaded_lanes() {
    let project = sample_project();
    let mut loaded = HashMap::new();

    let status = draft_audio_cue_status(&project, |lane| loaded.get(lane));
    assert_eq!((status.loaded_tracks, status.total_lanes), (0, 2));
    assert_eq!(status.missing_lane_labels, vec!["Speaker 1", "Speaker 2"]);
    assert_eq!(
        status.missing_lane_sources,
        vec!["audio/spk-1.wav", "audio/spk-2.wav"]
    );
    let guard = draft_audio_guard(&status, 0).expect("no audio should open the guard");
    assert_eq!(guard.captured_speaker_lanes, 0);
    assert!(!guard.can_use_partial_audio);

    loaded.insert("lane-spk-1".to_string(), audio());
    let status = draft_audio_cue_status(&project, |lane| loaded.get(lane));
    assert_eq!(status.loaded_tracks, 1);
    assert_eq!(status.missing_lane_labels, vec!["Speaker 2"]);
    let guard = draft_audio_guard(&status, 1).unwrap();
    assert!(guard.can_use_partial_audio);
    assert_eq!(guard.expected_speaker_lanes, 2);
    assert_eq!(guard.captured_tracks, 1);

    loaded.insert("lane-spk-2".to_string(), audio());
    let status = draft_audio_cue_status(&project, |lane| loaded.get(lane));
    assert!(draft_audio_guard(&status, 2).is_none());
}

#[test]
fn broker_tracks_attach_loaded_lanes_as_pcm16_wav() {
    let project = sample_project();
    let loaded = HashMap::from([("lane-spk-2".to_string(), audio())]);

    let tracks = broker_audio_tracks(&project, |lane| loaded.get(lane));

    assert_eq!(tracks.len(), 1);
    let track = &tracks[0];
    assert_eq!(track.track_id, "lane-spk-2");
    assert_eq!(track.file_name, "lane_spk_2.wav");
    assert_eq!(track.mime_type, "audio/wav");
    assert_eq!(track.metadata["speakerKey"], "spk-2");
    assert_eq!(track.metadata["trackLabel"], "Speaker 2");
    assert_eq!(&track.bytes[0..4], b"RIFF");
    let decoded = studio_waveform::decode_pcm16_wav(&track.bytes).unwrap();
    assert_eq!(decoded.sample_rate(), 4);
    assert_eq!(decoded.frame_count(), 4);
}

#[test]
fn wav_encoding_interleaves_and_clamps_samples() {
    let stereo = DecodedAudio::new(8000, vec![vec![1.5, -1.0], vec![0.0, 0.5]]).unwrap();
    let bytes = encode_pcm16_wav(&stereo).unwrap();
    assert_eq!(bytes.len(), 44 + 2 * 2 * 2);
    let samples = bytes[44..]
        .chunks(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    assert_eq!(samples, vec![32767, 0, -32768, 16384]);
    let decoded = studio_waveform::decode_pcm16_wav(&bytes).unwrap();
    assert_eq!(decoded.number_of_channels(), 2);
}
