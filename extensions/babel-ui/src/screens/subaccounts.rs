//! Sub-account owner management: create / revoke relay logins, project access, analytics.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use exaudio_babel_services::babel::DashboardProject;
use exaudio_babel_services::subaccounts::owner::{self, ManagedSubaccount, OwnerDashboard};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::ShellModels;
use crate::text::{count, is_russian, localized, localized_fmt};
use exaudio_shell::screens::ui;
use exaudio_shell::services::spawn_service_in;

/// Asks the dashboard to (re)load projects for the access checkboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadProjects;

enum OwnerRequest {
    Load,
    Create(String),
    Revoke(String),
    Assign(String, Vec<String>),
}

pub struct SubaccountsView {
    /// The owner account's `auth.json`; owner requests authenticate with it.
    auth_store: PathBuf,
    projects: Vec<DashboardProject>,
    host_url: Entity<InputState>,
    create_label: Entity<InputState>,
    subaccounts: Vec<ManagedSubaccount>,
    /// Draft project access per sub-account; empty = every project.
    assignments: BTreeMap<String, Vec<String>>,
    /// Plain headline plus optional technical text for the Details disclosure.
    error: Option<(SharedString, Option<SharedString>)>,
    request: Option<Task<()>>,
    loaded: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LoadProjects> for SubaccountsView {}

impl SubaccountsView {
    pub fn new(auth_store: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let relay = ShellModels::relay(cx);
        let host = relay.read(cx).host_url().to_string();
        let host_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(cx, "Relay address", "Адрес ретранслятора"))
                .default_value(host)
        });
        let create_label = cx.new(|cx| {
            InputState::new(window, cx).placeholder(localized(
                cx,
                "Name for a new sub-account, e.g. Travel laptop",
                "Название нового под-аккаунта, например «Ноутбук в дороге»",
            ))
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &host_url,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.send(OwnerRequest::Load, window, cx),
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&create_label, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.create(window, cx);
                }
            }),
            cx.observe_in(&relay, window, |this, relay, window, cx| {
                let host = relay.read(cx).host_url().to_string();
                if this.host_url.read(cx).value() != host.as_str() {
                    this.host_url
                        .update(cx, |input, cx| input.set_value(host, window, cx));
                }
            }),
        ];
        Self {
            auth_store,
            projects: Vec::new(),
            host_url,
            create_label,
            subaccounts: Vec::new(),
            assignments: BTreeMap::new(),
            error: None,
            request: None,
            loaded: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_projects(&mut self, projects: Vec<DashboardProject>, cx: &mut Context<Self>) {
        self.projects = projects;
        cx.notify();
    }

    pub fn cancel_request(&mut self, cx: &mut Context<Self>) {
        self.request = None;
        cx.notify();
    }

    /// Loads the owner dashboard the first time the section is shown (when a host is known).
    pub fn ensure_loaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.loaded
            && self.request.is_none()
            && !self.host_url.read(cx).value().trim().is_empty()
        {
            self.send(OwnerRequest::Load, window, cx);
        }
    }

    fn pending(&self) -> bool {
        self.request.is_some()
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let label = self.create_label.read(cx).value().trim().to_string();
        if label.is_empty() {
            self.error = Some((
                localized(
                    cx,
                    "Enter a name for the sub-account.",
                    "Введите название под-аккаунта.",
                )
                .into(),
                None,
            ));
            cx.notify();
            return;
        }
        self.send(OwnerRequest::Create(label), window, cx);
    }

    fn send(&mut self, request: OwnerRequest, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending() {
            return;
        }
        let entered = self.host_url.read(cx).value().to_string();
        let host = match ShellModels::relay(cx)
            .update(cx, |relay, cx| relay.remember_host_url(&entered, cx))
        {
            Ok(host) => host,
            Err(error) => {
                self.error = Some((error.into(), None));
                cx.notify();
                return;
            }
        };
        self.error = None;
        let creating = matches!(request, OwnerRequest::Create(_));
        let auth_store = self.auth_store.clone();
        let future = async move {
            match request {
                OwnerRequest::Load => owner::load_dashboard(&auth_store, &host).await,
                OwnerRequest::Create(label) => {
                    owner::create_subaccount(&auth_store, &host, &label).await
                }
                OwnerRequest::Revoke(id) => owner::revoke_subaccount(&auth_store, &host, &id).await,
                OwnerRequest::Assign(id, projects) => {
                    owner::assign_projects(&auth_store, &host, &id, &projects).await
                }
            }
        };
        self.request = Some(spawn_service_in(
            window,
            cx,
            future,
            move |this, result, window, cx| {
                this.request = None;
                match result {
                    Ok(dashboard) => {
                        this.complete(dashboard, cx);
                        if creating {
                            this.create_label
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                    }
                    Err(error) => {
                        this.error = Some((
                            localized(
                                cx,
                                "Couldn't update sub-accounts",
                                "Не удалось обновить под-аккаунты",
                            )
                            .into(),
                            Some(format!("{error:#}").into()),
                        ))
                    }
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn complete(&mut self, dashboard: OwnerDashboard, cx: &mut Context<Self>) {
        self.loaded = true;
        self.subaccounts = dashboard.subaccounts;
        self.assignments = self
            .subaccounts
            .iter()
            .map(|sub| (sub.id.clone(), sub.allowed_project_ids.clone()))
            .collect();
        cx.notify();
    }

    fn toggle_project(&mut self, subaccount_id: &str, project_id: &str, selected: bool) {
        if self.pending() {
            return;
        }
        let known: Vec<&str> = self.projects.iter().map(|p| p.id.as_str()).collect();
        let current = self
            .assignments
            .get(subaccount_id)
            .cloned()
            .unwrap_or_default();
        let ids = toggle_assignment(&known, current, project_id, selected);
        self.assignments.insert(subaccount_id.to_string(), ids);
        self.error = None;
    }

    fn render_subaccount(&self, sub: &ManagedSubaccount, cx: &mut Context<Self>) -> Div {
        let pending = self.pending();
        let theme = cx.theme();
        let assigned = self.assignments.get(&sub.id).cloned().unwrap_or_default();
        let all_projects = assigned.is_empty();
        let unknown = assigned
            .iter()
            .filter(|id| !self.projects.iter().any(|p| &p.id == *id))
            .count();
        let code = match sub.code.as_deref() {
            Some(code) => localized_fmt(
                cx,
                "Login code {code}",
                "Код входа {code}",
                &[("code", code)],
            ),
            None => localized(
                cx,
                "Login code hidden by the relay",
                "Ретранслятор скрывает код входа",
            )
            .to_string(),
        };
        is_russian(cx);
        let stats = format!(
            "{} · {} · {}",
            count(
                sub.completed_tasks,
                ("task", "tasks"),
                ("задача", "задачи", "задач")
            ),
            format_worked_time(sub.worked_seconds, cx),
            format_money_cents(sub.earnings_cents)
        );

        let mut access = v_flex().gap_1p5();
        if self.projects.is_empty() {
            access = access.child(
                h_flex()
                    .gap_2()
                    .child(ui::muted(
                        localized(
                            cx,
                            "Load projects to choose access by name.",
                            "Загрузите проекты, чтобы выбрать доступ по названию.",
                        ),
                        cx,
                    ))
                    .child(
                        Button::new(SharedString::from(format!("load-projects-{}", sub.id)))
                            .ghost()
                            .xsmall()
                            .label(localized(cx, "Load projects", "Загрузить проекты"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(LoadProjects))),
                    ),
            );
        } else {
            for project in &self.projects {
                let checked = all_projects || assigned.contains(&project.id);
                let (sub_id, project_id) = (sub.id.clone(), project.id.clone());
                access = access.child(
                    Checkbox::new(SharedString::from(format!(
                        "access-{}-{}",
                        sub.id, project.id
                    )))
                    .label(project.name.clone())
                    .checked(checked)
                    .disabled(pending)
                    .on_click(cx.listener(
                        move |this, checked: &bool, _, cx| {
                            this.toggle_project(&sub_id, &project_id, *checked);
                            cx.notify();
                        },
                    )),
                );
            }
        }
        if unknown > 0 {
            is_russian(cx);
            access = access.child(ui::muted(
                localized_fmt(
                    cx,
                    "{n} not in the current project list. Refresh projects to review.",
                    "{n} нет в текущем списке проектов. Обновите проекты, чтобы проверить.",
                    &[(
                        "n",
                        &count(
                            unknown as u64,
                            ("saved assignment is", "saved assignments are"),
                            (
                                "сохранённого назначения",
                                "сохранённых назначений",
                                "сохранённых назначений",
                            ),
                        ),
                    )],
                ),
                cx,
            ));
        }

        let revoke_id = sub.id.clone();
        let save_id = sub.id.clone();
        let allow_id = sub.id.clone();
        v_flex()
            .gap_3()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(sub.label.clone()),
                                    )
                                    .child(if sub.revoked {
                                        Tag::danger().small().child(localized(
                                            cx,
                                            "Revoked",
                                            "Отозван",
                                        ))
                                    } else {
                                        Tag::success().small().child(localized(
                                            cx,
                                            "Active",
                                            "Активен",
                                        ))
                                    }),
                            )
                            .child(ui::muted(code, cx))
                            .child(ui::muted(stats, cx)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("revoke-{}", sub.id)))
                            .danger()
                            .small()
                            .label(localized(cx, "Revoke", "Отозвать"))
                            .disabled(pending || sub.revoked)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.send(OwnerRequest::Revoke(revoke_id.clone()), window, cx)
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .justify_between()
                            .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(
                                if all_projects {
                                    localized(
                                        cx,
                                        "Project access · every project",
                                        "Доступ к проектам · все проекты",
                                    )
                                } else {
                                    localized(
                                        cx,
                                        "Project access · selected projects",
                                        "Доступ к проектам · выбранные проекты",
                                    )
                                },
                            ))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "allow-all-{}",
                                            sub.id
                                        )))
                                        .ghost()
                                        .xsmall()
                                        .label(localized(
                                            cx,
                                            "Allow every project",
                                            "Разрешить все проекты",
                                        ))
                                        .disabled(pending || all_projects)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.assignments
                                                    .insert(allow_id.clone(), Vec::new());
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "save-access-{}",
                                            sub.id
                                        )))
                                        .outline()
                                        .xsmall()
                                        .label(localized(cx, "Save access", "Сохранить доступ"))
                                        .disabled(pending)
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                let projects = this
                                                    .assignments
                                                    .get(&save_id)
                                                    .cloned()
                                                    .unwrap_or_default();
                                                this.send(
                                                    OwnerRequest::Assign(save_id.clone(), projects),
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        ),
                                    ),
                            ),
                    )
                    .child(access),
            )
    }
}

