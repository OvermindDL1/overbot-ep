use crate::logger::cache_appender::{Cache, CachedLogRecord};
use cursive::{theme, Printer, Vec2, View};
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use tracing::log::Level;

pub struct LogView {
	logs: Arc<RwLock<VecDeque<CachedLogRecord>>>,
	max_level: Level,
}

impl LogView {
	pub fn max_level(&self) -> Level {
		self.max_level
	}

	pub fn set_max_level(&mut self, max_level: Level) {
		self.max_level = max_level;
	}
}

impl Default for LogView {
	fn default() -> Self {
		let logs = Cache::get_or_create("tui_log_view".to_owned());
		LogView {
			logs,
			max_level: Level::Info,
		}
	}
}

impl View for LogView {
	fn draw(&self, printer: &Printer<'_, '_>) {
		if printer.size.y == 0 {
			return;
		}
		let logs = self.logs.read().expect("poisoned lock");

		for (offset, record) in logs
			.iter()
			.rev()
			.filter(|record| record.level() <= self.max_level)
			.take(printer.size.y)
			.enumerate()
		{
			let color = match record.level() {
				Level::Error => theme::BaseColor::Red.dark(),
				Level::Warn => theme::BaseColor::Yellow.dark(),
				Level::Info => theme::BaseColor::Black.light(),
				Level::Debug => theme::BaseColor::Green.dark(),
				Level::Trace => theme::BaseColor::Blue.dark(),
			};
			printer.with_color(color.into(), |printer| {
				printer.print((0, printer.size.y - offset - 1), record.msg());
			});
		}
	}

	fn required_size(&mut self, constraint: Vec2) -> Vec2 {
		Vec2::new(constraint.x.min(70), constraint.y.min(4))
	}
}
