use std::any::Any;

use iced::advanced::widget::operation::{Focusable, Scrollable, TextInput};
use iced::advanced::widget::{self, Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::widget::text_input;
use iced::{Element, Length, Rectangle, Renderer, Size, Task, Theme, Vector};

use super::super::Message;

const TRANSCRIPT_TEXT_EDITOR_ID_PREFIX: &str = "transcript-editor:";
const TRANSCRIPT_TEXT_EDITOR_BLUR_ID: &str = "transcript-editor:__blur__";

pub(super) fn transcript_text_editor_id(segment_id: &str) -> text_input::Id {
    text_input::Id::new(format!("{TRANSCRIPT_TEXT_EDITOR_ID_PREFIX}{segment_id}"))
}

pub(in crate::app) fn focus_transcript_text_editor(segment_id: &str) -> Task<Message> {
    text_input::focus(transcript_text_editor_id(segment_id))
}

pub(in crate::app) fn blur_transcript_text_editors() -> Task<Message> {
    text_input::focus(text_input::Id::new(TRANSCRIPT_TEXT_EDITOR_BLUR_ID))
}

pub(super) fn view_focusable_text_editor<'a>(
    id: text_input::Id,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    Element::new(FocusableTextEditor {
        id: id.into(),
        content,
    })
}

struct FocusableTextEditor<'a> {
    id: widget::Id,
    content: Element<'a, Message>,
}

impl<'a> Widget<Message, Theme, Renderer> for FocusableTextEditor<'a> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.content.as_widget())]
    }

    fn diff(&self, tree: &mut Tree) {
        if let Some(child) = tree.children.get_mut(0) {
            child.diff(self.content.as_widget());
        } else {
            tree.children.push(Tree::new(self.content.as_widget()));
        }
        tree.children.truncate(1);
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget().layout(
            tree.children
                .get_mut(0)
                .expect("focusable text editor should have one child tree"),
            renderer,
            limits,
        )
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let mut operation = TranscriptTextEditorOperation {
            id: &self.id,
            operation,
        };
        self.content.as_widget().operate(
            tree.children
                .get_mut(0)
                .expect("focusable text editor should have one child tree"),
            layout,
            renderer,
            &mut operation,
        );
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> iced::event::Status {
        self.content.as_widget_mut().on_event(
            tree.children
                .get_mut(0)
                .expect("focusable text editor should have one child tree"),
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            tree.children
                .first()
                .expect("focusable text editor should have one child tree"),
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            tree.children
                .first()
                .expect("focusable text editor should have one child tree"),
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            tree.children
                .get_mut(0)
                .expect("focusable text editor should have one child tree"),
            layout,
            renderer,
            translation,
        )
    }
}

struct TranscriptTextEditorOperation<'a, 'b> {
    id: &'a widget::Id,
    operation: &'b mut dyn Operation,
}

impl Operation for TranscriptTextEditorOperation<'_, '_> {
    fn container(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        operate_on_children: &mut dyn FnMut(&mut dyn Operation),
    ) {
        self.operation.container(id, bounds, operate_on_children);
    }

    fn focusable(&mut self, state: &mut dyn Focusable, _id: Option<&widget::Id>) {
        self.operation.focusable(state, Some(self.id));
    }

    fn scrollable(
        &mut self,
        state: &mut dyn Scrollable,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
    ) {
        self.operation
            .scrollable(state, id, bounds, content_bounds, translation);
    }

    fn text_input(&mut self, state: &mut dyn TextInput, id: Option<&widget::Id>) {
        self.operation.text_input(state, id);
    }

    fn custom(&mut self, state: &mut dyn Any, id: Option<&widget::Id>) {
        self.operation.custom(state, id);
    }
}
