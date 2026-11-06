//! Transcript grid: virtualized rows bound to the [`EditorDoc`] drafts, a pooled text editor
//! for the focused row, time-cell editing, tag suggestions, lint cells and row menus. A grid
//! shows every speaker or, in the split layouts, one audio lane; the [`FindBar`] above the
//! grids serves them all.
//!
//! Only one row is edited at a time, so a single multi-line input is mounted in the focused
//! row and every other row paints its draft as static text laid out exactly like the input
//! (same font, size, padding, wrap width and line breaks — see [`wrap`]). Row heights for
//! `v_virtual_list` come from the same wrap (see [`layout`]).
//!
//! Keys pressed while the row editor has focus are intercepted before the input's own key
//! bindings so keymap actions (row undo/redo, escape, tag list keys, text moves, global
//! shortcuts) reach the doc instead of the input.
//!
//! [`EditorDoc`]: studio_editor::edit::EditorDoc

mod find_bar;
pub use find_bar::FindBar;
mod layout;
mod row;
mod wrap;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use exaudio_extension::document::Completion;
use gpui_kit::component::input::{InputEvent, InputState, RopeExt as _, TextareaState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme, VirtualListScrollHandle, WindowExt as _, h_flex, v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::edit::keymap::{FocusTarget, KeyFocus};
use studio_editor::edit::time::TimeField;
use studio_editor::edit::{EditorDoc, Outcome, TextSelection};
use studio_editor::timeline_model::LaneColor;
use studio_editor::transcript_model::{TranscriptActionCommand, TranscriptRowView};

use crate::editor::model::{EditorEvent, EditorModel};
use crate::keymap::{ActiveKeymap, keystroke_string};
use crate::screens::ui::localized;
use layout::{
    ACTIONS_W, CELL_PX, ID_W, LINT_W, REVIEWER_W, RowMetrics, TEXT_PX, TIME_W, TRACK_W, TextMetrics,
};
use row::RowParams;

const MAX_EDITOR_ROWS: usize = 400;

pub struct TranscriptView {
    model: Entity<EditorModel>,
    /// Lane this grid is limited to (split layouts); `None` shows every lane.
    lane: Option<LaneRows>,
    focus_handle: FocusHandle,
    window_handle: AnyWindowHandle,
    scroll_handle: VirtualListScrollHandle,

    /// Pooled multi-line editor, mounted in `text_row`.
    text_input: Entity<TextareaState>,
    text_row: Option<SharedString>,
    text_focused: bool,
    /// Draft text last loaded into, or taken from, the editor.
    synced_text: String,

    /// Pooled time-cell editor, mounted in `time_cell`.
    time_input: Entity<InputState>,
    time_cell: Option<(SharedString, TimeField)>,

    /// Row whose speaker menu is open.
    speaker_menu: Option<SharedString>,
    /// Pooled speaker-name editor, mounted in the speaker cell of `speaker_rename.0` and
    /// renaming speaker `speaker_rename.1`.
    speaker_input: Entity<InputState>,
    speaker_rename: Option<(SharedString, String)>,

    /// Tag list scroll; follows the highlighted item when it changes.
    tag_scroll: ScrollHandle,
    tag_highlight: Cell<Option<usize>>,

    metrics: RowMetrics,
    /// Width of the text column from the last layout (measured by the header).
    text_column_width: Rc<Cell<Pixels>>,
    frame: Frame,
    render_key: RenderKey,
    /// Ghost cursor line (row + its centre in content coordinates) the grid last centred on;
    /// it re-centres only when the ghost moves to another line, so manual scrolling sticks.
    ghost_line: Option<(String, Pixels)>,
    _subscriptions: Vec<Subscription>,
}

/// Rows of one lane, refiltered only when the doc's row list changes.
struct LaneRows {
    lane_id: String,
    /// The doc rows last filtered.
    source: Rc<[TranscriptRowView]>,
    /// Rows of `lane_id` in `source`.
    rows: Rc<[TranscriptRowView]>,
}

impl LaneRows {
    fn new(lane_id: String) -> Self {
        Self {
            lane_id,
            source: Rc::default(),
            rows: Rc::default(),
        }
    }

    fn filter(&mut self, rows: Rc<[TranscriptRowView]>) -> Rc<[TranscriptRowView]> {
        if !Rc::ptr_eq(&self.source, &rows) {
            self.rows = rows
                .iter()
                .filter(|row| row.lane_id == self.lane_id)
                .cloned()
                .collect();
            self.source = rows;
        }
        Rc::clone(&self.rows)
    }
}

/// Rows and sizes of the current render, read by the virtual list callback.
#[derive(Default)]
struct Frame {
    rows: Rc<[TranscriptRowView]>,
    sizes: Rc<Vec<Size<Pixels>>>,
    find_matches: HashSet<String>,
    lane_colors: HashMap<String, Hsla>,
}

/// Doc state the grid renders from; the view re-renders only when it changes (the model
/// notifies every frame while playing).
#[derive(Default, PartialEq)]
struct RenderKey {
    generation: u64,
    selected: Option<String>,
    is_playing: bool,
    ghost: Option<(String, usize)>,
    /// The find query while the find bar is open (matches are highlighted).
    find_query: Option<String>,
}

impl RenderKey {
    fn of(doc: &EditorDoc) -> Self {
        let session = doc.session();
        let ghost = doc.helper().pending.as_ref().and_then(|pending| {
            doc.ghost_cursor(&pending.segment_id)
                .map(|ghost| (ghost.segment_id, ghost.char_offset))
        });
        Self {
            generation: doc.generation(),
            selected: session.selected_segment_id.clone(),
            is_playing: session.is_playing,
            ghost,
            find_query: doc
                .find_state()
                .open
                .then(|| doc.find_state().query.clone()),
        }
    }
}

impl TranscriptView {
    /// A grid of every lane (`lane: None`) or of one lane.
    pub fn new(
        model: Entity<EditorModel>,
        lane: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, MAX_EDITOR_ROWS)
                .soft_wrap(true)
        });
        let time_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(localized(cx, "mm:ss.cc", "мм:сс.сс"))
        });
        let speaker_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(localized(cx, "Speaker name", "Имя спикера"))
        });

        let view = cx.entity().downgrade();
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| {
                this.on_model_changed(window, cx);
            }),
            cx.subscribe_in(&model, window, Self::on_editor_event),
            cx.subscribe_in(&text_input, window, Self::on_text_input_event),
            cx.observe(&text_input, |this, _, cx| this.sync_text_selection(cx)),
            cx.subscribe_in(&time_input, window, Self::on_time_input_event),
            cx.subscribe_in(&speaker_input, window, Self::on_speaker_input_event),
            cx.intercept_keystrokes(move |event, window, cx| {
                view.update(cx, |this, cx| this.intercept_keystroke(event, window, cx))
                    .ok();
            }),
        ];

        let render_key = RenderKey::of(model.read(cx).doc());
        Self {
            model,
            lane: lane.map(LaneRows::new),
            focus_handle: cx.focus_handle(),
            window_handle: window.window_handle(),
            scroll_handle: VirtualListScrollHandle::new(),
            text_input,
            text_row: None,
            text_focused: false,
            synced_text: String::new(),
            time_input,
            time_cell: None,
            speaker_menu: None,
            speaker_input,
            speaker_rename: None,
            tag_scroll: ScrollHandle::new(),
            tag_highlight: Cell::new(None),
            metrics: RowMetrics::default(),
            text_column_width: Rc::new(Cell::new(px(0.))),
            frame: Frame::default(),
            render_key,
            ghost_line: None,
            _subscriptions: subscriptions,
        }
    }

    /// Lane this grid is limited to; `None` for the grid of every lane.
    pub fn lane_id(&self) -> Option<&str> {
        self.lane.as_ref().map(|lane| lane.lane_id.as_str())
    }

    /// Segment whose text editor currently has keyboard focus.
    pub fn focused_segment_id(&self, window: &Window, cx: &App) -> Option<String> {
        self.text_row
            .as_ref()
            .filter(|_| self.text_focused && self.text_input.focus_handle(cx).is_focused(window))
            .map(ToString::to_string)
    }

    /// Rows of this grid: the doc's rows under the track filter, limited to the lane.
    fn rows(&mut self, doc: &EditorDoc) -> Rc<[TranscriptRowView]> {
        let rows = doc.transcript_rows();
        match &mut self.lane {
            Some(lane) => lane.filter(rows),
            None => rows,
        }
    }

    /// Whether `segment_id` belongs in this grid.
    fn owns(&self, doc: &EditorDoc, segment_id: &str) -> bool {
        self.lane.as_ref().is_none_or(|lane| {
            doc.project()
                .segment(segment_id)
                .is_some_and(|segment| segment.lane_id == lane.lane_id)
        })
    }

    // ---- model → view -------------------------------------------------------------------

    fn on_model_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = self.model.read(cx).doc();
        // A row that left this grid's lane (track change) unmounts its editor here.
        let text_draft = self.text_row.as_ref().map(|row| {
            doc.draft(row)
                .filter(|_| self.owns(doc, row))
                .map(|draft| (draft.text().to_string(), draft.selection()))
        });
        let time_row_gone = self
            .time_cell
            .as_ref()
            .is_some_and(|(row, _)| doc.draft(row).is_none());
        let speaker_row_gone = self
            .speaker_rename
            .as_ref()
            .is_some_and(|(row, _)| doc.draft(row).is_none());
        let speaker_menu_gone = self
            .speaker_menu
            .as_ref()
            .is_some_and(|row| doc.draft(row).is_none());

        match text_draft {
            Some(None) => self.unmount_text_editor(window, cx),
            Some(Some((text, selection))) if text != self.synced_text => {
                self.load_text_editor(text, selection, window, cx);
            }
            _ => {}
        }
        if time_row_gone {
            self.time_cell = None;
        }
        if speaker_row_gone {
            self.speaker_rename = None;
        }
        if speaker_menu_gone {
            self.speaker_menu = None;
        }

        self.refresh_render_key(cx);
    }

    /// Re-renders when the doc state the grid shows changed. Playback frames only emit
    /// [`EditorEvent::Frame`], yet move the selected row and the ghost cursor.
    fn refresh_render_key(&mut self, cx: &mut Context<Self>) {
        let key = RenderKey::of(self.model.read(cx).doc());
        if key != self.render_key {
            self.render_key = key;
            cx.notify();
        }
    }

    fn on_editor_event(
        &mut self,
        _: &Entity<EditorModel>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::RevealSegment { segment_id, center } => {
                self.reveal(segment_id, *center, cx);
            }
            EditorEvent::FocusText { segment_id, caret } => {
                let doc = self.model.read(cx).doc();
                if self.owns(doc, segment_id) {
                    // The caret lands where the ghost was: same line centring as playback, so
                    // pausing or Escape never moves the transcript.
                    if let Some(y) = self.caret_line_y(doc, segment_id, *caret, window) {
                        self.center_on(y);
                    }
                    self.ghost_line = None;
                    self.activate_text(segment_id, Some(TextSelection::caret(*caret)), window, cx);
                }
            }
            EditorEvent::SelectText {
                segment_id,
                selection,
            } => {
                if self.owns(self.model.read(cx).doc(), segment_id) {
                    self.activate_text(segment_id, Some(*selection), window, cx);
                    self.reveal(segment_id, false, cx);
                }
            }
            EditorEvent::BlurText => self.blur_text(window, cx),
            EditorEvent::DraftChanged { segment_id } => {
                let doc = self.model.read(cx).doc();
                if !self.owns(doc, segment_id) {
                    return;
                }
                if self.text_row.as_deref() == Some(segment_id.as_str())
                    && let Some(draft) = doc.draft(segment_id)
                {
                    let (text, selection) = (draft.text().to_string(), draft.selection());
                    self.load_text_editor(text, selection, window, cx);
                }
                cx.notify();
            }
            EditorEvent::Frame => self.refresh_render_key(cx),
            EditorEvent::ExecuteCommand { .. } => {}
        }
    }

    fn reveal(&mut self, segment_id: &str, center: bool, cx: &mut Context<Self>) {
        let doc = self.model.read(cx).doc();
        // A row showing the ghost cursor is scrolled by `follow_ghost` (line centring).
        if doc.ghost_cursor(segment_id).is_some() {
            return;
        }
        let rows = self.rows(doc);
        if let Some(index) = rows.iter().position(|row| row.segment_id == segment_id) {
            let strategy = if center {
                ScrollStrategy::Center
            } else {
                ScrollStrategy::Top
            };
            self.scroll_handle.scroll_to_item(index, strategy);
            cx.notify();
        }
    }

    /// Centre of the wrapped text line holding `char_offset` of `segment_id`, in the grid's
    /// content coordinates (from the rows and heights of the last render).
    fn caret_line_y(
        &mut self,
        doc: &EditorDoc,
        segment_id: &str,
        char_offset: usize,
        window: &Window,
    ) -> Option<Pixels> {
        let index = self
            .frame
            .rows
            .iter()
            .position(|row| row.segment_id == segment_id)?;
        let row_top: Pixels = self.frame.sizes[..index]
            .iter()
            .map(|size| size.height)
            .sum();
        let within = self.metrics.caret_line_center(
            &self.frame.rows[index],
            doc,
            char_offset,
            self.text_column_width.get(),
            window,
        );
        Some(row_top + within)
    }

    /// Scrolls so content position `y` sits in the middle of the viewport (clamped to the
    /// content), even inside a row taller than the viewport.
    fn center_on(&self, y: Pixels) {
        let handle = self.scroll_handle.base_handle();
        let viewport = handle.bounds().size.height;
        if viewport <= px(0.) {
            return;
        }
        let content: Pixels = self.frame.sizes.iter().map(|size| size.height).sum();
        let max = (content - viewport).max(px(0.));
        let target = (y - viewport / 2.).clamp(px(0.), max);
        handle.set_offset(point(handle.offset().x, -target));
    }

    /// Keeps the ghost cursor's line in the middle of the grid while it moves.
    fn follow_ghost(&mut self, doc: &EditorDoc, window: &Window) {
        let Some((segment_id, offset)) = self.render_key.ghost.clone() else {
            self.ghost_line = None;
            return;
        };
        let Some(y) = self.caret_line_y(doc, &segment_id, offset, window) else {
            return;
        };
        let line = Some((segment_id, y));
        if self.ghost_line != line {
            self.center_on(y);
            self.ghost_line = line;
        }
    }

    // ---- row text editor ----------------------------------------------------------------

    /// Mounts the pooled editor in `segment_id`, loads its draft and focuses it.
    fn activate_text(
        &mut self,
        segment_id: &str,
        selection: Option<TextSelection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let doc = self.model.read(cx).doc();
        if doc.is_read_only() {
            return;
        }
        let Some(draft) = doc.draft(segment_id) else {
            return;
        };
        let text = draft.text().to_string();
        let selection = selection.unwrap_or_else(|| draft.selection());
        let transcription = doc
            .transcript_rows()
            .iter()
            .find(|row| row.segment_id == segment_id)
            .is_none_or(|row| row.annotation_kind.is_transcription());

        self.text_row = Some(SharedString::from(segment_id.to_string()));
        self.text_focused = true;
        self.load_text_editor(text, selection, window, cx);
        self.text_input.update(cx, |input, cx| {
            input.set_placeholder(
                if transcription {
                    localized(cx, "What was said…", "Что было сказано…")
                } else {
                    localized(cx, "Annotation…", "Аннотация…")
                },
                window,
                cx,
            );
            input.focus(window, cx);
        });
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| {
                doc.set_selection(segment_id, selection);
                doc.set_text_focus(Some(segment_id))
            });
        });
        cx.notify();
    }

    /// Click into a row's static text: select the row and edit at the clicked byte.
    pub(super) fn activate_text_at_byte(
        &mut self,
        segment_id: &str,
        byte: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let caret = self
            .model
            .read(cx)
            .doc()
            .draft(segment_id)
            .map(|draft| {
                let text = draft.text();
                text[..byte.min(text.len())].chars().count()
            })
            .unwrap_or_default();
        self.model.update(cx, |model, cx| {
            model.run(cx, |doc| doc.press_row(segment_id, 1));
        });
        self.activate_text(segment_id, Some(TextSelection::caret(caret)), window, cx);
    }

    /// Alt+Click on a row's static text: seek to the clicked word through the word timings
    /// (nothing happens without timings).
    pub(super) fn seek_to_word_at_byte(&mut self, segment_id: &str, byte: usize, cx: &mut App) {
        let doc = self.model.read(cx).doc();
        let Some(char_offset) = doc.draft(segment_id).map(|draft| {
            let text = draft.text();
            text[..byte.min(text.len())].chars().count()
        }) else {
            return;
        };
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| doc.seek_to_word(segment_id, char_offset));
        });
    }

    /// Click into the padding/placeholder of a row's text cell: edit at the end.
    pub(super) fn activate_text_at_end(
        &mut self,
        segment_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let end = self
            .model
            .read(cx)
            .doc()
            .draft(segment_id)
            .map_or(0, |draft| draft.text().chars().count());
        self.model.update(cx, |model, cx| {
            model.run(cx, |doc| doc.press_row(segment_id, 1));
        });
        self.activate_text(segment_id, Some(TextSelection::caret(end)), window, cx);
    }

    fn load_text_editor(
        &mut self,
        text: String,
        selection: TextSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.synced_text.clone_from(&text);
        self.text_input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            let range = selection.range();
            let rope = input.text();
            let start = rope.char_index_to_offset(range.start);
            let end = rope.char_index_to_offset(range.end);
            input.set_selected_range(start..end, cx);
        });
    }

    fn unmount_text_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.blur_text(window, cx);
        self.text_row = None;
        self.text_focused = false;
        self.synced_text.clear();
        cx.notify();
    }

    /// Leaves the row editor and commits its typed text. The textarea's own `Blur` event is
    /// not relied on: focus moves initiated here do not always produce one.
    fn blur_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_input.focus_handle(cx).is_focused(window) {
            self.focus_handle.focus(window, cx);
        }
        self.text_focused = self.text_input.focus_handle(cx).is_focused(window);
        self.commit_text_row(cx);
    }

    /// Commits the mounted row's draft to the document (no-op when nothing changed).
    fn commit_text_row(&mut self, cx: &mut Context<Self>) {
        let Some(segment_id) = self.text_row.clone() else {
            return;
        };
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| doc.blur_text(&segment_id));
        });
        cx.notify();
    }

    fn on_text_input_event(
        &mut self,
        input: &Entity<TextareaState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(segment_id) = self.text_row.clone() else {
            return;
        };
        match event {
            InputEvent::Change => {
                // IME composition: sync once the text is committed.
                if input.update(cx, |input, cx| {
                    input.marked_text_range(window, cx).is_some()
                }) {
                    return;
                }
                let (text, selection) = {
                    let input = input.read(cx);
                    (input.value().to_string(), input_selection(input))
                };
                self.model.update(cx, |model, cx| {
                    model.edit(cx, |doc| {
                        doc.edit_text(&segment_id, text.clone(), selection)
                    });
                });
                let draft = self
                    .model
                    .read(cx)
                    .doc()
                    .draft(&segment_id)
                    .map(|draft| (draft.text().to_string(), draft.selection()));
                match draft {
                    Some((draft_text, _)) if draft_text == text => self.synced_text = text,
                    Some((draft_text, selection)) => {
                        self.load_text_editor(draft_text, selection, window, cx);
                    }
                    None => self.unmount_text_editor(window, cx),
                }
                cx.notify();
            }
            InputEvent::Focus => {
                self.text_focused = true;
                self.model.update(cx, |model, cx| {
                    model.edit(cx, |doc| doc.set_text_focus(Some(&segment_id)));
                });
                cx.notify();
            }
            InputEvent::Blur => {
                // The editor's row scrolled out of the list; focus is still on the editor.
                if input.focus_handle(cx).is_focused(window) {
                    return;
                }
                self.text_focused = false;
                self.commit_text_row(cx);
                self.text_row = None;
            }
            // Enter leaves the row; the blur that follows commits its text.
            InputEvent::PressEnter { .. } => self.blur_text(window, cx),
        }
    }

    /// Mirrors caret/selection moves of the editor into the draft (tags, SKAZ and text moves
    /// read it).
    fn sync_text_selection(&mut self, cx: &mut Context<Self>) {
        let Some(segment_id) = self.text_row.clone().filter(|_| self.text_focused) else {
            return;
        };
        let selection = input_selection(self.text_input.read(cx));
        let current = self
            .model
            .read(cx)
            .doc()
            .draft(&segment_id)
            .map(|draft| draft.selection());
        if current.is_some_and(|current| current != selection) {
            self.model.update(cx, |model, cx| {
                model.update_doc(cx, |doc| doc.set_selection(&segment_id, selection));
            });
            cx.notify();
        }
    }

    /// Keymap actions for the focused row editor run before the input's own key bindings.
    fn intercept_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.window_handle() != self.window_handle
            || window.has_active_dialog(cx)
            || window.has_active_sheet(cx)
        {
            return;
        }
        // Escape in a time or speaker cell cancels its draft, never an editor shortcut.
        if self.time_cell.is_some()
            && self.time_input.focus_handle(cx).is_focused(window)
            && keystroke_string(&event.keystroke) == "escape"
        {
            self.cancel_time_cell(window, cx);
            cx.stop_propagation();
            return;
        }
        if self.speaker_rename.is_some()
            && self.speaker_input.focus_handle(cx).is_focused(window)
            && keystroke_string(&event.keystroke) == "escape"
        {
            self.cancel_speaker_rename(window, cx);
            cx.stop_propagation();
            return;
        }
        if !self.text_focused || !self.text_input.focus_handle(cx).is_focused(window) {
            return;
        }
        let Some(segment_id) = self.text_row.clone() else {
            return;
        };
        self.sync_text_selection(cx);

        let keystroke = &event.keystroke;
        let (focus, has_selection) = {
            let doc = self.model.read(cx).doc();
            let focus = KeyFocus {
                target: FocusTarget::TranscriptText,
                cut_preview_active: doc.session().overlay.is_some(),
                completions_open: doc.completions(&segment_id).is_some(),
            };
            let has_selection = doc
                .draft(&segment_id)
                .is_some_and(|draft| !draft.selection().is_empty());
            (focus, has_selection)
        };

        if let Some(action) = ActiveKeymap::resolve(cx, keystroke, focus) {
            let handled = self
                .model
                .update(cx, |model, cx| model.perform(action, Some(&segment_id), cx));
            if handled {
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }

        let command = cx
            .global::<crate::ContributionRegistry>()
            .resolve_keybinding(&keystroke_string(keystroke), true, has_selection);
        if let Some(command) = command {
            let shell = cx.global::<crate::ShellHandle>().clone();
            shell.dispatch(&command, window, cx);
            cx.stop_propagation();
            return;
        }

        match keystroke_string(keystroke).as_str() {
            // Enter leaves the row (no line breaks in transcript text); the blur that follows
            // commits its text (`InputEvent::Blur`).
            "enter" | "shift-enter" => {
                self.blur_text(window, cx);
                cx.stop_propagation();
            }
            // Tab never inserts a tab character.
            "tab" | "shift-tab" => cx.stop_propagation(),
            _ => {}
        }
    }

    fn apply_completion(&mut self, suggestion: Completion, cx: &mut Context<Self>) {
        let Some(segment_id) = self.text_row.clone() else {
            return;
        };
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| doc.apply_completion(&segment_id, &suggestion));
        });
        cx.notify();
    }

    // ---- time cells ---------------------------------------------------------------------

    /// Press on a start/end cell. A single click selects the row like a click on its body (so
    /// Space then plays); a double click opens the cell for typing.
    pub(super) fn press_time_cell(
        &mut self,
        segment_id: &str,
        field: TimeField,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if click_count >= 2 {
            self.activate_time(segment_id, field, window, cx);
            return;
        }
        if self.time_input.focus_handle(cx).is_focused(window) {
            self.commit_time_cell(true, cx);
            self.focus_handle.focus(window, cx);
        }
        self.press_row(segment_id, 1, window, cx);
    }

    fn activate_time(
        &mut self,
        segment_id: &str,
        field: TimeField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_time_cell(true, cx);
        let Some(value) = self
            .model
            .read(cx)
            .doc()
            .draft(segment_id)
            .map(|draft| draft.time_value(field).to_string())
        else {
            return;
        };
        self.time_cell = Some((SharedString::from(segment_id.to_string()), field));
        self.time_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.select_all(window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Enter (`from_blur = false`) or leaving the cell (`true`).
    fn commit_time_cell(&mut self, from_blur: bool, cx: &mut Context<Self>) {
        let Some((segment_id, field)) = self.time_cell.clone() else {
            return;
        };
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| {
                doc.commit_time_edit(&segment_id, field, from_blur)
            });
        });
        if from_blur {
            self.time_cell = None;
        }
        cx.notify();
    }

    /// Escape: leaves the cell without applying the typed value.
    fn cancel_time_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((segment_id, field)) = self.time_cell.take() else {
            return;
        };
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| doc.cancel_time_edit(&segment_id, field));
        });
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn on_time_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((segment_id, field)) = self.time_cell.clone() else {
            return;
        };
        match event {
            InputEvent::Change => {
                let value = input.read(cx).value().to_string();
                self.model.update(cx, |model, cx| {
                    model.edit(cx, |doc| doc.set_time_draft(&segment_id, field, value));
                });
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                self.commit_time_cell(false, cx);
                let value = self
                    .model
                    .read(cx)
                    .doc()
                    .draft(&segment_id)
                    .map(|draft| draft.time_value(field).to_string());
                if let Some(value) = value
                    && value != input.read(cx).value().as_ref()
                {
                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }
            }
            InputEvent::Blur => {
                if !input.focus_handle(cx).is_focused(window) {
                    self.commit_time_cell(true, cx);
                }
            }
            InputEvent::Focus => {}
        }
    }

    // ---- row actions --------------------------------------------------------------------

    pub(super) fn press_row(
        &mut self,
        segment_id: &str,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.blur_text(window, cx);
        self.model.update(cx, |model, cx| {
            model.run(cx, |doc| doc.press_row(segment_id, click_count));
        });
    }

    // ---- speaker cell -------------------------------------------------------------------

    /// Click on a row's speaker cell (the press already selected the row): opens the menu of
    /// project speakers.
    pub(super) fn open_speaker_menu(
        &mut self,
        segment_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_speaker_rename(true, window, cx);
        self.speaker_menu = Some(SharedString::from(segment_id.to_string()));
        cx.notify();
    }

    pub(super) fn close_speaker_menu(&mut self, cx: &mut Context<Self>) {
        if self.speaker_menu.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn choose_speaker(
        &mut self,
        segment_id: &str,
        speaker_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.speaker_menu = None;
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| doc.set_segment_speaker(segment_id, speaker_id));
        });
        cx.notify();
    }

    /// `New speaker`: adds the next free `Speaker {n}`, assigns it to the row (one undo step)
    /// and opens its name for editing.
    pub(super) fn add_speaker(
        &mut self,
        segment_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.speaker_menu = None;
        let template = localized(cx, "Speaker {n}", "Спикер {n}");
        let added_status = localized(cx, "Added {speaker}.", "Добавлен {speaker}.");
        let added = self.model.update(cx, |model, cx| {
            let mut added = None;
            model.edit(cx, |doc| {
                let speaker_id = doc.add_speaker_for_segment(segment_id, template)?;
                let status = added_status.replace(
                    "{speaker}",
                    &doc.project()
                        .speaker(&speaker_id)
                        .map(|speaker| speaker.label.clone())
                        .unwrap_or_default(),
                );
                added = Some(speaker_id);
                Ok(Outcome::status(status))
            });
            added
        });
        match added {
            Some(speaker_id) => self.begin_speaker_rename(segment_id, &speaker_id, window, cx),
            None => cx.notify(),
        }
    }

    /// `Rename…`: mounts the name editor in the row's speaker cell.
    pub(super) fn begin_speaker_rename(
        &mut self,
        segment_id: &str,
        speaker_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.speaker_menu = None;
        self.commit_speaker_rename(true, window, cx);
        let Some(label) = self
            .model
            .read(cx)
            .doc()
            .project()
            .speaker(speaker_id)
            .map(|speaker| speaker.label.clone())
        else {
            cx.notify();
            return;
        };
        self.speaker_rename = Some((
            SharedString::from(segment_id.to_string()),
            speaker_id.to_string(),
        ));
        self.speaker_input.update(cx, |input, cx| {
            input.set_value(label, window, cx);
            input.select_all(window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Enter (`from_blur = false`) or leaving the cell (`true`): applies the typed name to the
    /// speaker (every row of that speaker follows).
    fn commit_speaker_rename(
        &mut self,
        from_blur: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, speaker_id)) = self.speaker_rename.take() else {
            return;
        };
        let label = self.speaker_input.read(cx).value().to_string();
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| doc.rename_speaker(&speaker_id, &label));
        });
        if !from_blur {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    /// Escape: leaves the cell without renaming.
    fn cancel_speaker_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.speaker_rename.take().is_none() {
            return;
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn on_speaker_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.speaker_rename.is_none() {
            return;
        }
        match event {
            InputEvent::PressEnter { .. } => self.commit_speaker_rename(false, window, cx),
            InputEvent::Blur => {
                if !input.focus_handle(cx).is_focused(window) {
                    self.commit_speaker_rename(true, window, cx);
                }
            }
            InputEvent::Change | InputEvent::Focus => {}
        }
    }

    /// Row menu `Split at playhead`: splits the row where the playhead is (rejected outside it).
    pub(super) fn split_row_at_playhead(
        &mut self,
        segment_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.blur_text(window, cx);
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| {
                let at_seconds = doc.session().playhead_seconds;
                doc.split_segment_at(segment_id, at_seconds)
            });
        });
    }

    pub(super) fn run_row_command(
        &mut self,
        command: TranscriptActionCommand,
        segment_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.blur_text(window, cx);
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| match command {
                TranscriptActionCommand::Edit => doc.open_segment_editor(segment_id),
                TranscriptActionCommand::MergeAbove => doc.merge_above(segment_id),
                TranscriptActionCommand::MergeBelow => doc.merge_below(segment_id),
                TranscriptActionCommand::CreateBefore => doc.create_segment_before(segment_id),
                TranscriptActionCommand::CreateAfter => doc.create_segment_after(segment_id),
                TranscriptActionCommand::Delete => doc.delete_segment(segment_id),
            });
        });
    }

    // ---- rendering ----------------------------------------------------------------------

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let rows = self.frame.rows.clone();
        let sizes = self.frame.sizes.clone();
        let metrics = TextMetrics::for_window(window);
        let cx: &Context<Self> = cx;
        let doc = self.model.read(cx).doc();
        let is_playing = doc.session().is_playing;
        let fallback = cx.theme().foreground;

        // Static row text paints the wraps the row heights were measured with.
        let text_column_width = self.text_column_width.get();
        let wraps: Vec<_> = range
            .filter_map(|index| {
                let row = rows.get(index)?;
                Some((
                    row,
                    self.metrics.text_wrap(row, doc, text_column_width, window),
                    index,
                ))
            })
            .collect();

        wraps
            .into_iter()
            .map(|(row, text_wrap, index)| {
                let params = RowParams {
                    row,
                    doc,
                    text_wrap,
                    height: sizes.get(index).map_or(px(32.), |size| size.height),
                    metrics,
                    lane_color: self
                        .frame
                        .lane_colors
                        .get(&row.lane_id)
                        .copied()
                        .unwrap_or(fallback),
                    is_find_match: self.frame.find_matches.contains(&row.segment_id),
                    is_playing,
                    text_editing: self.text_row.as_deref() == Some(row.segment_id.as_str()),
                    time_editing: self
                        .time_cell
                        .as_ref()
                        .filter(|(id, _)| id.as_ref() == row.segment_id)
                        .map(|(_, field)| *field),
                };
                self.render_row(params, cx)
            })
            .collect()
    }

    /// Speaker strip on top of a one-lane grid: lane color dot and lane label.
    fn render_lane_header(&self, doc: &EditorDoc, cx: &Context<Self>) -> Option<impl IntoElement> {
        let lane_id = self.lane_id()?;
        let lane = doc.project().lanes.iter().find(|lane| lane.id == lane_id)?;
        let theme = cx.theme();
        let color = self
            .frame
            .lane_colors
            .get(lane_id)
            .copied()
            .unwrap_or(theme.foreground);
        Some(
            h_flex()
                .w_full()
                .h(rems(1.75))
                .flex_none()
                .gap_2()
                .px_2()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .child(div().size(rems(0.625)).flex_none().rounded_full().bg(color))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child(lane.label.clone()),
                ),
        )
    }

    fn render_header(&self, doc: &EditorDoc, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let cells = doc.session().transcript_visible_cells(doc.project());
        let width = self.text_column_width.clone();
        let label = |text: &'static str, w: Option<Rems>| {
            div()
                .when_some(w, |el, w| el.w(w).flex_none())
                .px(CELL_PX)
                .truncate()
                .child(text)
        };

        h_flex()
            .w_full()
            .h(rems(1.75))
            .flex_none()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .when(cells.id, |el| {
                el.child(label(localized(cx, "#", "№"), Some(ID_W)))
            })
            .when(cells.track, |el| {
                el.child(label(localized(cx, "Speaker", "Спикер"), Some(TRACK_W)))
            })
            .when(cells.start, |el| {
                el.child(label(localized(cx, "Start", "Начало"), Some(TIME_W)))
            })
            .when(cells.end, |el| {
                el.child(label(localized(cx, "End", "Конец"), Some(TIME_W)))
            })
            .when(cells.reviewer, |el| {
                el.child(label(
                    localized(cx, "Reviewer", "Ревьюер"),
                    Some(REVIEWER_W),
                ))
            })
            .when(cells.text, |el| {
                el.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .relative()
                        .pl(TEXT_PX)
                        .child(localized(cx, "Text", "Текст"))
                        .child(
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
                            .size_full(),
                        ),
                )
            })
            .when(cells.linter, |el| {
                el.child(
                    h_flex()
                        .w(LINT_W)
                        .flex_none()
                        .justify_center()
                        .child(localized(cx, "Checks", "Проверки")),
                )
            })
            .when(cells.actions, |el| el.child(div().w(ACTIONS_W).flex_none()))
    }

    fn render_empty_state(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let title = localized(cx, "No segments yet", "Пока нет сегментов");
        let description = localized(
            cx,
            "Drag across the waveform to create a segment, then type what was said.",
            "Выделите участок на волне, чтобы создать сегмент, и введите текст.",
        );

        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_2()
            .p_6()
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(title),
            )
            .child(
                div()
                    .max_w(rems(26.25))
                    .text_sm()
                    .text_center()
                    .text_color(theme.muted_foreground)
                    .child(description),
            )
    }

    fn render_tag_suggestions(
        &self,
        doc: &EditorDoc,
        scale: f32,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let Some(list) = self
            .text_row
            .as_ref()
            .filter(|_| self.text_focused)
            .and_then(|segment_id| doc.completions(segment_id))
        else {
            self.tag_highlight.set(None);
            return None;
        };
        let (caret, line_height) = self.text_input.read(cx).cursor_layout()?;
        let anchor = point(
            caret.origin.x - px(4. * scale),
            caret.origin.y + line_height + px(4. * scale),
        );
        if self.tag_highlight.replace(Some(list.highlighted)) != Some(list.highlighted) {
            self.tag_scroll.scroll_to_item(list.highlighted);
        }
        let theme = cx.theme();
        let items = list.items.into_iter().enumerate().map(|(index, item)| {
            let highlighted = index == list.highlighted;
            let label = item.label.clone();
            div()
                .id(("tag-suggestion", index))
                .px_2()
                .py_0p5()
                .rounded(theme.radius)
                .text_sm()
                .cursor_pointer()
                .when(highlighted, |el| {
                    el.bg(theme.accent).text_color(theme.accent_foreground)
                })
                .hover(|style| style.bg(theme.list_hover))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.apply_completion(item.clone(), cx);
                }))
                .child(label)
        });

        Some(
            deferred(
                anchored()
                    .position(anchor)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        v_flex()
                            .id("tag-suggestions")
                            .occlude()
                            .min_w(rems(11.25))
                            .max_h(rems(13.75))
                            .overflow_y_scroll()
                            .track_scroll(&self.tag_scroll)
                            .p_1()
                            .gap_0p5()
                            .bg(theme.popover)
                            .text_color(theme.popover_foreground)
                            .border_1()
                            .border_color(theme.border)
                            .rounded(theme.radius)
                            .shadow_md()
                            .children(items),
                    ),
            )
            .with_priority(2),
        )
    }
}

