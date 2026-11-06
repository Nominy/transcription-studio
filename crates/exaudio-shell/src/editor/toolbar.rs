//! Core playback, viewport and document controls.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderScale, SliderState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Sizable as _, WindowExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::edit::EditorAction;
use studio_editor::edit::format::{
    SPEED_OPTIONS, ZOOM_SLIDER_MAX, ZOOM_SLIDER_MIN, format_speed, slider_from_zoom_factor,
    zoom_factor_from_slider,
};
use studio_editor::editor_model::SaveStatus;

use crate::editor::RequestLeave;
use crate::editor::model::EditorModel;
use crate::editor::panels::ui::{stable_text, tabular_figures};
use crate::editor::timeline::overview::OverviewView;
use crate::editor::timeline::shortcut_hint;
use crate::screens::ui::localized;

/// Zoom factor change per zoom-button press.
const ZOOM_STEP: f32 = 1.5;
/// Below this toolbar width the back button drops its label.
const COMPACT_WIDTH: Rems = rems(60.);
/// Below this width the zoom slider and the speed step buttons are hidden too.
const NARROW_WIDTH: Rems = rems(46.);

pub struct Toolbar {
    model: Entity<EditorModel>,
    overview: Entity<OverviewView>,
    zoom: Entity<SliderState>,
    /// Zoom factor last written into the slider (detects zoom changes from wheel/keys).
    synced_zoom: f32,
    /// Toolbar width as last laid out (zero before the first layout).
    width: Rc<Cell<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RequestLeave> for Toolbar {}

impl Toolbar {
    pub const HEIGHT: Rems = rems(2.75);

    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let overview = cx.new(|cx| OverviewView::new(model.clone(), window, cx));
        let zoom_factor = model.read(cx).doc().session().zoom;
        let zoom = cx.new(|_| {
            SliderState::new()
                .min(ZOOM_SLIDER_MIN)
                .max(ZOOM_SLIDER_MAX)
                .scale(SliderScale::Logarithmic)
                .step(1.0)
                .default_value(slider_from_zoom_factor(zoom_factor))
        });
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&zoom, |this, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    let factor = zoom_factor_from_slider(value.end());
                    this.synced_zoom = factor;
                    this.model.update(cx, |model, cx| {
                        model.update_doc(cx, |doc| doc.set_zoom(factor));
                    });
                }
            }),
        ];
        Self {
            model,
            overview,
            zoom,
            synced_zoom: zoom_factor,
            width: Rc::default(),
            _subscriptions: subscriptions,
        }
    }

    fn step_zoom(&mut self, factor: f32, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                let zoom = doc.session().zoom * factor;
                doc.set_zoom(zoom);
            });
        });
    }

    fn copy_project_id(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.model.read(cx).doc().project().id.clone();
        cx.write_to_clipboard(ClipboardItem::new_string(id));
        window.push_notification(
            localized(cx, "Project ID copied.", "ID проекта скопирован."),
            cx,
        );
    }

    /// A native document that has never been saved: no folder to write to, no provider that
    /// saves it elsewhere.
    fn has_no_save_location(&self, cx: &App) -> bool {
        let origin = self.model.read(cx).origin();
        origin.native_path.is_none()
            && origin.instance.is_none()
            && cx
                .global::<crate::ContributionRegistry>()
                .save_handler(&origin.mode_id)
                .is_none()
    }

    fn state_badge(&self, cx: &App) -> AnyElement {
        let doc = self.model.read(cx).doc();
        if doc.is_read_only() {
            return Tag::secondary()
                .small()
                .child(h_flex().gap_1().child(IconName::Lock).child(localized(
                    cx,
                    "Read only",
                    "Только чтение",
                )))
                .into_any_element();
        }
        let labels = [
            localized(cx, "Saving…", "Сохранение…"),
            localized(cx, "Not saved", "Не сохранено"),
            localized(cx, "Unsaved changes", "Есть изменения"),
            localized(cx, "Saved", "Сохранено"),
            localized(cx, "Unsaved", "Не сохранён"),
        ];
        let (tag, label) = match doc.session().save_status() {
            SaveStatus::Saving => (Tag::info(), labels[0]),
            SaveStatus::Failed { .. } => (Tag::danger(), labels[1]),
            SaveStatus::Dirty | SaveStatus::Saved if self.has_no_save_location(cx) => {
                (Tag::warning(), labels[4])
            }
            SaveStatus::Dirty => (Tag::warning(), labels[2]),
            SaveStatus::Saved => (Tag::success(), labels[3]),
        };
        // As wide as the widest state, so saving never moves the overview beside it.
        tag.outline()
            .small()
            .child(stable_text(label, labels))
            .into_any_element()
    }
}

