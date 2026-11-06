//! [`LocalBackend`]: the shared [`InferenceBackend`] over Gold's host page in WebView2.
//!
//! The page host starts lazily on first use, is restarted on the next request after the browser
//! or page process dies, and is shut down by [`LocalBackend::shutdown`] (app exit) or drop.
//! Lane audio reaches the page as files in a staging folder mapped to
//! `https://studio-audio.example/`; each request's files are deleted when it settles.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::remote::GOLD_ENHANCEMENT_MODEL;
use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;
use studio_inference::{
    AlignRequest, AlignResponse, BackendStatus, DraftResponse, EnhancedLane, InferenceBackend,
    InferenceError, InferenceResult, LaneAudio, PreservedRow, ProgressSink, TimingResponse,
};

use crate::AUDIO_HOST;
use crate::bundle::GoldBuildPin;
use crate::protocol::{
    self, AlignResult, AudioAssembler, AudioRef, EnhanceResult, HostEvent, HostStatus,
    ModelsStatus, SampleResult,
};

/// Where the local backend keeps its state and which Gold build it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalConfig {
    /// Root for the downloaded Gold builds (`gold-inference`), the WebView2 profile
    /// (`webview-profile`, which holds the downloaded models) and the audio staging folder
    /// (`audio`).
    pub data_dir: PathBuf,
    pub gold_build: GoldBuildPin,
}

impl LocalConfig {
    /// `data_dir` with the Gold build pinned into this Studio version.
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            gold_build: GoldBuildPin::bundled(),
        }
    }

    /// The app's layout: state under Studio's app-support folder.
    pub fn for_app() -> Self {
        Self::new(studio_services::paths::app_support_dir().join("inference"))
    }

    #[cfg(windows)]
    fn builds_dir(&self) -> PathBuf {
        self.data_dir.join("gold-inference")
    }

    #[cfg(windows)]
    fn profile_dir(&self) -> PathBuf {
        self.data_dir.join("webview-profile")
    }

    fn audio_dir(&self) -> PathBuf {
        self.data_dir.join("audio")
    }
}

/// Gold "This PC" inference.
pub struct LocalBackend {
    config: LocalConfig,
    next_request: AtomicU64,
    #[cfg(windows)]
    builds: crate::bundle::GoldBuilds,
    #[cfg(windows)]
    host: windows_host::HostSlot,
}

impl LocalBackend {
    pub fn new(config: LocalConfig) -> Self {
        Self {
            #[cfg(windows)]
            builds: crate::bundle::GoldBuilds::new(config.builds_dir(), config.gold_build.clone()),
            config,
            next_request: AtomicU64::new(0),
            #[cfg(windows)]
            host: windows_host::HostSlot::default(),
        }
    }

    pub fn config(&self) -> &LocalConfig {
        &self.config
    }

    /// Full host status for the settings screen (models, test, WebGPU adapter).
    pub async fn host_status(&self) -> InferenceResult<HostStatus> {
        let value = self.call("status", "status", Vec::new(), None).await?;
        protocol::decode("status", value)
    }

    /// Downloads (or resumes) the current Gold release's models. Fetches the pinned Gold runtime
    /// first when it is not installed yet (`Stage` progress [`DOWNLOAD_STAGE`] /
    /// [`EXTRACT_STAGE`]), then emits `Download` progress for the models.
    ///
    /// [`DOWNLOAD_STAGE`]: crate::bundle::DOWNLOAD_STAGE
    /// [`EXTRACT_STAGE`]: crate::bundle::EXTRACT_STAGE
    pub async fn install_models(&self, progress: ProgressSink) -> InferenceResult<ModelsStatus> {
        let request_id = self.request_id();
        let value = self
            .call(
                &request_id,
                "installModels",
                protocol::request_args(&request_id),
                Some(Events::new(progress, None)),
            )
            .await?;
        protocol::decode("installModels", value)
    }

    /// Installs the pinned Gold runtime if needed (any first use does this implicitly) and
    /// returns its folder. Emits `Stage` progress [`DOWNLOAD_STAGE`] / [`EXTRACT_STAGE`].
    ///
    /// [`DOWNLOAD_STAGE`]: crate::bundle::DOWNLOAD_STAGE
    /// [`EXTRACT_STAGE`]: crate::bundle::EXTRACT_STAGE
    #[cfg(windows)]
    pub async fn install_runtime(&self, progress: ProgressSink) -> InferenceResult<PathBuf> {
        self.builds.ensure(&progress).await
    }

