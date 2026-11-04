//! `EditorModel`: the gpui entity that owns one open task's [`EditorDoc`] and the audio engine,
//! applies [`Effect`]s, drives the playback clock, and loads lane audio in the background.
//!
//! Views hold an `Entity<EditorModel>`, observe it for re-render, and subscribe to
//! [`EditorEvent`] for effects that target a specific view (scrolling, focus, draft reloads).
//! Playback frames that only advance the clock do not notify observers; they emit
//! [`EditorEvent::Frame`], which only views showing the playhead or viewport listen to.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::*;
use studio_audio::{
    AudioEngine, AudioError, DecodedAudio, LaneAudioFile, PlaybackState, load_lane_audio,
};
use studio_core::AudioLane;
use studio_editor::edit::{EditResult, EditorAction, EditorDoc, Effect, Outcome};
use studio_editor::timeline_model::LaneWaveform;
use studio_editor::workflow::StudioWorkflow;

use crate::editor::blended_waveforms::{BlendKey, BlendedWaveforms, Selection, build_blend};
use crate::editor::inference_audio::LaneSource;

/// View-directed effects re-emitted from [`Effect`].
#[derive(Debug, Clone, PartialEq)]
pub enum EditorEvent {
    RevealSegment {
        segment_id: String,
        center: bool,
    },
    FocusText {
        segment_id: String,
        caret: usize,
    },
    BlurText,
    DraftChanged {
        segment_id: String,
    },
    /// The playback clock advanced: only the playhead and the (follow/autoscroll) viewport
    /// changed. Emitted instead of a notification on such frames.
    Frame,
}

/// Lane audio loading progress for the status bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AudioLoadState {
    pub pending: usize,
    pub loaded: usize,
    pub failed: usize,
}

pub struct EditorModel {
    doc: EditorDoc,
    audio: AudioEngine,
    audio_base_dir: Option<PathBuf>,
    audio_load: AudioLoadState,
    /// Original file each loaded lane was decoded from, by lane id.
    lane_files: HashMap<String, PathBuf>,
    status: Option<SharedString>,
    last_frame: Option<Instant>,
    load_tasks: Vec<Task<()>>,
    /// What each lane's waveform paints while enhanced audio is installed.
    blends: BlendedWaveforms,
    /// Blend being built per lane.
    blend_tasks: HashMap<String, Task<()>>,
}

impl EventEmitter<EditorEvent> for EditorModel {}

