use iced::advanced::renderer::Renderer as _;
use iced::advanced::{Layout, Widget, layout, mouse, renderer};
use iced::{Color, Element, Length, Rectangle, Renderer, Size, Theme};

use super::super::Message;

pub(super) fn view_playhead_line<'a>(
    playhead_percent: f32,
    height: f32,
    width: f32,
    color: Color,
) -> Element<'a, Message> {
    Element::new(PlayheadLine {
        playhead_percent,
        height,
        width,
        color,
    })
}

struct PlayheadLine {
    playhead_percent: f32,
    height: f32,
    width: f32,
    color: Color,
}

impl Widget<Message, Theme, Renderer> for PlayheadLine {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.height.max(0.0)))
    }

    fn layout(
        &self,
        _tree: &mut iced::advanced::widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let height = self.height.max(0.0);
        let size = limits.resolve(
            Length::Fill,
            Length::Fixed(height),
            Size::new(limits.max().width, height),
        );
        layout::Node::new(size)
    }

    fn draw(
        &self,
        _tree: &iced::advanced::widget::Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let width = self.width.max(0.0).min(bounds.width);
        if width <= 0.0 || bounds.height <= 0.0 {
            return;
        }

        let x = bounds.x + playhead_pixel_x(self.playhead_percent, bounds.width, width);
        let line_bounds = Rectangle {
            x,
            y: bounds.y,
            width,
            height: bounds.height,
        };
        let Some(clipped_bounds) = line_bounds.intersection(viewport) else {
            return;
        };

        renderer.fill_quad(
            renderer::Quad {
                bounds: clipped_bounds,
                ..renderer::Quad::default()
            },
            self.color,
        );
    }
}

fn playhead_pixel_x(playhead_percent: f32, width: f32, cursor_width: f32) -> f32 {
    let fraction = (playhead_percent / 100.0).clamp(0.0, 1.0);
    (width - cursor_width).max(0.0) * fraction
}

#[cfg(test)]
mod tests {
    use super::playhead_pixel_x;

    #[test]
    fn playhead_pixel_x_preserves_subpercent_motion() {
        assert_eq!(playhead_pixel_x(12.5, 802.0, 2.0), 100.0);
        assert!((playhead_pixel_x(12.6, 802.0, 2.0) - 100.8).abs() < 0.001);
        assert_eq!(playhead_pixel_x(-10.0, 802.0, 2.0), 0.0);
        assert_eq!(playhead_pixel_x(120.0, 802.0, 2.0), 800.0);
    }
}