impl Render for SubaccountsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.pending();
        let controls = ui::card(
            localized(cx, "Relay", "Ретранслятор"),
            Some(localized(
                cx,
                "Sub-accounts sign in through your relay. Load it to manage logins and project access.",
                "Под-аккаунты входят через ваш ретранслятор. Загрузите его, чтобы управлять входами и доступом к проектам.",
            )),
            cx,
        )
        .child(
            h_flex()
                .gap_2()
                .child(div().flex_1().child(Input::new(&self.host_url)))
                .child(
                    Button::new("owner-refresh")
                        .outline()
                        .icon(IconName::RefreshCw)
                        .label(localized(cx, "Load", "Загрузить"))
                        .loading(pending)
                        .disabled(pending)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.send(OwnerRequest::Load, window, cx)
                        })),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .child(div().flex_1().child(Input::new(&self.create_label)))
                .child(
                    Button::new("owner-create")
                        .primary()
                        .icon(IconName::Plus)
                        .label(localized(cx, "Create", "Создать"))
                        .disabled(pending)
                        .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
                ),
        )
        .when_some(self.error.clone(), |this, (headline, technical)| {
            this.child(ui::error_block(headline, technical, cx))
        });

        let list = if self.subaccounts.is_empty() {
            v_flex().child(ui::muted(
                if self.loaded {
                    localized(cx, "No sub-accounts yet.", "Под-аккаунтов пока нет.")
                } else {
                    localized(
                        cx,
                        "Load the relay to see your sub-accounts.",
                        "Загрузите ретранслятор, чтобы увидеть под-аккаунты.",
                    )
                },
                cx,
            ))
        } else {
            let subaccounts = self.subaccounts.clone();
            v_flex().gap_3().children(
                subaccounts
                    .iter()
                    .map(|sub| self.render_subaccount(sub, cx))
                    .collect::<Vec<_>>(),
            )
        };

        v_flex().w_full().gap_4().child(controls).child(list)
    }
}