impl EditorModel {
    /// Opens a task. `audio_base_dir` resolves lane audio paths (Babel staging dir, or the
    /// directory of an imported file); `None` means the task has no local audio.
    pub fn new(
        workflow: StudioWorkflow,
        audio_base_dir: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Self {
        let doc = EditorDoc::new(workflow);
        let mut audio = AudioEngine::new();
        audio.set_speed(doc.session().speed);
        let mut this = Self {
            doc,
            audio,
            audio_base_dir,
            audio_load: AudioLoadState::default(),
            lane_files: HashMap::new(),
            status: None,
            last_frame: None,
            load_tasks: Vec::new(),
            blends: BlendedWaveforms::default(),
            blend_tasks: HashMap::new(),
        };
        this.sync_timeline_end();
        this.load_lane_audio(cx);
        this
    }

    pub fn doc(&self) -> &EditorDoc {
        &self.doc
    }

    pub fn audio_load(&self) -> AudioLoadState {
        self.audio_load
    }

    pub fn audio_base_dir(&self) -> Option<&PathBuf> {
        self.audio_base_dir.as_ref()
    }

    pub fn status(&self) -> Option<&SharedString> {
        self.status.as_ref()
    }

    /// Project lanes whose original audio is loaded, in project order, with the files they
    /// were decoded from (the bytes inference backends receive).
    pub fn inference_lane_sources(&self) -> Vec<LaneSource> {
        self.doc
            .project()
            .lanes
            .iter()
            .filter_map(|lane| {
                self.lane_files.get(&lane.id).map(|path| LaneSource {
                    lane: lane.clone(),
                    path: path.clone(),
                })
            })
            .collect()
    }

    /// Installs enhanced PCM beside the loaded originals (all or nothing). Playback keeps the
    /// original clock; lane waveforms switch to what the lanes now play.
    pub fn set_enhanced_audio(
        &mut self,
        lanes: Vec<(String, Arc<DecodedAudio>)>,
        cx: &mut Context<Self>,
    ) -> Result<(), AudioError> {
        let result = self.audio.set_enhanced_audio(lanes);
        if result.is_ok() {
            self.blends.sources_changed();
            self.sync_display_waveforms(cx);
        }
        cx.notify();
        result
    }

    /// Share of enhanced audio in every lane (`0` original … `1` enhanced). Applies to live
    /// playback without restarting it; lane waveforms follow once their blend is built.
    pub fn set_enhanced_ratio(&mut self, ratio: f32, cx: &mut Context<Self>) {
        for lane in &self.doc.project().lanes {
            self.audio.set_lane_enhanced_ratio(&lane.id, ratio);
        }
        if self.sync_display_waveforms(cx) {
            cx.notify();
        }
    }

    /// Paints each loaded lane's waveform of what it plays: its original, or its blend with
    /// its enhanced audio at its enhanced ratio. A blend not built yet is built in the
    /// background while the current painting stays. Returns whether any painting changed.
    fn sync_display_waveforms(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        let lane_ids: Vec<String> = self.audio.lane_ids().map(str::to_owned).collect();
        for lane_id in lane_ids {
            let wanted = self.blends.wanted(
                &lane_id,
                self.audio.lane_enhanced_audio(&lane_id).is_some(),
                self.audio.lane_mix(&lane_id).enhanced_ratio,
            );
            match self.blends.select(&lane_id, wanted.clone()) {
                Selection::Unchanged => {}
                Selection::Paint { waveform, replaced } => {
                    let previous = self
                        .doc
                        .set_lane_display_waveform(&lane_id, waveform.map(|waveform| *waveform));
                    if let (Some(key), Some(previous)) = (replaced, previous) {
                        self.blends.store(key, previous);
                    }
                    changed = true;
                }
                Selection::Missing => {
                    if let Some(key) = wanted {
                        self.start_blend(key, cx);
                    }
                }
            }
        }
        changed
    }

    /// Builds `key`'s blend in the background unless its lane is already building one; when
    /// that finishes, the lane's then wanted blend is painted or built next.
    fn start_blend(&mut self, key: BlendKey, cx: &mut Context<Self>) {
        if self.blend_tasks.contains_key(&key.lane_id) {
            return;
        }
        let (Some(original), Some(enhanced)) = (
            self.audio.lane_audio(&key.lane_id).cloned(),
            self.audio.lane_enhanced_audio(&key.lane_id).cloned(),
        ) else {
            return;
        };
        let lane_id = key.lane_id.clone();
        let build = cx.background_spawn(async move {
            let waveform = build_blend(&key, &original, &enhanced);
            (key, waveform)
        });
        let task = cx.spawn(async move |this, cx| {
            let (key, waveform) = build.await;
            this.update(cx, |this, cx| this.finish_blend(key, waveform, cx))
                .ok();
        });
        self.blend_tasks.insert(lane_id, task);
    }

    fn finish_blend(
        &mut self,
        key: BlendKey,
        waveform: Option<LaneWaveform>,
        cx: &mut Context<Self>,
    ) {
        self.blend_tasks.remove(&key.lane_id);
        // Audio the engine accepted always blends; without a waveform there is nothing to
        // paint, and rebuilding would fail again.
        let Some(waveform) = waveform else {
            return;
        };
        self.blends.store(key, waveform);
        if self.sync_display_waveforms(cx) {
            cx.notify();
        }
    }

    pub fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = Some(status.into());
        cx.notify();
    }