impl Focusable for TranscriptView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TranscriptView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let doc = model.doc();
        let rows = self.rows(doc);
        let font_family = cx.theme().font_family.clone();
        let sizes = self.metrics.row_sizes(
            &rows,
            doc,
            self.text_column_width.get(),
            &font_family,
            window,
        );
        let find_matches = if doc.find_state().open {
            doc.find_matches().into_iter().collect()
        } else {
            HashSet::new()
        };
        let lane_colors = doc
            .project()
            .lanes
            .iter()
            .enumerate()
            .map(|(index, lane)| {
                (
                    lane.id.clone(),
                    Hsla::from(rgb(LaneColor::for_lane_index(index).rgb())),
                )
            })
            .collect();
        let sizes = Rc::new(sizes);
        let is_empty = rows.is_empty();
        self.frame = Frame {
            rows,
            sizes: sizes.clone(),
            find_matches,
            lane_colors,
        };
        self.follow_ghost(doc, window);

        let lane_header = self.render_lane_header(doc, cx);
        let header = self.render_header(doc, cx);
        let empty = is_empty.then(|| self.render_empty_state(cx));
        let tag_suggestions = self.render_tag_suggestions(doc, crate::zoom::ui_scale(window), cx);

        v_flex()
            .id("transcript")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .children(lane_header)
            .child(header)
            .children(empty)
            .when(!is_empty, |el| {
                el.child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .child(
                            v_virtual_list(
                                cx.entity(),
                                "transcript-rows",
                                sizes,
                                |this, range, window, cx| this.render_rows(range, window, cx),
                            )
                            .track_scroll(&self.scroll_handle),
                        )
                        .vertical_scrollbar(&self.scroll_handle),
                )
            })
            .children(tag_suggestions)
    }
}

/// The editor's selection as draft char offsets (`head` = caret).
fn input_selection(input: &TextareaState) -> TextSelection {
    let rope = input.text();
    let range = input.selected_range();
    let head = input.cursor();
    let anchor = if head == range.start {
        range.end
    } else {
        range.start
    };
    TextSelection::new(
        rope.offset_to_char_index(anchor),
        rope.offset_to_char_index(head),
    )
}
