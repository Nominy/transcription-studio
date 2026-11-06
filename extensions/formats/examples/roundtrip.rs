//! Writes known synthetic subtitle exports and reads them through the same public API.
use exaudio_extension::format::{ExportOptions, FormatProvider, ImportContent};
use exaudio_formats::SubtitleFormat;
use studio_core::{Project, Segment, Speaker};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project = Project {
        id: "synthetic".into(),
        name: "Synthetic roundtrip".into(),
        speakers: vec![
            Speaker {
                id: "one".into(),
                label: "Alice".into(),
            },
            Speaker {
                id: "two".into(),
                label: "Боб".into(),
            },
        ],
        lanes: vec![],
        metadata: Default::default(),
        segments: vec![
            Segment {
                id: "one".into(),
                source_annotation_id: None,
                speaker_id: "one".into(),
                lane_id: "selected-audio".into(),
                start_seconds: 0.0,
                end_seconds: 2.5,
                text: "Hello & 世界\nSecond line".into(),
                metadata: Default::default(),
            },
            Segment {
                id: "two".into(),
                source_annotation_id: None,
                speaker_id: "two".into(),
                lane_id: "selected-audio".into(),
                start_seconds: 1.25,
                end_seconds: 3.0,
                text: "Перекрытие".into(),
                metadata: Default::default(),
            },
        ],
    };
    for codec in [
        SubtitleFormat::Srt,
        SubtitleFormat::WebVtt,
        SubtitleFormat::Text,
    ] {
        let encoded = codec.export(&project, &ExportOptions::default())?;
        println!(
            "--- {} ---\n{}",
            codec.descriptor().name,
            std::str::from_utf8(&encoded)?
        );
        match codec.import(&encoded)? {
            ImportContent::Timed(cues) => {
                assert_eq!(cues.len(), 2);
                assert_eq!(cues[0].text, project.segments[0].text);
                assert_eq!(cues[1].start_seconds, 1.25);
            }
            ImportContent::Untimed(text) => {
                assert_eq!(text, "Hello & 世界\nSecond line\nПерекрытие")
            }
        }
    }
    Ok(())
}