    /// Mutates the doc directly (no effects). Notifies observers.
    pub fn update_doc<R>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut EditorDoc) -> R,
    ) -> R {
        let result = f(&mut self.doc);
        self.sync_lane_mix();
        cx.notify();
        result
    }

    /// Runs an editing operation and applies its outcome (status + effects).
    pub fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut EditorDoc) -> EditResult) {
        let result = f(&mut self.doc);
        self.apply_result(result, cx);
    }

    /// Runs an infallible doc operation that returns an [`Outcome`].
    pub fn run(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut EditorDoc) -> Outcome) {
        let outcome = f(&mut self.doc);
        self.apply_outcome(outcome, cx);
    }

    /// Runs a keymap action through the doc. Returns `false` for app-level actions the caller
    /// must handle (`BackToProjects`, `DraftTranscribeSelectedSegment`, `SwapSplitPanes`,
    /// dialog defaults).
    pub fn perform(
        &mut self,
        action: EditorAction,
        focused_segment: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.doc.perform(action, focused_segment) {
            Some(result) => {
                self.apply_result(result, cx);
                true
            }
            None => false,
        }
    }

    pub fn apply_result(&mut self, result: EditResult, cx: &mut Context<Self>) {
        match result {
            Ok(outcome) => self.apply_outcome(outcome, cx),
            Err(error) => {
                self.status = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    pub fn apply_outcome(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        if let Some(status) = outcome.status {
            self.status = Some(status.into());
        }
        let mut restart = false;
        for effect in outcome.effects {
            match effect {
                Effect::StartPlayback => {
                    self.last_frame = None;
                    let start = self.doc.session().playhead_seconds;
                    let result = self.audio.play_from(start);
                    self.report_audio(result);
                }
                Effect::StopPlayback => {
                    self.audio.pause();
                    self.audio.seek(self.doc.session().playhead_seconds);
                    // Stop wins over any earlier jump in the same outcome.
                    restart = false;
                }
                Effect::RestartPlayback => restart = true,
                Effect::PlayRange {
                    start_seconds,
                    end_seconds,
                } => {
                    let result = self.audio.audition(start_seconds, end_seconds);
                    self.report_audio(result);
                }
                Effect::SpeedChanged(speed) => self.audio.set_speed(speed),
                Effect::RevealSegment { segment_id, center } => {
                    cx.emit(EditorEvent::RevealSegment { segment_id, center });
                }
                Effect::FocusText { segment_id, caret } => {
                    cx.emit(EditorEvent::FocusText { segment_id, caret });
                }
                Effect::BlurText => cx.emit(EditorEvent::BlurText),
                Effect::DraftChanged { segment_id } => {
                    cx.emit(EditorEvent::DraftChanged { segment_id });
                }
            }
        }
        // Coalesce playhead jumps: one seek per outcome. The engine seeks live sources in place.
        if restart {
            let playhead = self.doc.session().playhead_seconds;
            if self.doc.session().is_playing && self.audio.state() == PlaybackState::Stopped {
                let result = self.audio.play_from(playhead);
                self.report_audio(result);
            } else {
                self.audio.seek(playhead);
            }
        }
        self.sync_lane_mix();
        cx.notify();
    }

    /// Whether the view must keep requesting animation frames.
    pub fn needs_frames(&self) -> bool {
        self.doc.needs_ticks() || self.audio.state() != PlaybackState::Stopped
    }

    /// Advances the playback clock by the wall time since the previous frame. Call once per
    /// animation frame while [`Self::needs_frames`] is true.
    pub fn frame(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let elapsed = self
            .last_frame
            .replace(now)
            .map_or(0.0, |previous| now.duration_since(previous).as_secs_f64());

        // The doc owns the playhead (end/loop handling); polling applies the engine's own
        // transitions so finished players are released.
        let audio_event = self.audio.poll();
        if self.doc.needs_ticks() {
            let generation = self.doc.generation();
            let outcome = self.doc.tick(elapsed);
            if outcome.status.is_none()
                && outcome.effects.is_empty()
                && self.doc.generation() == generation
            {
                cx.emit(EditorEvent::Frame);
            } else {
                self.apply_outcome(outcome, cx);
            }
        } else if audio_event.is_some() {
            cx.notify();
        }
        if !self.needs_frames() {
            self.last_frame = None;
        }
    }

    /// Stops playback and releases decoded audio and the output device. Call before dropping
    /// the editor (back to projects, task switch, hide to tray).
    pub fn unload(&mut self) {
        self.load_tasks.clear();
        self.blend_tasks.clear();
        self.blends = BlendedWaveforms::default();
        self.lane_files.clear();
        self.audio.unload();
    }

    /// Mirrors the session's muted lanes into the audio engine.
    fn sync_lane_mix(&mut self) {
        let muted = &self.doc.session().muted_lane_ids;
        let lane_ids: Vec<String> = self.audio.lane_ids().map(str::to_owned).collect();
        for id in lane_ids {
            self.audio.set_lane_muted(&id, muted.contains(&id));
        }
    }

    fn report_audio(&mut self, result: Result<usize, AudioError>) {
        if let Err(error) = result {
            self.status = Some(format!("Audio playback unavailable: {error}").into());
        }
    }

    fn sync_timeline_end(&mut self) {
        let end = self
            .doc
            .session()
            .timeline_bounds(self.doc.project())
            .end_seconds;
        self.audio.set_timeline_end(Some(end));
    }

    fn load_lane_audio(&mut self, cx: &mut Context<Self>) {
        let Some(base) = self.audio_base_dir.clone() else {
            return;
        };
        let lanes: Vec<AudioLane> = self.doc.project().lanes.clone();
        self.audio_load.pending = lanes.len();
        for lane in lanes {
            let base = base.clone();
            // Decoding and the waveform's display signal both run off the UI thread.
            let decode = cx.background_spawn(async move {
                let result = load_lane_audio(&base, &lane).map(|LaneAudioFile { path, audio }| {
                    let audio = Arc::new(audio);
                    // The clone shares the sample buffers with the engine; PCM is held once.
                    let waveform = LaneWaveform::from_decoded((*audio).clone());
                    LoadedLane {
                        path,
                        audio,
                        waveform,
                    }
                });
                (lane.id, result)
            });
            let task = cx.spawn(async move |this, cx| {
                let (lane_id, result) = decode.await;
                this.update(cx, |this, cx| this.finish_lane_load(lane_id, result, cx))
                    .ok();
            });
            self.load_tasks.push(task);
        }
    }

    fn finish_lane_load(
        &mut self,
        lane_id: String,
        result: Result<LoadedLane, AudioError>,
        cx: &mut Context<Self>,
    ) {
        self.audio_load.pending = self.audio_load.pending.saturating_sub(1);
        match result.and_then(|loaded| {
            self.audio
                .insert_lane(lane_id.clone(), Arc::clone(&loaded.audio))
                .map(|()| loaded)
        }) {
            Ok(LoadedLane { path, waveform, .. }) => {
                self.audio_load.loaded += 1;
                self.lane_files.insert(lane_id.clone(), path);
                self.doc.set_lane_waveform(lane_id, waveform);
                self.sync_lane_mix();
                self.sync_timeline_end();
                // New originals drop the lane's enhanced audio and outdate its blends.
                self.blends.sources_changed();
                self.sync_display_waveforms(cx);
            }
            Err(error) => {
                self.audio_load.failed += 1;
                let lane = self.doc.lane_label(&lane_id);
                self.status = Some(format!("{lane} audio: {error}").into());
            }
        }
        cx.notify();
    }
}

/// A lane's decoded original with its waveform, built in the background.
struct LoadedLane {
    path: PathBuf,
    audio: Arc<DecodedAudio>,
    waveform: LaneWaveform,
}