    #[cfg(not(windows))]
    pub async fn install_runtime(&self, _progress: ProgressSink) -> InferenceResult<PathBuf> {
        Err(unsupported())
    }

    /// Removes the cached model bundles.
    pub async fn remove_models(&self) -> InferenceResult<ModelsStatus> {
        let request_id = self.request_id();
        let value = self
            .call(
                &request_id,
                "removeModels",
                protocol::request_args(&request_id),
                None,
            )
            .await?;
        protocol::decode("removeModels", value)
    }

    /// Runs Gold's GPU sample test (full pipeline on the release sample) and marks the models
    /// tested; timing/draft need a passed test.
    pub async fn test_sample(&self, progress: ProgressSink) -> InferenceResult<SampleResult> {
        let request_id = self.request_id();
        let value = self
            .call(
                &request_id,
                "testSample",
                protocol::request_args(&request_id),
                Some(Events::new(progress, None)),
            )
            .await?;
        protocol::decode("testSample", value)
    }

    /// Stops the page host (app exit). The next request starts it again.
    pub fn shutdown(&self) {
        #[cfg(windows)]
        self.host.shutdown();
    }

    fn request_id(&self) -> String {
        format!(
            "studio-{}-{}",
            std::process::id(),
            self.next_request.fetch_add(1, Ordering::Relaxed)
        )
    }

    #[cfg(windows)]
    async fn call(
        &self,
        request_id: &str,
        method: &str,
        args: Vec<Value>,
        events: Option<Events>,
    ) -> InferenceResult<Value> {
        // Any first use installs the pinned Gold runtime, reporting to the caller's progress.
        let progress = events
            .as_ref()
            .map_or_else(ProgressSink::none, |events| events.progress.clone());
        let extension_dir = self.builds.ensure(&progress).await?;
        let host = self
            .host
            .get_or_start(&self.config, extension_dir)
            .await
            .map_err(windows_host::host_error)?;
        let sink = events.map(Events::into_sink);
        let cancel = windows_host::CancelOnDrop::new(&host, request_id, method != "status");
        let result = host.call(request_id, method, &args, sink).await;
        cancel.disarm();
        result.map_err(windows_host::host_error)
    }

    #[cfg(not(windows))]
    async fn call(
        &self,
        _request_id: &str,
        _method: &str,
        _args: Vec<Value>,
        _events: Option<Events>,
    ) -> InferenceResult<Value> {
        Err(unsupported())
    }

    /// Writes `wavs` into the staging folder for one request.
    async fn stage(&self, request_id: &str, wavs: Vec<Bytes>) -> InferenceResult<StagedAudio> {
        let dir = self.config.audio_dir();
        let request_id = request_id.to_string();
        tokio::task::spawn_blocking(move || StagedAudio::write(&dir, &request_id, wavs))
            .await
            .map_err(|error| {
                InferenceError::other(format!("Couldn't prepare the audio. ({error})"))
            })?
    }

    async fn call_with_audio(
        &self,
        method: &str,
        wavs: Vec<Bytes>,
        args: impl FnOnce(&str, &[AudioRef]) -> Vec<Value>,
        events: Events,
    ) -> InferenceResult<Value> {
        let request_id = self.request_id();
        let staged = self.stage(&request_id, wavs).await?;
        let args = args(&request_id, &staged.refs);
        let result = self.call(&request_id, method, args, Some(events)).await;
        drop(staged);
        result
    }
}

#[async_trait]
impl InferenceBackend for LocalBackend {
    async fn status(&self) -> BackendStatus {
        match self.host_status().await {
            Ok(status) => protocol::backend_status(&status),
            Err(error) => BackendStatus::Unavailable {
                kind: error.kind,
                message: error.message,
            },
        }
    }

    async fn timing(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        progress: ProgressSink,
    ) -> InferenceResult<TimingResponse> {
        let wavs = lanes.iter().map(|lane| lane.wav.clone()).collect();
        let value = self
            .call_with_audio(
                "timing",
                wavs,
                |request_id, audio| protocol::timing_args(request_id, task_id, lanes, audio),
                Events::new(progress, None),
            )
            .await?;
        protocol::decode_timing(value)
    }

