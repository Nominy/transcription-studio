//! Studio's line waveform against Babel's own code: the fixtures hold short WAVs and what
//! Babel's editor chunk computes for them in Chromium — the 8 kHz display signal (f32 bits)
//! and the outline wavesurfer strokes on each canvas for a few views. Regenerate with
//! `tools/waveform-parity/run.ps1 -Fixtures`.

use std::{fs, path::Path};

use serde_json::Value;
use studio_waveform::{DisplaySignal, LineView, LineWaveform, decode_pcm16_wav};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/babel_line");

fn fixtures() -> Vec<(String, DisplaySignal, Value)> {
    let mut names: Vec<_> = fs::read_dir(FIXTURES)
        .expect("fixture directory")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".wav"))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no fixtures in {FIXTURES}");
    names
        .into_iter()
        .map(|name| {
            let wav = fs::read(Path::new(FIXTURES).join(&name)).expect("fixture wav");
            let audio = decode_pcm16_wav(&wav).expect("PCM16 fixture");
            let expected: Value = serde_json::from_str(
                &fs::read_to_string(Path::new(FIXTURES).join(name.replace(".wav", ".json")))
                    .expect("fixture json"),
            )
            .expect("valid fixture json");
            (name, DisplaySignal::from_decoded(&audio), expected)
        })
        .collect()
}

#[test]
fn display_signal_matches_babels_streaming_downsampler_bit_for_bit() {
    for (name, signal, expected) in fixtures() {
        let expected: Vec<u32> = expected["signal_bits"]
            .as_array()
            .expect("signal bits")
            .iter()
            .map(|bits| bits.as_u64().expect("u32 bits") as u32)
            .collect();
        let actual: Vec<u32> = signal.samples().iter().map(|s| s.to_bits()).collect();
        assert_eq!(actual.len(), expected.len(), "{name}: length");
        let differing = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
        assert_eq!(differing, 0, "{name}: {differing} samples differ");
    }
}

#[test]
fn canvas_outlines_match_wavesurfers_line_renderer_vertex_for_vertex() {
    for (name, signal, expected) in fixtures() {
        let waveform = LineWaveform::new(signal);
        for case in expected["views"].as_array().expect("views") {
            let number = |key: &str| case["view"][key].as_f64().expect("view number");
            let view = LineView {
                min_px_per_sec: number("min_px_per_sec"),
                client_width: number("client_width"),
                height: number("height"),
                pixel_ratio: number("pixel_ratio"),
                scroll_left: number("scroll_left"),
            };
            let bar_height = number("bar_height");
            let frame = waveform.frame(&view).expect("frame");
            assert!(!frame.canvases.is_empty(), "{name}: no canvases");
            for canvas in &frame.canvases {
                let reference = case["canvases"]
                    .as_array()
                    .expect("canvases")
                    .iter()
                    .find(|c| c["left"].as_f64() == Some(canvas.left))
                    .unwrap_or_else(|| panic!("{name}: Babel drew no canvas at {}", canvas.left));
                assert_eq!(
                    reference["device_width"].as_u64(),
                    Some(u64::from(canvas.device_width)),
                    "{name}: canvas width"
                );
                assert_eq!(
                    reference["device_height"].as_u64(),
                    Some(u64::from(canvas.device_height)),
                    "{name}: canvas height"
                );
                let all = 0..canvas.columns.columns.len();
                for (key, direction) in [("top", -1.0), ("bottom", 1.0)] {
                    let actual: Vec<(f64, f64)> =
                        canvas.outline(all.clone(), bar_height, direction).collect();
                    let expected: Vec<(f64, f64)> = reference[key]
                        .as_array()
                        .expect("outline")
                        .iter()
                        .map(|point| (point[0].as_f64().expect("x"), point[1].as_f64().expect("y")))
                        .collect();
                    assert_eq!(actual, expected, "{name} {key} outline of {view:?}");
                }
            }
        }
    }
}
