use super::{
    Message, StudioState,
    audio_backend::{AudioPlaybackBackend, PlaybackTrack},
};
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Task};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use studio_waveform::DecodedAudio;

pub(super) fn is_supported_project(id: &str) -> bool {
    matches!(
        id,
        "d35d73aa-4816-4571-b77d-960ddb15eeae"
            | "2c68b806-ca86-419f-83ea-2551e85e4619"
            | "3882e77e-3ace-4ff2-b2df-290e3439cfce"
    )
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum EvaluationMessage {
    Load,
    Reply(String, Result<Value, String>),
    Choose(String, String),
    Play(usize),
    Audio(usize, Result<Arc<DecodedAudio>, String>),
    Stop,
    Next,
    Previous,
    Submit,
    Tick,
    Back,
}

pub(super) struct EvaluationState {
    pub(super) visible: bool,
    owner_route: Option<(String, String)>,
    pub(super) project: String,
    session: String,
    tasks: Vec<Value>,
    index: usize,
    answers: BTreeMap<String, BTreeMap<String, String>>,
    listened: BTreeSet<String>,
    audio: BTreeMap<usize, Arc<DecodedAudio>>,
    player: AudioPlaybackBackend,
    playing: Option<(String, Instant, f64)>,
    pending: bool,
    audio_pending: bool,
    hold_pending: bool,
    last_hold: Instant,
    last_tick: Instant,
    expired: bool,
    error: Option<String>,
    notice: String,
}

fn msg(m: EvaluationMessage) -> Message {
    Message::Evaluation(m)
}
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn request(e: &EvaluationState, op: &str, payload: Value) -> Task<Message> {
    let operation = op.to_string();
    Task::perform(
        super::service_executor::execute_evaluation_request(
            operation.clone(),
            payload,
            e.owner_route.clone(),
        ),
        move |r| msg(EvaluationMessage::Reply(operation.clone(), r)),
    )
}

pub(super) fn open(state: &mut StudioState, project: String) -> Task<Message> {
    if !is_supported_project(&project) {
        return Task::none();
    }
    if let Some(e) = state.evaluation.as_mut() {
        if e.project != project {
            if e.pending
                || e.audio_pending
                || e.hold_pending
                || (!e.session.is_empty() && !e.expired)
            {
                e.visible = true;
                e.error =
                    Some("Finish the current project's session before switching projects.".into());
                return Task::none();
            }
            e.stop();
            state.evaluation = None;
        }
    }
    if state.evaluation.is_none() {
        state.evaluation = Some(EvaluationState {
            visible: true,
            owner_route: None,
            project,
            session: String::new(),
            tasks: vec![],
            index: 0,
            answers: BTreeMap::new(),
            listened: BTreeSet::new(),
            audio: BTreeMap::new(),
            player: AudioPlaybackBackend::default(),
            playing: None,
            pending: false,
            audio_pending: false,
            hold_pending: false,
            last_hold: Instant::now(),
            error: None,
            last_tick: Instant::now(),
            expired: false,
            notice: "Listen to each audio clip fully, then answer the questions.".into(),
        });
    }
    let e = state.evaluation.as_mut().unwrap();
    e.visible = true;
    if !super::subaccounts::subaccount_session_is_active() {
        if let Some(auth) = state.relay_process.owner_authorization.clone() {
            e.owner_route = Some((state.relay_process.url.clone(), auth));
        } else {
            e.notice="Starting the local relay. If it fails, check Projects > Sub-accounts, then retry Load session.".into();
            return state.begin_subaccount_relay_start();
        }
    }
    if !e.session.is_empty() && !e.expired {
        return Task::none();
    }
    update(state, EvaluationMessage::Load)
}

impl EvaluationState {
    fn draft_path(&self) -> PathBuf {
        super::platform_paths::application_support_dir().join(format!("eval-{}.json", self.project))
    }
    fn persist(&self) -> Result<(), String> {
        let path = self.draft_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // Keep only answer values locally; expiring signed media URLs are not persisted.
        let temporary = path.with_extension("pending.json");
        let backup = path.with_extension("backup.json");
        fs::write(
            &temporary,
            json!({"sessionId":self.session,"answers":self.answers}).to_string(),
        )
        .map_err(|e| e.to_string())?;
        if path.exists() {
            fs::copy(&path, &backup).map_err(|e| e.to_string())?;
        }
        fs::rename(&temporary, &path).map_err(|e| e.to_string())
    }
    fn task(&self) -> Option<&Value> {
        self.tasks.get(self.index)
    }
    fn stop(&mut self) {
        self.player.stop();
        self.playing = None;
    }
    fn validate(&self, index: usize) -> Result<(), String> {
        if self.expired || self.session.is_empty() {
            return Err("Reload the expired session before continuing.".into());
        }
        let t = self.tasks.get(index).ok_or("No evaluation task loaded.")?;
        validate_task(t)?;
        for subject in array(t, "subjects")
            .iter()
            .filter(|subject| is_audio(subject))
        {
            if !self.listened.contains(s(subject, "id")) {
                return Err("Listen to every clip fully before continuing.".into());
            }
        }
        let answers = self.answers.get(s(t, "taskId"));
        for q in array(t, "questions") {
            if !question_visible(q, t, answers) {
                continue;
            }
            if q.pointer("/metadata/required") == Some(&Value::Bool(false))
                && answers
                    .and_then(|a| a.get(s(q, "id")))
                    .is_none_or(|v| v.trim().is_empty())
            {
                continue;
            }
            let value = answers
                .and_then(|a| a.get(s(q, "id")))
                .ok_or("Answer every question before continuing.")?;
            if (s(q, "type") == "text" && value.trim().is_empty())
                || (s(q, "type") != "text"
                    && !array(q, "values").iter().any(|v| v.as_str() == Some(value)))
            {
                return Err("Invalid answer value.".into());
            }
        }
        Ok(())
    }
    fn payload(&self, only_current: bool) -> Value {
        let tasks: Vec<Value> = self
            .tasks
            .iter()
            .enumerate()
            .filter(|(i, _)| !only_current || *i == self.index)
            .map(|(_, t)| {
                let values = self.answers.get(s(t, "taskId"));
                let responses: Vec<Value> = array(t, "questions")
                    .iter()
                    .filter_map(|q| {
                        if !question_visible(q, t, values) {
                            return None;
                        }
                        let value = values?.get(s(q, "id"))?;
                        Some(json!({"formInputId":s(q,"id"),"value":value}))
                    })
                    .collect();
                json!({"taskId":s(t,"taskId"),"inputResponses":responses})
            })
            .collect();
        json!({"sessionId":self.session,"tasks":tasks})
    }
}

fn is_audio(subject: &Value) -> bool {
    !s(subject, "mediaUrl").is_empty()
}

fn question_visible(q: &Value, task: &Value, answers: Option<&BTreeMap<String, String>>) -> bool {
    let Some(condition) = q.pointer("/metadata/showWhen").filter(|v| !v.is_null()) else {
        return true;
    };
    match s(condition, "kind") {
        "answer" => answers
            .and_then(|a| a.get(s(condition, "questionId")))
            .is_some_and(|answer| {
                array(condition, "values")
                    .iter()
                    .any(|v| v.as_str() == Some(answer))
            }),
        "scenario" => {
            s(&task["scenarioContext"], s(condition, "field"))
                .trim()
                .to_lowercase()
                == s(condition, "value").trim().to_lowercase()
        }
        _ => false,
    }
}

fn validate_task(t: &Value) -> Result<(), String> {
    if s(t, "taskId").is_empty()
        || !matches!(
            s(t, "format"),
            "tts-naturalness" | "default" | "tts-model" | "tts-model-subtitles"
        )
        || array(t, "subjects").is_empty()
        || array(t, "questions").is_empty()
    {
        return Err("This evaluation task has an unsupported format.".into());
    }
    let audio_count = array(t, "subjects")
        .iter()
        .filter(|subject| is_audio(subject))
        .count();
    if audio_count == 0 || (s(t, "format") == "tts-naturalness" && audio_count != 2) {
        return Err("Audio subjects are missing or incomplete.".into());
    }
    let mut ids = BTreeSet::new();
    for subject in array(t, "subjects") {
        if s(subject, "id").is_empty()
            || !ids.insert(s(subject, "id"))
            || (is_audio(subject) && !s(subject, "mediaUrl").starts_with("https://"))
            || (!is_audio(subject) && s(subject, "transcript").is_empty())
        {
            return Err("Incomplete audio metadata returned by Babel.".into());
        }
    }
    if !array(t, "formLinters").is_empty() {
        return Err("This form requires additional validation that is not supported yet.".into());
    }
    let mut ids = BTreeSet::new();
    for q in array(t, "questions") {
        if s(q, "id").is_empty()
            || !ids.insert(s(q, "id"))
            || s(q, "label").is_empty()
            || !matches!(
                s(q, "type"),
                "side_by_side" | "mcq" | "likert" | "dropdown" | "text"
            )
            || (s(q, "type") != "text" && array(q, "values").is_empty())
            || (!array(q, "valueLabels").is_empty()
                && array(q, "values").len() != array(q, "valueLabels").len())
            || array(q, "values")
                .iter()
                .chain(array(q, "valueLabels"))
                .any(|v| v.as_str().is_none_or(str::is_empty))
            || !array(q, "linters").is_empty()
            || q.pointer("/metadata/showWhen")
                .filter(|v| !v.is_null())
                .is_some_and(|v| match s(v, "kind") {
                    "answer" => {
                        !array(t, "questions")
                            .iter()
                            .any(|other| s(other, "id") == s(v, "questionId"))
                            || array(v, "values").is_empty()
                    }
                    "scenario" => {
                        !matches!(s(v, "field"), "objective" | "domain" | "subdomain")
                            || s(v, "value").is_empty()
                    }
                    _ => true,
                })
        {
            return Err("This question requires a newer evaluation adapter.".into());
        }
    }
    Ok(())
}

pub(super) fn update(state: &mut StudioState, message: EvaluationMessage) -> Task<Message> {
    if matches!(message, EvaluationMessage::Load)
        && !super::subaccounts::subaccount_session_is_active()
        && state
            .evaluation
            .as_ref()
            .is_some_and(|e| e.owner_route.is_none())
    {
        let project = state.evaluation.as_ref().unwrap().project.clone();
        return open(state, project);
    }
    let Some(e) = state.evaluation.as_mut() else {
        return Task::none();
    };
    match message {
        EvaluationMessage::Load if !e.pending => {
            if !e.session.is_empty() && !e.expired {
                e.error = Some("Finish the current session before loading another.".into());
                return Task::none();
            }
            e.error = None;
            e.pending = true;
            return request(
                e,
                "evalTasks.serveSessionTasks",
                json!({"projectId":e.project}),
            );
        }
        EvaluationMessage::Reply(op, result) => {
            if op == "evalTasks.holdSession" {
                e.hold_pending = false;
                match result {
                    Ok(Value::Bool(true)) => {}
                    Ok(_) => {
                        e.expired = true;
                        e.stop();
                        e.error=Some("Session expired. Your answers are kept locally; reload the session to continue.".into());
                    }
                    Err(error) => e.error = Some(error),
                }
                return Task::none();
            }
            e.pending = false;
            match result {
                Err(error) => e.error = Some(error),
                Ok(v) if op == "evalTasks.serveSessionTasks" => {
                    if s(&v, "kind") == "empty" {
                        e.notice = if s(&v, "reason") == "all_at_consensus" {
                            "No available tasks: all current recordings have already received enough evaluations.".into()
                        } else {
                            format!("No available tasks: {}", s(&v, "reason"))
                        };
                        return Task::none();
                    }
                    if s(&v, "kind") != "session" {
                        e.error = Some(format!("Unsupported session type: {}", s(&v, "kind")));
                        return Task::none();
                    }
                    if s(&v, "sessionId").is_empty() || array(&v, "tasks").is_empty() {
                        e.error = Some("Babel returned an incomplete session.".into());
                        return Task::none();
                    }
                    e.expired = false;
                    e.notice = "Listen to each audio clip fully, then answer the questions.".into();
                    e.session = s(&v, "sessionId").into();
                    e.tasks = array(&v, "tasks").to_vec();
                    e.index = 0;
                    e.audio.clear();
                    e.listened.clear();
                    e.answers.clear();
                    for task in &mut e.tasks {
                        if let Some(subjects) =
                            task.get_mut("subjects").and_then(Value::as_array_mut)
                        {
                            subjects.sort_by_key(|v| v["position"].as_i64().unwrap_or(0));
                        }
                        if let Some(questions) =
                            task.get_mut("questions").and_then(Value::as_array_mut)
                        {
                            questions.sort_by_key(|v| v["sequenceNumber"].as_i64().unwrap_or(0));
                        }
                    }
                    if let Ok(raw) = fs::read_to_string(e.draft_path()).or_else(|_| {
                        fs::read_to_string(e.draft_path().with_extension("backup.json"))
                    }) {
                        if let Ok(draft) = serde_json::from_str::<Value>(&raw) {
                            if s(&draft, "sessionId") == e.session {
                                e.answers = serde_json::from_value(draft["answers"].clone())
                                    .unwrap_or_default();
                            }
                        }
                    }
                    for t in &e.tasks {
                        if let Err(error) = validate_task(t) {
                            e.error = Some(error);
                            break;
                        }
                    }
                    e.last_hold = Instant::now();
                }
                Ok(v) if op == "evalTasks.saveProgress" => {
                    if s(&v, "kind") != "saved" {
                        e.error =
                            Some("Babel did not confirm saving. Local answers retained.".into());
                    } else if e.index + 1 < e.tasks.len() {
                        e.stop();
                        e.index += 1;
                        e.audio.clear();
                        e.notice = "Progress saved".into();
                    } else {
                        e.notice = "Progress saved".into();
                    }
                }
                Ok(v) if op == "evalTasks.submitSession" => {
                    if s(&v, "kind") == "completed" {
                        e.stop();
                        e.session.clear();
                        e.tasks.clear();
                        e.answers.clear();
                        e.audio.clear();
                        e.notice =
                            "Session submitted successfully. Load the next session when ready."
                                .into();
                    } else {
                        e.error=Some("Babel did not confirm completion. Answers retained; check the session before retrying.".into());
                    }
                }
                Ok(_) => {}
            }
        }
        EvaluationMessage::Choose(id, value) if !e.pending => {
            if let Some(t) = e.task() {
                let task_id = s(t, "taskId").to_string();
                if array(t, "questions").iter().any(|q| {
                    s(q, "id") == id
                        && (s(q, "type") == "text"
                            || array(q, "values")
                                .iter()
                                .any(|v| v.as_str() == Some(&value)))
                }) {
                    e.answers.entry(task_id).or_default().insert(id, value);
                    // Hidden answers must not survive changes to a controlling question.
                    let task = e.task().unwrap().clone();
                    let answers = e.answers.get_mut(s(&task, "taskId")).unwrap();
                    for _ in 0..array(&task, "questions").len() {
                        let hidden: Vec<String> = array(&task, "questions")
                            .iter()
                            .filter(|q| !question_visible(q, &task, Some(answers)))
                            .map(|q| s(q, "id").to_owned())
                            .collect();
                        for id in hidden {
                            answers.remove(&id);
                        }
                    }
                    if let Err(error) = e.persist() {
                        e.error = Some(format!("Could not save local draft: {error}"));
                    }
                }
            }
        }
        EvaluationMessage::Play(index) if !e.pending && !e.audio_pending => {
            e.stop();
            e.error = None;
            if let Some(audio) = e.audio.get(&index).cloned() {
                let id = e
                    .task()
                    .and_then(|t| array(t, "subjects").get(index))
                    .map(|v| s(v, "id").to_string())
                    .unwrap_or_default();
                match e.player.play_tracks(
                    vec![PlaybackTrack::new(id.clone(), audio.clone())],
                    0.0,
                    1.0,
                ) {
                    Ok(_) => {
                        e.last_tick = Instant::now();
                        e.playing = Some((id, Instant::now(), audio.duration()));
                    }
                    Err(error) => e.error = Some(error),
                }
            } else if let Some(url) = e
                .task()
                .and_then(|t| array(t, "subjects").get(index))
                .map(|v| s(v, "mediaUrl").to_string())
            {
                e.audio_pending = true;
                return Task::perform(
                    async move {
                        let url = url::Url::parse(&url).map_err(|e| e.to_string())?;
                        if url.scheme() != "https" {
                            return Err("Audio requires HTTPS.".into());
                        }
                        let mut response = super::http_client::shared_http_client()
                            .get(url)
                            .timeout(Duration::from_secs(60))
                            .send()
                            .await
                            .map_err(|_| "Audio download failed. Try again.".to_string())?
                            .error_for_status()
                            .map_err(|_| "Audio link is unavailable or expired.".to_string())?;
                        let mut bytes = Vec::new();
                        while let Some(chunk) = response
                            .chunk()
                            .await
                            .map_err(|_| "Audio download interrupted.".to_string())?
                        {
                            if bytes.len() + chunk.len() > 64 * 1024 * 1024 {
                                return Err("Audio exceeds the 64 MB limit.".into());
                            }
                            bytes.extend_from_slice(&chunk);
                        }
                        tokio::task::spawn_blocking(move || {
                            super::evaluation_audio::decode(bytes).map(Arc::new)
                        })
                        .await
                        .map_err(|_| {
                            "Audio decoder failed. Your answers are retained.".to_string()
                        })?
                    },
                    move |r| msg(EvaluationMessage::Audio(index, r)),
                );
            }
        }
        EvaluationMessage::Audio(index, result) => {
            e.audio_pending = false;
            match result {
                Ok(audio) => {
                    e.audio.insert(index, audio);
                    return update(state, EvaluationMessage::Play(index));
                }
                Err(error) => e.error = Some(error),
            }
        }
        EvaluationMessage::Stop if !e.audio_pending => e.stop(),
        EvaluationMessage::Tick => {
            // Suspend/resume must never count as having listened to an entire clip.
            if e.last_tick.elapsed() > Duration::from_secs(3) {
                e.stop();
            }
            e.last_tick = Instant::now();
            if let Some((id, start, duration)) = &e.playing {
                if start.elapsed().as_secs_f64() >= *duration && e.player.playback_finished() {
                    e.listened.insert(id.clone());
                    e.playing = None;
                }
            }
            if e.visible
                && !e.expired
                && !e.session.is_empty()
                && !e.pending
                && !e.hold_pending
                && e.last_hold.elapsed() > Duration::from_secs(180)
            {
                e.hold_pending = true;
                e.last_hold = Instant::now();
                return request(e, "evalTasks.holdSession", json!({"sessionId":e.session}));
            }
        }
        EvaluationMessage::Next if !e.pending && !e.audio_pending => {
            match e.validate(e.index).and_then(|_| e.persist()) {
                Err(error) => e.error = Some(error),
                Ok(()) => {
                    e.error = None;
                    e.pending = true;
                    return request(e, "evalTasks.saveProgress", e.payload(true));
                }
            }
        }
        EvaluationMessage::Previous if !e.pending && !e.audio_pending && e.index > 0 => {
            e.stop();
            e.index -= 1;
            e.audio.clear();
        }
        EvaluationMessage::Submit if !e.pending && !e.audio_pending && !e.tasks.is_empty() => {
            let valid = (0..e.tasks.len())
                .try_for_each(|i| e.validate(i))
                .and_then(|_| e.persist());
            match valid {
                Err(error) => e.error = Some(error),
                Ok(()) => {
                    e.error = None;
                    e.pending = true;
                    return request(e, "evalTasks.submitSession", e.payload(false));
                }
            }
        }
        EvaluationMessage::Back if !e.pending && !e.audio_pending => {
            e.stop();
            if let Err(error) = e.persist() {
                e.error = Some(error);
            } else {
                e.visible = false;
            }
        }
        _ => {}
    }
    Task::none()
}

pub(super) fn view<'a>(
    e: &'a EvaluationState,
    resources: &'a super::project_resources::ResourcesState,
) -> Element<'a, Message> {
    let mut content = column![
        row![
            text(if e.project == "3882e77e-3ace-4ff2-b2df-290e3439cfce" {
                "RU Alpha Eval"
            } else if e.project == "2c68b806-ca86-419f-83ea-2551e85e4619" {
                "RU Naturalness Eval 2"
            } else {
                "RU Naturalness Eval"
            })
            .size(26),
            button("Projects").on_press(msg(EvaluationMessage::Back))
        ]
        .spacing(24),
        text(&e.notice)
    ]
    .spacing(16);
    if let Some(error) = &e.error {
        content = content.push(text(error).color(iced::Color::from_rgb8(180, 30, 30)));
    }
    if e.pending {
        content = content.push(text("Waiting for Babel…"));
    }
    if e.expired && !e.pending {
        content = content.push(button("Reload session").on_press(msg(EvaluationMessage::Load)));
    }
    if let Some(t) = e.task() {
        content = content.push(text(format!(
            "Form {} / {} — {}",
            e.index + 1,
            e.tasks.len(),
            s(t, "formDisplayName")
        )));
        let mut clips = row![].spacing(20);
        for (i, subject) in array(t, "subjects").iter().enumerate() {
            let label = if s(subject, "label").is_empty() {
                format!("Clip {}", (b'A' + i as u8) as char)
            } else {
                s(subject, "label").to_owned()
            };
            if !is_audio(subject) {
                content = content
                    .push(column![text(label).size(18), text(s(subject, "transcript"))].spacing(8));
                continue;
            }
            let heard = e.listened.contains(s(subject, "id"));
            clips = clips.push(
                column![
                    text(label.clone()).size(20),
                    text(s(subject, "emotion")),
                    text(s(subject, "transcript")),
                    button(text(format!("Play {label}"))).on_press_maybe(
                        (!e.pending && !e.audio_pending).then_some(msg(EvaluationMessage::Play(i)))
                    ),
                    text(if heard {
                        "Listened ✓"
                    } else {
                        "Listen fully"
                    })
                ]
                .spacing(8)
                .width(Fill),
            );
        }
        content = content
            .push(clips)
            .push(button("Stop audio").on_press(msg(EvaluationMessage::Stop)));
        for (key, label) in [
            ("writtenForm", "Written form"),
            ("spokenForm", "Spoken form"),
        ] {
            if !s(t, key).is_empty() {
                content = content.push(column![text(label).size(18), text(s(t, key))].spacing(6));
            }
        }
        if let Some((id, start, duration)) = &e.playing {
            let label = if array(t, "subjects")
                .first()
                .is_some_and(|v| s(v, "id") == id)
            {
                "A"
            } else {
                "B"
            };
            content = content.push(text(format!(
                "Playing {label}: {:.0} / {:.0} sec",
                start.elapsed().as_secs_f64().min(*duration),
                duration
            )));
        }
        if e.audio_pending {
            content = content.push(text("Loading audio…"));
        }
        for q in array(t, "questions") {
            if !question_visible(q, t, e.answers.get(s(t, "taskId"))) {
                continue;
            }
            let mut question = column![text(s(q, "label")).size(18)].spacing(5);
            if !s(q, "caption").is_empty() {
                question = question.push(text(s(q, "caption")));
            }
            if s(q, "type") == "text" {
                let value = e
                    .answers
                    .get(s(t, "taskId"))
                    .and_then(|a| a.get(s(q, "id")))
                    .map(String::as_str)
                    .unwrap_or_default();
                let id = s(q, "id").to_owned();
                question =
                    question.push(text_input("Your answer", value).on_input_maybe(
                        (!e.pending).then_some(move |value| {
                            msg(EvaluationMessage::Choose(id.clone(), value))
                        }),
                    ));
            }
            for (i, value) in array(q, "values").iter().enumerate() {
                let value = value.as_str().unwrap_or("");
                let selected = e
                    .answers
                    .get(s(t, "taskId"))
                    .and_then(|a| a.get(s(q, "id")))
                    .is_some_and(|v| v == value);
                let label = array(q, "valueLabels")
                    .get(i)
                    .and_then(Value::as_str)
                    .unwrap_or(value);
                question = question.push(
                    button(text(format!(
                        "{} {label}",
                        if selected { "●" } else { "○" }
                    )))
                    .on_press_maybe((!e.pending).then_some(msg(EvaluationMessage::Choose(
                        s(q, "id").into(),
                        value.into(),
                    ))))
                    .width(Fill),
                );
            }
            content = content.push(container(question).padding(12));
        }
        content = content.push(
            row![
                button("Previous form").on_press_maybe(
                    (e.index > 0 && !e.pending).then_some(msg(EvaluationMessage::Previous))
                ),
                button("Save / next form")
                    .on_press_maybe((!e.pending).then_some(msg(EvaluationMessage::Next))),
                button("Submit session").on_press_maybe(
                    (!e.pending && e.index + 1 == e.tasks.len())
                        .then_some(msg(EvaluationMessage::Submit))
                )
            ]
            .spacing(12),
        );
    } else if !e.pending {
        content = content.push(button("Load session").on_press(msg(EvaluationMessage::Load)));
    }
    content = content.push(super::project_resources::view(resources, false));
    container(scrollable(content.padding(24)))
        .width(Fill)
        .height(Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alpha_supports_one_audio_and_labeled_script_subjects() {
        let task = json!({"taskId":"alpha-task","format":"default","formLinters":[],
            "subjects":[{"id":"audio","label":"Recording","mediaUrl":"https://example.test/audio.wav"},
                {"id":"written","label":"Written form","transcript":"04.03."},
                {"id":"spoken","label":"Spoken form","transcript":"четвёртое марта"}],
            "questions":[{"id":"correct","label":"Read correctly?","type":"mcq","values":["yes","no"],"valueLabels":["Да","Нет"],"linters":[]},
                {"id":"why","label":"Explain","type":"text","metadata":{"showWhen":{"kind":"answer","questionId":"correct","values":["no"]}},"linters":[]}]});
        assert!(validate_task(&task).is_ok());
        assert_eq!(
            array(&task, "subjects")
                .iter()
                .filter(|s| is_audio(s))
                .count(),
            1
        );
        let mut answers = BTreeMap::from([("correct".into(), "yes".into())]);
        assert!(!question_visible(
            &task["questions"][1],
            &task,
            Some(&answers)
        ));
        answers.insert("correct".into(), "no".into());
        assert!(question_visible(
            &task["questions"][1],
            &task,
            Some(&answers)
        ));
        let mut unsafe_task = task;
        unsafe_task["formLinters"] = json!([{"linterKey":"unknown"}]);
        assert!(validate_task(&unsafe_task).is_err());
    }

    #[test]
    fn scenario_visibility_matches_site_case_and_whitespace_rules() {
        let q =
            json!({"metadata":{"showWhen":{"kind":"scenario","field":"domain","value":" PHONE "}}});
        assert!(question_visible(
            &q,
            &json!({"scenarioContext":{"domain":"phone"}}),
            None
        ));
        assert!(!question_visible(
            &q,
            &json!({"scenarioContext":{"domain":"dates"}}),
            None
        ));
    }
    #[test]
    fn unknown_forms_are_not_silently_submitted() {
        assert!(
            validate_task(&json!({"taskId":"t","format":"other","questions":[{}],"subjects":[{}]}))
                .is_err()
        );
    }
    #[test]
    fn accepts_observed_naturalness_contract() {
        let task = fixture();
        assert!(validate_task(&task).is_ok());
    }

    fn fixture() -> Value {
        json!({"taskId":"t","format":"tts-naturalness","subjects":[{"id":"a","mediaUrl":"https://example.test/a.wav"},{"id":"b","mediaUrl":"https://example.test/b.wav"}],"formLinters":[],"questions":[{"id":"q","label":"Which sounds natural?","type":"side_by_side","values":["a_better","tie","b_better"],"valueLabels":["A","Same","B"],"linters":[]}]})
    }

    #[test]
    fn rejects_missing_audio_and_conditional_questions() {
        let mut task = fixture();
        task["subjects"][0]["mediaUrl"] = json!("");
        assert!(validate_task(&task).is_err());
        let mut task = fixture();
        task["questions"][0]["metadata"] = json!({"showWhen":{"id":"other"}});
        assert!(validate_task(&task).is_err());
    }

    #[test]
    fn answers_must_be_complete_and_listened_and_preserve_server_ids() {
        let mut e = EvaluationState {
            visible: true,
            owner_route: None,
            project: "project".into(),
            session: "session".into(),
            tasks: vec![fixture()],
            index: 0,
            answers: BTreeMap::new(),
            listened: BTreeSet::new(),
            audio: BTreeMap::new(),
            player: AudioPlaybackBackend::default(),
            playing: None,
            pending: false,
            audio_pending: false,
            hold_pending: false,
            last_hold: Instant::now(),
            last_tick: Instant::now(),
            expired: false,
            error: None,
            notice: String::new(),
        };
        assert!(e.validate(0).is_err());
        e.listened.extend(["a".into(), "b".into()]);
        assert!(e.validate(0).is_err());
        e.answers
            .insert("t".into(), BTreeMap::from([("q".into(), "tie".into())]));
        assert!(e.validate(0).is_ok());
        assert_eq!(
            e.payload(false),
            json!({"sessionId":"session","tasks":[{"taskId":"t","inputResponses":[{"formInputId":"q","value":"tie"}]}]})
        );
        e.answers
            .get_mut("t")
            .unwrap()
            .insert("q".into(), "invalid".into());
        assert!(e.validate(0).is_err());
        e.expired = true;
        assert!(e.validate(0).is_err());
    }
}