    async fn draft(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        timing: &TimingResponse,
        preserve_rows: Option<Vec<PreservedRow>>,
        progress: ProgressSink,
    ) -> InferenceResult<DraftResponse> {
        let wavs = lanes.iter().map(|lane| lane.wav.clone()).collect();
        let value = self
            .call_with_audio(
                "draft",
                wavs,
                |request_id, audio| {
                    protocol::draft_args(
                        request_id,
                        task_id,
                        timing,
                        preserve_rows.as_deref(),
                        lanes,
                        audio,
                    )
                },
                Events::new(progress, None),
            )
            .await?;
        protocol::decode_draft(value)
    }

    async fn enhance(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        progress: ProgressSink,
    ) -> InferenceResult<Vec<EnhancedLane>> {
        let wavs = lanes.iter().map(|lane| lane.wav.clone()).collect();
        let assembler = Arc::new(Mutex::new(AudioAssembler::default()));
        let value = self
            .call_with_audio(
                "enhance",
                wavs,
                |request_id, audio| protocol::enhance_args(request_id, task_id, lanes, audio),
                Events::new(progress, Some(assembler.clone())),
            )
            .await?;
        let result: EnhanceResult = protocol::decode("enhance", value)?;
        if result.model != GOLD_ENHANCEMENT_MODEL.id {
            return Err(InferenceError::other(format!(
                "The AI engine used a different denoise model. (got {}, expected {})",
                result.model, GOLD_ENHANCEMENT_MODEL.id
            )));
        }
        let mut assembler = assembler.lock().unwrap_or_else(|e| e.into_inner());
        lanes
            .iter()
            .map(|lane| {
                let metadata = result
                    .lanes
                    .iter()
                    .find(|metadata| metadata.lane_id == lane.lane_id)
                    .ok_or_else(|| {
                        InferenceError::other(format!(
                            "Denoise returned no audio for track {}.",
                            lane.lane_id
                        ))
                    })?;
                let wav = assembler
                    .take(&lane.lane_id, metadata.total_bytes)
                    .map_err(|error| {
                        InferenceError::other(format!(
                            "Denoise returned incomplete audio. ({error})"
                        ))
                    })?;
                Ok(EnhancedLane {
                    lane_id: lane.lane_id.clone(),
                    model_id: result.model.clone(),
                    wav,
                })
            })
            .collect()
    }

    async fn align(
        &self,
        request: AlignRequest,
        progress: ProgressSink,
    ) -> InferenceResult<AlignResponse> {
        let language = request.language.clone();
        let value = self
            .call_with_audio(
                "align",
                vec![request.wav],
                |request_id, audio| protocol::align_args(request_id, &language, &audio[0]),
                Events::new(progress, None),
            )
            .await?;
        let result: AlignResult = protocol::decode("align", value)?;
        Ok(AlignResponse {
            tokens: result.tokens(),
            duration_seconds: result.duration_seconds,
            model: result.receipt.model,
        })
    }
}

#[cfg(not(windows))]
fn unsupported() -> InferenceError {
    InferenceError::new(
        studio_inference::InferenceErrorKind::Unsupported,
        "Running AI on this computer needs Windows with Microsoft Edge WebView2. Use a remote server instead.",
    )
}

/// Routes a request's page events to its progress sink (and enhancement audio assembler).
struct Events {
    progress: ProgressSink,
    audio: Option<Arc<Mutex<AudioAssembler>>>,
}

