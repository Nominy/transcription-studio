//! Smoke run of [`LocalBackend`] on real audio: status → install models (if needed) → GPU sample
//! test (if needed) → timing → draft → align (8 s clip of the first lane) → enhance.
//!
//! Usage: `local_smoke <data-dir> <run-dir> <task-id> <lane-wav>...` — `data-dir` as in
//! [`LocalConfig`]; the pinned Gold runtime is downloaded into it on first use.
//!
//! Observability: one line per event on stdout, `<run-dir>/events.jsonl` (append-only) and
//! `<run-dir>/status.json` (atomically replaced). Graceful stop: create `<run-dir>/stop`; the
//! in-flight request is cancelled (model downloads keep their staged chunks) and the run exits.
//! Resume: rerun the same command; installed/tested models are detected from Gold's status and a
//! finished `timing.json` in the run dir is reused.

#[cfg(windows)]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    smoke::run().await
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod smoke {
    use std::{
        future::Future,
        io::Write,
        path::{Path, PathBuf},
        process::ExitCode,
        sync::{Arc, Mutex},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use bytes::Bytes;
    use exaudio_gold::{LocalBackend, LocalConfig, protocol::ModelsState};
    use serde_json::{Value, json};
    use studio_inference::{
        AlignRequest, InferenceBackend, InferenceProgress, LaneAudio, ProgressSink, TimingResponse,
    };

    struct Run {
        dir: PathBuf,
        started: Instant,
        state: Mutex<Value>,
        last_print: Mutex<Instant>,
    }

    impl Run {
        fn event(&self, kind: &str, data: Value) {
            let line = json!({
                "ts": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                "elapsedS": self.started.elapsed().as_secs_f64(),
                "kind": kind,
                "data": data,
            });
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join("events.jsonl"))
                .expect("events.jsonl");
            writeln!(file, "{line}").expect("write event");
        }

        fn status(&self, patch: Value) {
            let mut state = self.state.lock().unwrap();
            if let (Value::Object(state), Value::Object(patch)) = (&mut *state, patch) {
                state.extend(patch);
                state.insert(
                    "elapsedS".to_string(),
                    json!(self.started.elapsed().as_secs_f64()),
                );
                state.insert(
                    "heartbeatMs".to_string(),
                    json!(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64
                    ),
                );
            }
            let tmp = self.dir.join("status.json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&*state).unwrap()).expect("status");
            std::fs::rename(&tmp, self.dir.join("status.json")).expect("status rename");
        }

        fn say(&self, text: &str) {
            println!("[{:>8.1}s] {text}", self.started.elapsed().as_secs_f64());
        }

        fn progress_sink(self: &Arc<Self>, stage: &'static str) -> ProgressSink {
            let run = self.clone();
            let stage_started = Instant::now();
            ProgressSink::new(move |progress| {
                let (detail, data) = match &progress {
                    InferenceProgress::Download { bytes, total } => {
                        let rate = *bytes as f64 / stage_started.elapsed().as_secs_f64().max(0.001);
                        let eta = (total.saturating_sub(*bytes)) as f64 / rate.max(1.0);
                        (
                            format!(
                                "download {:.1}/{:.1} MB ({:.1} MB/s, eta {:.0}s)",
                                *bytes as f64 / 1e6,
                                *total as f64 / 1e6,
                                rate / 1e6,
                                eta
                            ),
                            json!({ "bytes": bytes, "total": total, "bytesPerSecond": rate, "etaS": eta }),
                        )
                    }
                    InferenceProgress::Stage {
                        stage: phase,
                        completed,
                        total,
                        detail,
                        ..
                    } => (
                        format!(
                            "{phase} {completed}/{} {}",
                            total.map_or("?".to_string(), |t| t.to_string()),
                            detail.as_deref().unwrap_or("")
                        ),
                        json!({ "phase": phase, "completed": completed, "total": total, "detail": detail }),
                    ),
                    InferenceProgress::Queued { position, count } => (
                        format!("queued {position}/{count}"),
                        json!({ "position": position, "count": count }),
                    ),
                };
                run.event("progress", json!({ "stage": stage, "progress": data }));
                let mut last = run.last_print.lock().unwrap();
                if last.elapsed() >= Duration::from_millis(500) {
                    *last = Instant::now();
                    drop(last);
                    run.say(&format!("{stage}: {detail}"));
                    run.status(json!({ "stage": stage, "progress": data }));
                }
            })
        }

        /// Runs `operation` unless `<run-dir>/stop` appears first (then the future is dropped,
        /// which cancels the page request).
        async fn stoppable<T>(&self, operation: impl Future<Output = T>) -> Option<T> {
            let stop = self.dir.join("stop");
            let watch = async {
                loop {
                    if stop.exists() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            };
            tokio::select! {
                result = operation => Some(result),
                () = watch => None,
            }
        }
    }

    pub async fn run() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.len() < 5 {
            eprintln!(
                "usage: local_smoke <extension-dir> <data-dir> <run-dir> <task-id> <lane-wav>..."
            );
            return ExitCode::from(2);
        }
        let run = Arc::new(Run {
            dir: PathBuf::from(&args[1]),
            started: Instant::now(),
            state: Mutex::new(json!({ "state": "initializing" })),
            last_print: Mutex::new(Instant::now() - Duration::from_secs(1)),
        });
        std::fs::create_dir_all(&run.dir).expect("run dir");
        let _ = std::fs::remove_file(run.dir.join("stop"));
        let task_id = args[2].clone();
        let backend = LocalBackend::new(LocalConfig::new(PathBuf::from(&args[0])));
        run.event("start", json!({ "taskId": task_id, "lanes": &args[3..] }));
        run.status(json!({ "state": "running", "stage": "status" }));
        let outcome = stages(&run, &backend, &task_id, &args[3..]).await;
        let (state, code) = match &outcome {
            Ok(true) => ("completed", ExitCode::SUCCESS),
            Ok(false) => ("stopped", ExitCode::from(3)),
            Err(error) => {
                run.say(&format!("FAILED: {error}"));
                ("failed", ExitCode::FAILURE)
            }
        };
        if !matches!(outcome, Ok(true)) {
            // Give the page a moment to observe the cancel before tearing it down.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let shutdown_started = Instant::now();
        tokio::task::spawn_blocking(move || backend.shutdown())
            .await
            .unwrap();
        run.event(
            state,
            json!({ "error": outcome.err(), "shutdownS": shutdown_started.elapsed().as_secs_f64() }),
        );
        run.status(json!({ "state": state }));
        run.say(state);
        code
    }

    async fn timed<T>(
        run: &Run,
        stage: &str,
        operation: impl Future<Output = T>,
    ) -> Option<(T, f64)> {
        let started = Instant::now();
        run.status(json!({ "stage": stage }));
        run.event("stage-start", json!({ "stage": stage }));
        let result = run.stoppable(operation).await?;
        let seconds = started.elapsed().as_secs_f64();
        run.event("stage-end", json!({ "stage": stage, "seconds": seconds }));
        Some((result, seconds))
    }

    async fn stages(
        run: &Arc<Run>,
        backend: &LocalBackend,
        task_id: &str,
        wavs: &[String],
    ) -> Result<bool, String> {
        let sink = run.progress_sink("runtime");
        let Some((runtime, seconds)) = timed(run, "runtime", backend.install_runtime(sink)).await
        else {
            return Ok(false);
        };
        let runtime = runtime.map_err(|e| format!("runtime: {e} ({:?})", e.kind))?;
        run.say(&format!(
            "Gold runtime ready in {seconds:.1}s: {}",
            runtime.display()
        ));
        let Some((status, seconds)) = timed(run, "status", backend.host_status()).await else {
            return Ok(false);
        };
        let status = status.map_err(|e| format!("status: {e} ({:?})", e.kind))?;
        run.say(&format!(
            "status in {seconds:.1}s: release {} models {:?} tested {} webgpu {:?}",
            status.release, status.models.state, status.models.tested, status.webgpu
        ));
        run.event("host-status", serde_json::to_value(&status).unwrap());

        if status.models.state != ModelsState::Ready {
            run.say(&format!(
                "installing models ({:?} bytes)",
                status.download_bytes
            ));
            let sink = run.progress_sink("install");
            let Some((models, seconds)) = timed(run, "install", backend.install_models(sink)).await
            else {
                return Ok(false);
            };
            let models = models.map_err(|e| format!("install: {e} ({:?})", e.kind))?;
            run.say(&format!("installed in {seconds:.1}s: {models:?}"));
        }
        if !backend
            .host_status()
            .await
            .map_err(|e| format!("status: {e}"))?
            .models
            .tested
        {
            let sink = run.progress_sink("test");
            let Some((sample, seconds)) = timed(run, "test", backend.test_sample(sink)).await
            else {
                return Ok(false);
            };
            let sample = sample.map_err(|e| format!("test: {e} ({:?})", e.kind))?;
            run.say(&format!(
                "sample test in {seconds:.1}s: {} tokens, {:.1}s audio: {}",
                sample.tokens.len(),
                sample.duration_seconds,
                sample.text
            ));
            run.event("sample", serde_json::to_value(&sample).unwrap());
        }

        let lanes: Vec<LaneAudio> = wavs
            .iter()
            .map(|path| {
                let path = Path::new(path);
                let key = path.file_stem().unwrap().to_string_lossy().to_string();
                LaneAudio {
                    lane_id: key.clone(),
                    speaker_key: key.clone(),
                    label: key,
                    wav: Bytes::from(std::fs::read(path).expect("lane wav")),
                }
            })
            .collect();

        let timing_path = run.dir.join("timing.json");
        let timing: TimingResponse = match std::fs::read(&timing_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<TimingResponse>(&bytes).ok())
            .filter(|timing| timing.task_id == task_id)
        {
            Some(timing) => {
                run.say("reusing timing.json from a previous run");
                timing
            }
            None => {
                let sink = run.progress_sink("timing");
                let Some((timing, seconds)) =
                    timed(run, "timing", backend.timing(task_id, &lanes, sink)).await
                else {
                    return Ok(false);
                };
                let timing = timing.map_err(|e| format!("timing: {e} ({:?})", e.kind))?;
                let tokens: Vec<usize> = timing.tracks.iter().map(|t| t.tokens.len()).collect();
                let segments: Vec<usize> = timing.tracks.iter().map(|t| t.segments.len()).collect();
                run.say(&format!(
                    "timing in {seconds:.1}s: tokens per lane {tokens:?}, segments per lane {segments:?}"
                ));
                run.event(
                    "timing",
                    json!({ "seconds": seconds, "tokens": tokens, "segments": segments, "summary": timing.summary }),
                );
                std::fs::write(&timing_path, serde_json::to_vec(&timing).unwrap())
                    .map_err(|e| format!("saving timing: {e}"))?;
                timing
            }
        };

        let sink = run.progress_sink("draft");
        let Some((draft, seconds)) = timed(
            run,
            "draft",
            backend.draft(task_id, &lanes, &timing, None, sink),
        )
        .await
        else {
            return Ok(false);
        };
        let draft = draft.map_err(|e| format!("draft: {e} ({:?})", e.kind))?;
        let words: usize = draft
            .rows
            .iter()
            .map(|row| row.text.split_whitespace().count())
            .sum();
        run.say(&format!(
            "draft in {seconds:.1}s: {} rows, {words} words",
            draft.rows.len()
        ));
        for row in draft.rows.iter().take(5) {
            run.say(&format!(
                "  {} {:.2}-{:.2}: {}",
                row.lane, row.start_seconds, row.end_seconds, row.text
            ));
        }
        run.event(
            "draft",
            json!({ "seconds": seconds, "rows": draft.rows.len(), "words": words, "summary": draft.summary }),
        );
        std::fs::write(
            run.dir.join("draft.json"),
            serde_json::to_vec(&draft).unwrap(),
        )
        .map_err(|e| format!("saving draft: {e}"))?;

        let clip = pcm16_clip(&lanes[0].wav, 100.0, 8.0)?;
        let request = AlignRequest {
            task_id: format!("error-task-clip:smoke:{task_id}"),
            request_id: "smoke".to_string(),
            lane: lanes[0].speaker_key.clone(),
            language: "ru".to_string(),
            wav: clip,
        };
        let sink = run.progress_sink("align");
        let Some((aligned, seconds)) = timed(run, "align", backend.align(request, sink)).await
        else {
            return Ok(false);
        };
        let aligned = aligned.map_err(|e| format!("align: {e} ({:?})", e.kind))?;
        let text: Vec<&str> = aligned.tokens.iter().map(|t| t.text.as_str()).collect();
        run.say(&format!(
            "align in {seconds:.1}s: {} words, duration {:?}, model {}: {}",
            aligned.tokens.len(),
            aligned.duration_seconds,
            aligned.model,
            text.join(" ")
        ));
        run.event(
            "align",
            json!({ "seconds": seconds, "words": aligned.tokens.len(), "durationSeconds": aligned.duration_seconds, "model": aligned.model }),
        );

        let sink = run.progress_sink("enhance");
        let Some((enhanced, seconds)) =
            timed(run, "enhance", backend.enhance(task_id, &lanes, sink)).await
        else {
            return Ok(false);
        };
        let enhanced = enhanced.map_err(|e| format!("enhance: {e} ({:?})", e.kind))?;
        for (lane, output) in lanes.iter().zip(&enhanced) {
            run.say(&format!(
                "enhanced {} with {}: {} bytes (original {} bytes)",
                output.lane_id,
                output.model_id,
                output.wav.len(),
                lane.wav.len()
            ));
            std::fs::write(
                run.dir.join(format!("enhanced-{}.wav", output.lane_id)),
                &output.wav,
            )
            .map_err(|e| format!("saving enhanced audio: {e}"))?;
        }
        run.event(
            "enhance",
            json!({ "seconds": seconds, "bytes": enhanced.iter().map(|lane| lane.wav.len()).collect::<Vec<_>>() }),
        );
        run.say(&format!("enhance in {seconds:.1}s"));
        Ok(true)
    }

    /// `seconds` of a canonical 44-byte-header PCM16 mono WAV starting at `start`.
    fn pcm16_clip(wav: &[u8], start: f64, seconds: f64) -> Result<Bytes, String> {
        if wav.len() < 44 || &wav[0..4] != b"RIFF" || &wav[36..40] != b"data" {
            return Err("align clip needs a canonical PCM16 WAV".to_string());
        }
        let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
        let from = 44 + (start * f64::from(rate)) as usize * 2;
        let len = (seconds * f64::from(rate)) as usize * 2;
        let data = wav
            .get(from..from + len)
            .ok_or("lane is shorter than the clip")?;
        let mut clip = Vec::with_capacity(44 + len);
        clip.extend_from_slice(&wav[..44]);
        clip[4..8].copy_from_slice(&(36 + len as u32).to_le_bytes());
        clip[40..44].copy_from_slice(&(len as u32).to_le_bytes());
        clip.extend_from_slice(data);
        Ok(Bytes::from(clip))
    }
}
