use cursive::theme::Color;
use cursive::{Printer, Vec2, View};

#[derive(Default)]
pub struct LogView {}

impl View for LogView {
	fn draw(&self, printer: &Printer<'_, '_>) {
		let logs = vec!["blah".to_owned(), "breep".to_owned()];

		let mut y = 0;
		for log in logs {
			printer.with_style(Color::Rgb(255, 0, 0), |printer| {
				printer.print((0, y), &log);
			});
			y += 1;
		}
	}

	fn required_size(&mut self, constraint: Vec2) -> Vec2 {
		Vec2::new(constraint.x.min(40), constraint.y.min(8))
	}
}
