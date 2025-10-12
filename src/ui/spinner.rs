use iced::widget::canvas::{self, Canvas, Geometry, Path, Stroke, LineCap};
use iced::widget::container;
use iced::{Element, Length, Color, Point, Background};
use std::f32::consts::PI;

#[derive(Debug, Clone)]
pub struct Spinner {
    rotation: f32,
}

impl Default for Spinner {
    fn default() -> Self {
        Self { rotation: 0.0 }
    }
}

impl Spinner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self) {
        // Rotate by 18 degrees per tick (180 degrees per second at 10 ticks/second)
        self.rotation += PI / 10.0; // 18 degrees in radians
        if self.rotation >= 2.0 * PI {
            self.rotation -= 2.0 * PI;
        }
    }

    pub fn view(&self) -> Element<'_, crate::types::Message> {
        container(
            Canvas::new(self)
                .width(Length::Fixed(20.0))
                .height(Length::Fixed(20.0))
        )
        .width(Length::Fixed(20.0))
        .height(Length::Fixed(20.0))
        .style(|_theme| container::Style {
            background: Some(Background::Color(Color::TRANSPARENT)),
            ..Default::default()
        })
        .into()
    }
}

impl canvas::Program<crate::types::Message> for Spinner {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius = bounds.width.min(bounds.height) / 2.0 - 4.0;

        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Draw a single gradient arc that spins
        let arc_length = PI * 1.5; // 3/4 of a full rotation (270 degrees)
        let num_segments = 20; // Number of segments to create gradient effect

        for i in 0..num_segments {
            let segment_angle = arc_length / num_segments as f32;
            let start_angle = self.rotation + (i as f32 * segment_angle);
            let end_angle = start_angle + segment_angle;

            // Calculate start and end points for this segment
            let start_point = Point::new(
                center.x + radius * start_angle.cos(),
                center.y + radius * start_angle.sin(),
            );

            let end_point = Point::new(
                center.x + radius * end_angle.cos(),
                center.y + radius * end_angle.sin(),
            );

            // Create gradient effect - segments fade from opaque to transparent
            let opacity = 1.0 - (i as f32 / num_segments as f32);
            let color = Color::from_rgba(0.5, 0.5, 0.5, opacity);

            // Draw the line segment
            let line_path = Path::line(start_point, end_point);
            let stroke = Stroke::default()
                .with_width(3.0)
                .with_line_cap(LineCap::Round);

            frame.stroke(&line_path, stroke.with_color(color));
        }

        vec![frame.into_geometry()]
    }
}