impl Events {
    fn new(progress: ProgressSink, audio: Option<Arc<Mutex<AudioAssembler>>>) -> Self {
        Self { progress, audio }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    fn handle(&self, event: &Value) {
        match protocol::decode_event(event) {
            Ok(Some(HostEvent::Progress(progress))) => self.progress.emit(progress),
            Ok(Some(HostEvent::Audio {
                lane_id,
                offset,
                total_bytes,
                data,
            })) => match &self.audio {
                Some(audio) => audio.lock().unwrap_or_else(|e| e.into_inner()).push(
                    &lane_id,
                    offset,
                    total_bytes,
                    &data,
                ),
                None => eprintln!("local inference: unexpected audio event for lane {lane_id}"),
            },
            Ok(None) => {}
            Err(error) => match &self.audio {
                Some(audio) => audio
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_error(error),
                None => eprintln!("local inference: malformed page event: {error}"),
            },
        }
    }

    #[cfg(windows)]
    fn into_sink(self) -> crate::host::EventSink {
        Arc::new(move |event| self.handle(&event))
    }
}

/// One request's staged lane files; deleted on drop.
struct StagedAudio {
    paths: Vec<PathBuf>,
    refs: Vec<AudioRef>,
}

impl StagedAudio {
    fn write(dir: &Path, request_id: &str, wavs: Vec<Bytes>) -> InferenceResult<Self> {
        let staging_error = |error: std::io::Error| {
            InferenceError::other(format!("Couldn't prepare the audio. ({error})"))
        };
        std::fs::create_dir_all(dir).map_err(staging_error)?;
        let mut staged = Self {
            paths: Vec::with_capacity(wavs.len()),
            refs: Vec::with_capacity(wavs.len()),
        };
        for (index, wav) in wavs.iter().enumerate() {
            let file_name = format!("{request_id}-{index}.wav");
            let path = dir.join(&file_name);
            std::fs::write(&path, wav).map_err(staging_error)?;
            staged.paths.push(path);
            staged.refs.push(AudioRef {
                field_name: format!("lane{index}"),
                url: format!("https://{}/{file_name}", AUDIO_HOST),
            });
        }
        Ok(staged)
    }
}

impl Drop for StagedAudio {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(windows)]
mod windows_host {
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::Duration,
    };

    use studio_inference::{InferenceError, InferenceErrorKind};

    use super::LocalConfig;
    use crate::host::{HostConfig, HostError, PageHost};

    /// The running page host, started on demand and replaced after it dies.
    #[derive(Default)]
    pub struct HostSlot {
        host: Mutex<Option<Arc<PageHost>>>,
        starting: tokio::sync::Mutex<()>,
    }

    impl HostSlot {
        fn running(&self) -> Option<Arc<PageHost>> {
            self.host
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .filter(|host| host.is_alive())
                .cloned()
        }

        pub async fn get_or_start(
            &self,
            config: &LocalConfig,
            extension_dir: PathBuf,
        ) -> Result<Arc<PageHost>, HostError> {
            if let Some(host) = self.running() {
                return Ok(host);
            }
            let _starting = self.starting.lock().await;
            if let Some(host) = self.running() {
                return Ok(host);
            }
            // A dead host's thread has already left its message loop; dropping joins it.
            let dead = self.host.lock().unwrap_or_else(|e| e.into_inner()).take();
            if dead.is_some() {
                eprintln!("local inference: the page host stopped; starting a new one");
            }
            let audio_dir = config.audio_dir();
            let host_config = HostConfig {
                extension_dir,
                user_data_dir: config.profile_dir(),
                audio_dir: audio_dir.clone(),
            };
            let host = tokio::task::spawn_blocking(move || {
                drop(dead);
                // Requests stage their audio before the host starts, and other Studio processes
                // share the folder: only remove files abandoned by crashed runs.
                remove_stale_files(&audio_dir);
                PageHost::start(host_config)
            })
            .await
            .map_err(|error| HostError::Crashed(format!("host start panicked: {error}")))??;
            let host = Arc::new(host);
            *self.host.lock().unwrap_or_else(|e| e.into_inner()) = Some(host.clone());
            Ok(host)
        }

        pub fn shutdown(&self) {
            let host = self.host.lock().unwrap_or_else(|e| e.into_inner()).take();
            drop(host);
        }
    }

    /// Staged files outlive their request only when Studio died mid-request.
    const STALE_STAGED_AUDIO: Duration = Duration::from_secs(24 * 60 * 60);

    fn remove_stale_files(dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > STALE_STAGED_AUDIO);
            if stale {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// Cancels the page request if the caller drops the future before it settles.
    pub struct CancelOnDrop<'a> {
        host: &'a PageHost,
        request_id: &'a str,
        armed: bool,
    }

    impl<'a> CancelOnDrop<'a> {
        pub fn new(host: &'a PageHost, request_id: &'a str, armed: bool) -> Self {
            Self {
                host,
                request_id,
                armed,
            }
        }

        pub fn disarm(mut self) {
            self.armed = false;
        }
    }

    impl Drop for CancelOnDrop<'_> {
        fn drop(&mut self) {
            if self.armed {
                self.host.cancel(self.request_id);
            }
        }
    }

    pub fn host_error(error: HostError) -> InferenceError {
        match error {
            HostError::Page(error) => crate::protocol::page_error(error),
            other => InferenceError::new(InferenceErrorKind::Other, other.to_string()),
        }
    }
}