/// Project access after (un)checking `project_id`. An empty list means "every project":
/// unchecking one project expands it to the explicit remainder, and selecting every known
/// project collapses back to empty. Order follows the dashboard project list.
fn toggle_assignment(
    known: &[&str],
    mut ids: Vec<String>,
    project_id: &str,
    selected: bool,
) -> Vec<String> {
    if ids.is_empty() && !selected {
        ids = known.iter().map(|id| id.to_string()).collect();
    }
    if selected {
        if !ids.iter().any(|id| id == project_id) {
            ids.push(project_id.to_string());
        }
    } else {
        ids.retain(|id| id != project_id);
    }
    let mut seen = BTreeSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    ids.sort_by_key(|id| {
        known
            .iter()
            .position(|known| known == id)
            .unwrap_or(usize::MAX)
    });
    if !known.is_empty()
        && ids.len() == known.len()
        && ids.iter().all(|id| known.contains(&id.as_str()))
    {
        ids.clear();
    }
    ids
}

fn format_worked_time(seconds: u64, cx: &App) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        localized_fmt(
            cx,
            "{hours}h {minutes}m",
            "{hours} ч {minutes} мин",
            &[
                ("hours", &hours.to_string()),
                ("minutes", &minutes.to_string()),
            ],
        )
    } else {
        localized_fmt(
            cx,
            "{minutes}m",
            "{minutes} мин",
            &[("minutes", &minutes.to_string())],
        )
    }
}

fn format_money_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let absolute = cents.unsigned_abs();
    format!("{sign}${}.{:02}", absolute / 100, absolute % 100)
}

#[cfg(test)]
mod tests {
    use super::toggle_assignment;

    const KNOWN: [&str; 3] = ["a", "b", "c"];

    #[test]
    fn unchecking_from_every_project_keeps_the_others_in_list_order() {
        assert_eq!(
            toggle_assignment(&KNOWN, Vec::new(), "b", false),
            ["a", "c"]
        );
    }

    #[test]
    fn checking_the_last_missing_project_collapses_to_every_project() {
        let ids = vec!["c".to_string(), "a".to_string()];
        assert!(toggle_assignment(&KNOWN, ids, "b", true).is_empty());
    }

    #[test]
    fn checking_keeps_unknown_saved_assignments_after_known_ones() {
        let ids = vec!["gone".to_string(), "c".to_string()];
        assert_eq!(
            toggle_assignment(&KNOWN, ids, "a", true),
            ["a", "c", "gone"]
        );
    }
}
