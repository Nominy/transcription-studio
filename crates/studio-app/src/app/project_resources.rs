use super::{Message, StudioState, service_executor::execute_project_resource};
use iced::widget::{button, column, container, horizontal_rule, row, scrollable, text};
use iced::{Element, Fill, Task};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum ResourcesMessage {
    Toggle,
    Tab(u8),
    Reload,
    Loaded(String, Bundle),
    Document(String),
    DocumentLoaded(String, String, Result<Value, String>),
    CopyLink(String),
}
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Bundle {
    instructions: Result<Value, String>,
    grading: Result<Value, String>,
    documents: Result<Value, String>,
}
#[derive(Default)]
pub(super) struct ResourcesState {
    project: String,
    pending: bool,
    open: bool,
    tab: u8,
    data: Option<Bundle>,
    document_id: String,
    document: Option<Result<Value, String>>,
}
fn msg(value: ResourcesMessage) -> Message {
    Message::Resources(value)
}
fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub(super) fn ensure_current(state: &mut StudioState) -> Task<Message> {
    let project = state
        .evaluation
        .as_ref()
        .filter(|e| e.visible)
        .map(|e| e.project.clone())
        .or_else(|| {
            state
                .project()
                .and_then(|p| p.metadata.get(studio_app::META_BABEL_PROJECT_ID).cloned())
        })
        .unwrap_or_else(|| state.dashboard.project_id.clone());
    if project.is_empty() || state.native_auth_flow_blocks_service_dispatch() {
        return Task::none();
    }
    if state.project_resources.project == project {
        return Task::none();
    }
    state.project_resources = ResourcesState {
        project,
        open: true,
        ..Default::default()
    };
    load(&mut state.project_resources)
}
fn load(state: &mut ResourcesState) -> Task<Message> {
    state.pending = true;
    let project = state.project.clone();
    Task::perform(
        async move {
            let payload = json!({"projectId":project});
            let (instructions, grading, documents) = tokio::join!(
                execute_project_resource("project.getProjectInstructions", payload.clone()),
                execute_project_resource("rubric.getProjectRubricFlags", payload.clone()),
                execute_project_resource("project.listProjectDocuments", payload),
            );
            (
                project,
                Bundle {
                    instructions,
                    grading,
                    documents,
                },
            )
        },
        |(project, bundle)| msg(ResourcesMessage::Loaded(project, bundle)),
    )
}
pub(super) fn update(state: &mut StudioState, message: ResourcesMessage) -> Task<Message> {
    let resources = &mut state.project_resources;
    match message {
        ResourcesMessage::Toggle => resources.open = !resources.open,
        ResourcesMessage::Tab(tab) if tab <= 2 => resources.tab = tab,
        ResourcesMessage::Reload if !resources.pending => return load(resources),
        ResourcesMessage::Loaded(project, data) if project == resources.project => {
            resources.pending = false;
            resources.data = Some(data);
        }
        ResourcesMessage::Document(id) => {
            let known = resources
                .data
                .as_ref()
                .and_then(|d| d.documents.as_ref().ok())
                .and_then(Value::as_array)
                .is_some_and(|docs| docs.iter().any(|doc| s(doc, "documentId") == id));
            if !known {
                return Task::none();
            }
            resources.document_id = id.clone();
            resources.document = None;
            let project = resources.project.clone();
            return Task::perform(
                async move {
                    let result = execute_project_resource(
                        "project.getProjectDocument",
                        json!({"projectId":project,"documentId":id}),
                    )
                    .await;
                    (project, id, result)
                },
                |(p, d, r)| msg(ResourcesMessage::DocumentLoaded(p, d, r)),
            );
        }
        ResourcesMessage::DocumentLoaded(project, id, result)
            if project == resources.project && id == resources.document_id =>
        {
            resources.document = Some(result)
        }
        ResourcesMessage::CopyLink(link)
            if url::Url::parse(&link).is_ok_and(|url| matches!(url.scheme(), "https" | "http")) =>
        {
            return iced::clipboard::write(link);
        }
        _ => {}
    }
    Task::none()
}

