//! Compact post-game evaluation graph.

use iced::widget::canvas::{self, Cache, Canvas, Frame, Geometry, Path, Stroke};
use iced::{Color, Element, Length, Point, Rectangle, mouse};

use super::app::Msg;

pub fn view(scores: &[Option<i32>], height: f32) -> Element<'_, Msg> {
    Canvas::new(EvaluationGraph {
        scores: scores.to_vec(),
        cache: Cache::new(),
    })
    .width(Length::Fill)
    .height(Length::Fixed(height))
    .into()
}

struct EvaluationGraph {
    scores: Vec<Option<i32>>,
    cache: Cache,
}

impl canvas::Program<Msg> for EvaluationGraph {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            draw_background(frame, bounds);
            draw_scores(frame, bounds, &self.scores);
        });
        vec![geometry]
    }
}

fn draw_background(frame: &mut Frame, bounds: Rectangle) {
    let middle = bounds.height * 0.5;
    let baseline = Path::line(Point::new(0.0, middle), Point::new(bounds.width, middle));
    frame.stroke(
        &baseline,
        Stroke::default()
            .with_color(Color::from_rgba(0.72, 0.72, 0.76, 0.32))
            .with_width(1.0),
    );
}

fn draw_scores(frame: &mut Frame, bounds: Rectangle, scores: &[Option<i32>]) {
    let points: Vec<_> = scores
        .iter()
        .enumerate()
        .filter_map(|(index, score)| {
            let score = score.as_ref()?;
            let x = if scores.len() <= 1 {
                bounds.width * 0.5
            } else {
                index as f32 * bounds.width / (scores.len() - 1) as f32
            };
            // A tanh scale keeps decisive positions visible without flattening
            // ordinary positional swings around equality.
            let normalized = (*score as f32 / 450.0).tanh();
            let y = bounds.height * (0.5 - normalized * 0.44);
            Some(Point::new(x, y))
        })
        .collect();

    if points.is_empty() {
        return;
    }

    let line = Path::new(|builder| {
        builder.move_to(points[0]);
        for point in points.iter().skip(1) {
            builder.line_to(*point);
        }
    });
    frame.stroke(
        &line,
        Stroke::default()
            .with_color(Color::from_rgb(0.94, 0.69, 0.22))
            .with_width(2.5),
    );

    for point in points {
        frame.fill(&Path::circle(point, 2.4), Color::from_rgb(0.98, 0.84, 0.48));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn graph_accepts_sparse_and_decisive_scores() {
        let scores = [None, Some(0), Some(42), Some(30_000), Some(-30_000)];
        assert_eq!(scores.iter().flatten().count(), 4);
        assert!((30_000_f32 / 450.0).tanh() <= 1.0);
    }
}
