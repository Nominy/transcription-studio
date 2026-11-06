//! The bundled sub-account relay process and the remembered relay host URL, shared by the
//! dashboard (Settings, Sub-accounts) and the tray menu.

use gpui_kit::*;
use sonus_babel_services::babel::DashboardProject;
use sonus_babel_services::relay::{RelayLaunchConfig, RelayProcess, RelayStatus};
use sonus_babel_services::subaccounts::{self, owner};

/// User-facing outcome of a relay action; the root shows it as a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayEvent {
    Info(SharedString),
    Error(SharedString),
}

pub struct RelayModel {
    process: RelayProcess,
    /// Owner relay host URL (remembered across launches); empty when never configured.
    host_url: String,
    start_task: Option<Task<()>>,
    sync_task: Option<Task<()>>,
}

impl EventEmitter<RelayEvent> for RelayModel {}

impl Default for RelayModel {
    fn default() -> Self {
        Self::new()
    }
}

impl RelayModel {
    pub fn new() -> Self {
        Self {
            process: RelayProcess::default(),
            host_url: subaccounts::default_relay_host_url(),
            start_task: None,
            sync_task: None,
        }
    }

    pub fn host_url(&self) -> &str {
        &self.host_url
    }

    pub fn status(&self) -> RelayStatus {
        self.process.status()
    }

    pub fn can_start(&self) -> bool {
        self.process.can_start()
    }

    pub fn can_stop(&self) -> bool {
        self.process.can_stop()
    }

    pub fn status_label(&self) -> String {
        match self.process.status() {
            RelayStatus::Stopped => "Stopped".to_string(),
            RelayStatus::Starting => "Starting…".to_string(),
            RelayStatus::Running => format!("Running at {}", self.process.url()),
            RelayStatus::Failed => format!(
                "Failed: {}",
                self.process.last_error().unwrap_or("unknown error")
            ),
        }
    }

    /// Normalizes and remembers `host_url`. Returns the normalized URL.
    pub fn remember_host_url(
        &mut self,
        host_url: &str,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let normalized = subaccounts::save_default_relay_host_url(host_url)
            .map_err(|error| format!("{error:#}"))?;
        self.host_url = normalized.clone();
        cx.notify();
        Ok(normalized)
    }

    /// Adopts a host URL that is already persisted (sub-account login).
    pub fn adopt_host_url(&mut self, host_url: String, cx: &mut Context<Self>) {
        self.host_url = host_url;
        cx.notify();
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if !self.process.can_start() {
            cx.emit(RelayEvent::Info(
                format!("Sub-account relay is already {}.", self.status_label()).into(),
            ));
            return;
        }
        let report = |error: anyhow::Error, cx: &mut Context<Self>| {
            cx.emit(RelayEvent::Error(
                format!("Sub-account relay start failed: {error:#}").into(),
            ));
            cx.notify();
        };
        let config = match RelayLaunchConfig::from_environment() {
            Ok(config) => config,
            Err(error) => return report(error, cx),
        };
        let ready = match self.process.start(&config) {
            Ok(ready) => ready,
            Err(error) => return report(error, cx),
        };
        cx.notify();
        self.start_task = Some(cx.spawn(async move |this, cx| {
            let result = ready.await;
            this.update(cx, |this, cx| this.complete_start(result, cx))
                .ok();
        }));
    }

    fn complete_start(&mut self, result: anyhow::Result<String>, cx: &mut Context<Self>) {
        self.start_task = None;
        self.process.complete_start(&result);
        match result {
            Ok(url) => {
                if self.host_url.trim().is_empty() {
                    match subaccounts::save_default_relay_host_url(&url) {
                        Ok(normalized) => self.host_url = normalized,
                        Err(error) => {
                            self.host_url = url.clone();
                            cx.emit(RelayEvent::Error(
                                format!(
                                    "Relay running at {url}; URL was not remembered: {error:#}"
                                )
                                .into(),
                            ));
                        }
                    }
                }
                cx.emit(RelayEvent::Info(
                    format!("Sub-account relay running at {url}.").into(),
                ));
            }
            Err(error) => cx.emit(RelayEvent::Error(
                format!("Sub-account relay start failed: {error:#}").into(),
            )),
        }
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.start_task = None;
        let was_running = self.process.can_stop();
        let event = match self.process.stop() {
            Ok(()) if was_running => RelayEvent::Info("Sub-account relay stopped.".into()),
            Ok(()) => RelayEvent::Info("Sub-account relay is not running.".into()),
            Err(error) => {
                RelayEvent::Error(format!("Sub-account relay stop failed: {error:#}").into())
            }
        };
        cx.emit(event);
        cx.notify();
    }

    /// Stops the relay without reporting (application exit).
    pub fn shutdown(&mut self) {
        self.start_task = None;
        self.sync_task = None;
        let _ = self.process.stop();
    }

    /// Owners push the dashboard's project queue ids to their relay so sub-accounts can claim
    /// from them; called only by the direct Babel owner's dashboard.
    pub fn sync_project_queues(&mut self, projects: &[DashboardProject], cx: &mut Context<Self>) {
        let configs: Vec<owner::ProjectQueueConfig> = projects
            .iter()
            .map(|project| owner::ProjectQueueConfig {
                project_id: project.id.clone(),
                chunked_queue_id: project.chunked_queue_id.clone(),
                stitched_queue_id: project.stitched_queue_id.clone(),
            })
            .collect();
        let Some(host_url) = owner::project_queue_sync_host(false, &self.host_url, &configs) else {
            return;
        };
        self.sync_task = Some(sonus_shell::services::spawn_service(
            cx,
            async move { owner::push_project_queue_configs(&host_url, &configs).await },
            |this, result, cx| {
                this.sync_task = None;
                if let Err(error) = result {
                    cx.emit(RelayEvent::Error(
                        format!("Sub-account project sync failed: {error:#}").into(),
                    ));
                }
            },
        ));
    }
}