// Render all source text, including tables, without executing HTML or scripts.
// Long tables scroll horizontally so examples remain readable at desktop width.
fn markdown(source: &str) -> Element<'static, Message> {
    let mut content = column![].spacing(10);
    let mut table = Vec::new();
    let flush = |content: iced::widget::Column<'static, Message>, table: &mut Vec<Vec<String>>| {
        if table.is_empty() {
            return content;
        }
        let table_width = table.iter().map(Vec::len).max().unwrap_or(1) as f32 * 252.0;
        let mut rows = column![].spacing(6).width(table_width);
        for cells in table.drain(..) {
            let mut cells_row = row![].spacing(12);
            for cell in cells {
                cells_row = cells_row.push(container(text(cell).size(14)).width(240).padding(6));
            }
            rows = rows.push(cells_row).push(horizontal_rule(1));
        }
        content.push(
            scrollable(rows).direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            )),
        )
    };
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with('|') {
            let cells: Vec<String> = line
                .trim_matches('|')
                .split('|')
                .map(|s| s.trim().replace("\\@", "@"))
                .collect();
            if !cells
                .iter()
                .all(|s| s.chars().all(|c| matches!(c, '-' | ':' | ' ')))
            {
                table.push(cells);
            }
            continue;
        }
        content = flush(content, &mut table);
        if line.is_empty() {
            continue;
        }
        let level = line.chars().take_while(|c| *c == '#').count();
        let label = line
            .trim_start_matches('#')
            .trim()
            .trim_start_matches('>')
            .trim()
            .replace("**", "")
            .replace("\\@", "@");
        let size = if level > 0 {
            (24usize.saturating_sub(level * 2)).max(16) as u16
        } else {
            15
        };
        content = content.push(text(label).size(size));
    }
    flush(content, &mut table).into()
}
fn document_body(value: &Value) -> Element<'static, Message> {
    let body = &value["body"];
    let mut content = column![text(s(value, "name").to_owned()).size(20)].spacing(10);
    if value["needsAcknowledgement"] == true {
        content = content.push(text("Updated document — review it before working. Acknowledgement is available on the Babel website."));
    }
    if let Some(table) = body.get("table") {
        let columns = table["columns"].as_array().cloned().unwrap_or_default();
        let rows = table["rows"].as_array().cloned().unwrap_or_default();
        let mut table_view = column![]
            .spacing(8)
            .width(columns.len().max(1) as f32 * 252.0);
        let mut header = row![].spacing(12);
        for column in &columns {
            header = header.push(text(s(column, "label").to_owned()).width(240));
        }
        table_view = table_view.push(header).push(horizontal_rule(1));
        for record in rows {
            let mut cells = row![].spacing(12);
            for column in &columns {
                let cell = &record[s(column, "key")];
                let value = cell.as_str().map(str::to_owned).unwrap_or_else(|| {
                    if cell.is_null() {
                        String::new()
                    } else {
                        cell.to_string()
                    }
                });
                cells = cells.push(text(value).width(240));
            }
            table_view = table_view.push(cells).push(horizontal_rule(1));
        }
        return content
            .push(
                scrollable(table_view).direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::default(),
                )),
            )
            .into();
    }
    for key in ["markdown", "content", "text"] {
        if let Some(source) = body.get(key).and_then(Value::as_str) {
            return content.push(markdown(source)).into();
        }
    }
    content.push(text("This library item is a file or media document. Open it in the project's Library on Babel."))
        .push(text(s(&value["latest"],"fileName").to_owned())).into()
}
pub(super) fn view(state: &ResourcesState, compact: bool) -> Element<'_, Message> {
    if state.project.is_empty() {
        return column![].into();
    }
    let mut content = column![
        row![
            button(if state.open {
                "Instructions / Library ▾"
            } else {
                "Instructions / Library ▸"
            })
            .on_press(msg(ResourcesMessage::Toggle)),
            button("Refresh rules")
                .on_press_maybe((!state.pending).then_some(msg(ResourcesMessage::Reload))),
        ]
        .spacing(12)
    ]
    .spacing(12);
    if !state.open {
        return container(content).padding(12).into();
    }
    content = content.push(
        row![
            button(if state.tab < 2 {
                "● Instructions"
            } else {
                "Instructions"
            })
            .on_press(msg(ResourcesMessage::Tab(0))),
            button(if state.tab == 2 {
                "● Library"
            } else {
                "Library"
            })
            .on_press(msg(ResourcesMessage::Tab(2))),
        ]
        .spacing(8),
    );
    if state.tab < 2 {
        content = content.push(
            row![
                button(if state.tab == 0 {
                    "● How to do the task"
                } else {
                    "How to do the task"
                })
                .on_press(msg(ResourcesMessage::Tab(0))),
                button(if state.tab == 1 {
                    "● Grading guide"
                } else {
                    "Grading guide"
                })
                .on_press(msg(ResourcesMessage::Tab(1))),
            ]
            .spacing(8),
        );
    }
    let mut body = column![].spacing(10);
    if state.pending {
        body = body.push(text("Loading project rules…"));
    }
    if let Some(data) = &state.data {
        let selected = match state.tab {
            1 => &data.grading,
            2 => &data.documents,
            _ => &data.instructions,
        };
        match selected {
            Err(error) => body = body.push(text(error)),
            Ok(value) => match state.tab {
                0 => {
                    if value["isAcknowledged"] == false {
                        body = body.push(text(
                            "Updated instructions. Read the current version below.",
                        ));
                    }
                    let source = s(value, "instructions");
                    body = body.push(if source.is_empty() {
                        text("No instructions published for this project.").into()
                    } else {
                        markdown(source)
                    });
                }
                1 => {
                    if let Some(flags) = value.as_array().filter(|v| !v.is_empty()) {
                        for flag in flags {
                            let name = if s(flag, "friendlyName").is_empty() {
                                s(flag, "name")
                            } else {
                                s(flag, "friendlyName")
                            };
                            body = body
                                .push(text(name).size(18))
                                .push(markdown(s(flag, "description")));
                            if !s(flag, "extendedDescription").is_empty() {
                                body = body.push(markdown(s(flag, "extendedDescription")));
                            }
                        }
                    } else {
                        body = body.push(text("No grading guide published for this project."));
                    }
                }
                _ => {
                    if let Some(docs) = value.as_array().filter(|v| !v.is_empty()) {
                        for doc in docs {
                            body = body.push(button(text(s(doc, "name"))).on_press(msg(
                                ResourcesMessage::Document(s(doc, "documentId").into()),
                            )));
                        }
                    } else {
                        body = body.push(text("No library documents published for this project."));
                    }
                    if !state.document_id.is_empty() {
                        body = body.push(match &state.document {
                            Some(Ok(doc)) => document_body(doc),
                            Some(Err(error)) => text(error).into(),
                            None => text("Loading document…").into(),
                        });
                    }
                    body = body.push(button("Copy Babel Library link").on_press(msg(
                        ResourcesMessage::CopyLink(format!(
                            "https://dashboard.babel.audio/projects/{}?tab=library",
                            state.project
                        )),
                    )));
                }
            },
        }
    }
    if compact {
        content = content.push(scrollable(body).height(240));
    } else {
        content = content.push(body);
    }
    container(content).padding(16).width(Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_project_and_document_responses_do_not_replace_current_rules() {
        let mut state = StudioState::default();
        state.project_resources.project = "new".into();
        state.project_resources.document_id = "current-document".into();
        let data = Bundle {
            instructions: Ok(json!({"instructions":"Old"})),
            grading: Ok(json!([])),
            documents: Ok(json!([])),
        };
        let _ = update(&mut state, ResourcesMessage::Loaded("old".into(), data));
        assert!(state.project_resources.data.is_none());
        let _ = update(
            &mut state,
            ResourcesMessage::DocumentLoaded("new".into(), "old-document".into(), Ok(json!({}))),
        );
        assert!(state.project_resources.document.is_none());
    }
    #[test]
    fn rules_tables_and_library_render_without_html_execution() {
        let _ = markdown(
            "## Правила\n\n| Written | Spoken |\n| --- | --- |\n| 04.03 | четвёртое марта |\n<script>example</script>",
        );
        let _ = document_body(
            &json!({"name":"Style guide","body":{"kind":"table","table":{"columns":[{"key":"a","label":"Written"}],"rows":[{"a":"04.03"}]}}}),
        );
    }
}