fn separator(cx: &App) -> Div {
    div()
        .w(px(1.))
        .h(rems(1.25))
        .flex_none()
        .bg(cx.theme().border)
}

impl Render for Toolbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (is_playing, speed, zoom_factor) = {
            let session = self.model.read(cx).doc().session();
            (session.is_playing, session.speed, session.zoom)
        };
        if (zoom_factor - self.synced_zoom).abs() > f32::EPSILON {
            self.synced_zoom = zoom_factor;
            self.zoom.update(cx, |slider, cx| {
                slider.set_value(slider_from_zoom_factor(zoom_factor), window, cx);
            });
        }
        // Container query: the toolbar sits in panes of varying width, so it adapts to its own
        // laid-out width (in rems, i.e. independent of the interface zoom).
        let width = self.width.get();
        let measured = width > px(0.);
        let compact = measured && width < COMPACT_WIDTH.to_pixels(window.rem_size());
        let narrow = measured && width < NARROW_WIDTH.to_pixels(window.rem_size());

        let back = Button::new("toolbar-back")
            .ghost()
            .small()
            .icon(IconName::ArrowLeft)
            .when(!compact, |button| {
                button.label(localized(cx, "Start", "Начало"))
            })
            .tooltip(shortcut_hint(
                cx,
                localized(cx, "Back to Start", "На начальный экран"),
                EditorAction::BackToProjects,
            ))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(RequestLeave)));

        let play = Button::new("toolbar-play")
            .primary()
            .small()
            .icon(if is_playing {
                IconName::Pause
            } else {
                IconName::Play
            })
            .tooltip(shortcut_hint(
                cx,
                if is_playing {
                    localized(cx, "Pause", "Пауза")
                } else {
                    localized(cx, "Play", "Воспроизвести")
                },
                EditorAction::PlayPause,
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                this.model
                    .update(cx, |model, cx| model.run(cx, |doc| doc.toggle_playback()));
            }));

        // The label keeps the width of the widest speed (tabular figures), so changing the
        // speed never moves the rest of the toolbar.
        let speed_menu = Button::new("toolbar-speed")
            .outline()
            .small()
            .child(
                stable_text(
                    format_speed(speed),
                    SPEED_OPTIONS.iter().map(|option| format_speed(*option)),
                )
                .font_features(tabular_figures()),
            )
            .accessibility_label(format_speed(speed))
            .tooltip(localized(cx, "Speed", "Скорость"))
            .dropdown_menu({
                let model = self.model.clone();
                move |menu, _, _| {
                    SPEED_OPTIONS.iter().fold(menu, |menu, option| {
                        let option = *option;
                        let model = model.clone();
                        menu.item(
                            PopupMenuItem::new(format_speed(option))
                                .checked((option - speed).abs() < f32::EPSILON)
                                .on_click(move |_, _, cx| {
                                    model.update(cx, |model, cx| {
                                        model.run(cx, |doc| doc.set_speed(option));
                                    });
                                }),
                        )
                    })
                }
            });
        let slower = Button::new("toolbar-slower")
            .ghost()
            .small()
            .icon(IconName::Minus)
            .tooltip(shortcut_hint(
                cx,
                localized(cx, "Slower", "Медленнее"),
                EditorAction::DecreasePlaybackSpeed,
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                this.model
                    .update(cx, |model, cx| model.run(cx, |doc| doc.step_speed(-1)));
            }));
        let faster = Button::new("toolbar-faster")
            .ghost()
            .small()
            .icon(IconName::Plus)
            .tooltip(shortcut_hint(
                cx,
                localized(cx, "Faster", "Быстрее"),
                EditorAction::IncreasePlaybackSpeed,
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                this.model
                    .update(cx, |model, cx| model.run(cx, |doc| doc.step_speed(1)));
            }));

        let zoom_out = Button::new("toolbar-zoom-out")
            .ghost()
            .small()
            .icon(IconName::ZoomOut)
            .tooltip(format!(
                "{} (Ctrl+wheel)",
                localized(cx, "Zoom out", "Уменьшить")
            ))
            .on_click(cx.listener(|this, _, _, cx| this.step_zoom(1.0 / ZOOM_STEP, cx)));
        let zoom_in = Button::new("toolbar-zoom-in")
            .ghost()
            .small()
            .icon(IconName::ZoomIn)
            .tooltip(format!(
                "{} (Ctrl+wheel)",
                localized(cx, "Zoom in", "Увеличить")
            ))
            .on_click(cx.listener(|this, _, _, cx| this.step_zoom(ZOOM_STEP, cx)));

        let commands: Vec<_> = cx
            .global::<crate::ContributionRegistry>()
            .commands()
            .into_iter()
            .filter(|command| command.menu)
            .collect();
        let toolbar = cx.entity().downgrade();
        let menu = Button::new("extension-commands")
            .small()
            .label("⋯")
            .tooltip(localized(cx, "More", "Ещё"))
            .dropdown_menu(move |mut menu, _, cx| {
                menu = menu
                    .item(
                        PopupMenuItem::new(localized(
                            cx,
                            "Import transcript…",
                            "Импортировать текст…",
                        ))
                        .on_click(|_, window, cx| {
                            let shell = cx.global::<crate::ShellHandle>().clone();
                            crate::screens::formats::import_document(shell, window, cx);
                        }),
                    )
                    .item(
                        PopupMenuItem::new(localized(cx, "Start", "Начало")).on_click(
                            |_, window, cx| {
                                cx.global::<crate::ShellHandle>()
                                    .clone()
                                    .show_home(window, cx);
                            },
                        ),
                    )
                    .item(
                        PopupMenuItem::new(localized(cx, "Settings…", "Настройки…")).on_click(
                            |_, window, cx| {
                                cx.global::<crate::ShellHandle>()
                                    .clone()
                                    .show_settings(window, cx);
                            },
                        ),
                    )
                    .item(
                        PopupMenuItem::new(localized(cx, "Extensions…", "Расширения…")).on_click(
                            |_, window, cx| {
                                cx.global::<crate::ShellHandle>()
                                    .clone()
                                    .show_extensions(window, cx);
                            },
                        ),
                    )
                    .item(
                        PopupMenuItem::new(localized(
                            cx,
                            "Keyboard shortcuts…",
                            "Сочетания клавиш…",
                        ))
                        .on_click(|_, window, cx| {
                            crate::editor::panels::dialogs::hotkeys::open(window, cx)
                        }),
                    )
                    .item(
                        PopupMenuItem::new(localized(
                            cx,
                            "Copy project ID",
                            "Скопировать ID проекта",
                        ))
                        .on_click({
                            let toolbar = toolbar.clone();
                            move |_, window, cx| {
                                toolbar
                                    .update(cx, |this, cx| this.copy_project_id(window, cx))
                                    .ok();
                            }
                        }),
                    );
                for command in &commands {
                    let id = command.id.clone();
                    menu = menu.item(PopupMenuItem::new(command.label.clone()).on_click(
                        move |_, window, cx| {
                            let shell = cx.global::<crate::ShellHandle>().clone();
                            shell.dispatch(&id, window, cx);
                        },
                    ));
                }
                menu
            });
        let export = Button::new("document-export")
            .small()
            .label(localized(cx, "Export…", "Экспорт…"))
            .on_click(|_, window, cx| {
                let shell = cx.global::<crate::ShellHandle>().clone();
                crate::screens::formats::export_document(shell, window, cx);
            });

        let badge = self.state_badge(cx);
        let theme = cx.theme();
        let measure = {
            let width = Rc::clone(&self.width);
            canvas(
                move |bounds, window, _| {
                    if (width.get() - bounds.size.width).abs() > px(0.5) {
                        width.set(bounds.size.width);
                        window.request_animation_frame();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
        };
        h_flex()
            .relative()
            .h(Self::HEIGHT)
            .flex_none()
            .w_full()
            .px_2()
            .gap_2()
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius)
            .bg(theme.background)
            .child(measure)
            .child(div().flex_none().child(back))
            .child(separator(cx))
            .child(div().flex_none().child(play))
            .child(
                h_flex()
                    .flex_none()
                    .gap_0p5()
                    .when(!narrow, |group| group.child(slower))
                    .child(speed_menu)
                    .when(!narrow, |group| group.child(faster)),
            )
            .child(separator(cx))
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(zoom_out)
                    .when(!narrow, |group| {
                        group.child(
                            div()
                                .w(rems(7.5))
                                .child(Slider::new(&self.zoom).horizontal()),
                        )
                    })
                    .child(zoom_in),
            )
            .child(separator(cx))
            .child(
                div()
                    .flex_1()
                    .min_w(rems(6.))
                    .h(rems(1.875))
                    .child(self.overview.clone()),
            )
            .child(separator(cx))
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .child(badge)
                    .child(export)
                    .child(menu),
            )
    }
}
