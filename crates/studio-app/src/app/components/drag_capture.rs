use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

use super::super::Message;

#[derive(Clone, Copy)]
enum DragAxis {
    Horizontal,
    Vertical,
}

#[derive(Default)]
struct DragCaptureState {
    dragging: bool,
    hovered: bool,
}

pub(super) fn view_horizontal_drag_capture<'a>(
    content: Element<'a, Message>,
    on_move: impl Fn(f32, f32) -> Message + 'a,
    on_press: Message,
    on_release: Message,
    on_exit: Option<Message>,
) -> Element<'a, Message> {
    Element::new(CapturedDragSurface {
        content,
        axis: DragAxis::Horizontal,
        on_move: Box::new(on_move),
        on_press,
        on_release,
        on_exit,
    })
}

pub(super) fn view_vertical_drag_capture<'a>(
    content: Element<'a, Message>,
    on_move: impl Fn(f32, f32) -> Message + 'a,
    on_press: Message,
    on_release: Message,
    on_exit: Option<Message>,
) -> Element<'a, Message> {
    Element::new(CapturedDragSurface {
        content,
        axis: DragAxis::Vertical,
        on_move: Box::new(on_move),
        on_press,
        on_release,
        on_exit,
    })
}

struct CapturedDragSurface<'a> {
    content: Element<'a, Message>,
    axis: DragAxis,
    on_move: Box<dyn Fn(f32, f32) -> Message + 'a>,
    on_press: Message,
    on_release: Message,
    on_exit: Option<Message>,
}

impl<'a> CapturedDragSurface<'a> {
    fn publish_move(&self, bounds: Rectangle, position: Point, shell: &mut Shell<'_, Message>) {
        let (coordinate, origin, extent) = match self.axis {
            DragAxis::Horizontal => (position.x, bounds.x, bounds.width.max(1.0)),
            DragAxis::Vertical => (position.y, bounds.y, bounds.height.max(1.0)),
        };
        let fraction = ((coordinate - origin) / extent).clamp(0.0, 1.0);
        shell.publish((self.on_move)(fraction, extent));
    }
}

impl<'a> Widget<Message, Theme, Renderer> for CapturedDragSurface<'a> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<DragCaptureState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(DragCaptureState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
        if let iced::event::Status::Captured = self.content.as_widget_mut().on_event(
            &mut tree.children[0],
            event.clone(),
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        ) {
            return iced::event::Status::Captured;
        }

        let bounds = layout.bounds();
        let hovered = cursor.is_over(bounds);
        let state: &mut DragCaptureState = tree.state.downcast_mut();
        if state.hovered
            && !hovered
            && !state.dragging
            && let Some(message) = self.on_exit.clone()
        {
            shell.publish(message);
        }
        state.hovered = hovered;

        match event {
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if state.dragging || hovered {
                    self.publish_move(bounds, position, shell);
                    return if state.dragging {
                        iced::event::Status::Captured
                    } else {
                        iced::event::Status::Ignored
                    };
                }
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if hovered => {
                state.dragging = true;
                if let Some(position) = cursor.position() {
                    self.publish_move(bounds, position, shell);
                }
                shell.publish(self.on_press.clone());
                return iced::event::Status::Captured;
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.dragging =>
            {
                state.dragging = false;
                shell.publish(self.on_release.clone());
                if !hovered && let Some(message) = self.on_exit.clone() {
                    shell.publish(message);
                }
                return iced::event::Status::Captured;
            }
            _ => {}
        }

        iced::event::Status::Ignored
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let content_interaction = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );
        let state: &DragCaptureState = tree.state.downcast_ref();
        if content_interaction == mouse::Interaction::None
            && (state.dragging || cursor.is_over(layout.bounds()))
        {
            mouse::Interaction::Pointer
        } else {
            content_interaction
        }
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
            &tree.children[0],
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
        self.content
            .as_widget_mut()
            .overlay(&mut tree.children[0], layout, renderer, translation)
    }
}
