//! Studio side of `tools/waveform-parity`: runs each resolved case through the timeline's
//! waveform pipeline (PCM16 decode → display signal → canvas columns → outline) and writes
//! the canvases the timeline would fill, in wavesurfer's canvas coordinates.
//!
//! usage: cargo run --release -p studio-waveform --example line_parity -- <outDir>
//! reads  <outDir>/resolved-cases.json, <outDir>/audio/*.wav
//! writes <outDir>/studio/<case>.json, <outDir>/studio/signal-<audio>.f32, timings.json

use std::{collections::HashMap, fs, path::Path, sync::Arc, time::Instant};

use serde_json::{Value, json};
use studio_waveform::{DisplaySignal, LineView, LineWaveform, decode_pcm16_wav};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .ok_or("usage: line_parity <outDir>")?;
    let out = Path::new(&out);
    let cases: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(out.join("resolved-cases.json"))?)?;
    fs::create_dir_all(out.join("studio"))?;

    let mut waveforms: HashMap<String, Arc<LineWaveform>> = HashMap::new();
    let mut timings = Vec::new();
    for case in &cases {
        let id = case["id"].as_str().ok_or("case id")?;
        let audio = case["audio"].as_str().ok_or("case audio")?;
        if !waveforms.contains_key(audio) {
            let bytes = fs::read(out.join("audio").join(audio))?;
            let decoded =
                decode_pcm16_wav(&bytes).map_err(|error| format!("{audio}: {error:?}"))?;
            let started = Instant::now();
            let signal = DisplaySignal::from_decoded(&decoded);
            let elapsed = started.elapsed();
            timings.push(json!({
                "audio": audio,
                "seconds": decoded.duration(),
                "sample_rate": decoded.sample_rate(),
                "channels": decoded.number_of_channels(),
                "display_signal_ms": elapsed.as_secs_f64() * 1e3,
            }));
            let bytes: Vec<u8> = signal
                .samples()
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect();
            fs::write(
                out.join("studio").join(format!("signal-{audio}.f32")),
                bytes,
            )?;
            waveforms.insert(audio.to_string(), Arc::new(LineWaveform::new(signal)));
        }
        let waveform = &waveforms[audio];
        let number = |key: &str| case[key].as_f64().ok_or(format!("{id}: {key}"));
        let view = LineView {
            min_px_per_sec: number("pxPerSec")?,
            client_width: number("clientWidth")?,
            height: number("height")?,
            pixel_ratio: number("dpr")?,
            scroll_left: number("scroll")?,
        };
        let bar_height = number("barHeight")?;
        let started = Instant::now();
        let frame = waveform.frame(&view).ok_or(format!("{id}: empty frame"))?;
        let cold = started.elapsed();
        let started = Instant::now();
        let _ = waveform.frame(&view);
        let warm = started.elapsed();
        let canvases: Vec<Value> = frame
            .canvases
            .iter()
            .map(|canvas| {
                let all = 0..canvas.columns.columns.len();
                let outline = |direction| {
                    canvas
                        .outline(all.clone(), bar_height, direction)
                        .map(|(x, y)| json!([x, y]))
                        .collect::<Vec<_>>()
                };
                json!({
                    "index": canvas.index,
                    "left": canvas.left,
                    "width": canvas.width,
                    "deviceWidth": canvas.device_width,
                    "deviceHeight": canvas.device_height,
                    "viewportX": canvas.viewport_x,
                    "top": outline(-1.0),
                    "bottom": outline(1.0),
                })
            })
            .collect();
        timings.push(json!({
            "case": id,
            "frame_cold_ms": cold.as_secs_f64() * 1e3,
            "frame_warm_ms": warm.as_secs_f64() * 1e3,
        }));
        fs::write(
            out.join("studio").join(format!("{id}.json")),
            serde_json::to_string(&json!({
                "scrollWidth": frame.layout.scroll_width,
                "canvasWidth": frame.layout.canvas_width,
                "canvases": canvases,
            }))?,
        )?;
        println!("studio {id}: {} canvases", frame.canvases.len());
    }
    fs::write(
        out.join("studio").join("timings.json"),
        serde_json::to_string_pretty(&timings)?,
    )?;
    Ok(())
}
