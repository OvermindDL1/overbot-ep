use crate::logger::conditional_map::ConditionalMap;
use crate::system::{System, SystemTask};
use cursive::event::{EventTrigger, Key};
use cursive::CursiveRunnable;
use std::sync::atomic::Ordering;
use tokio::task::{spawn_blocking, JoinHandle};
use tracing::*;

#[allow(clippy::upper_case_acronyms)]
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct TUI {
	enabled: bool,
}

#[typetag::serde]
impl SystemTask for TUI {
	fn spawn(&self, system: &System) -> anyhow::Result<Option<JoinHandle<()>>> {
		if system.daemon {
			// Forced daemon mode
			return Ok(None);
		}
		if !self.enabled && !system.tui {
			return Ok(None);
		}
		let quit = system.quit.clone();
		let mut on_quit = system.quit.subscribe();
		let handle = spawn_blocking(move || {
			info!("TUI is starting up");
			let mut siv = cursive::default();
			siv.set_fps(1);
			siv.set_on_post_event(EventTrigger::any(), move |siv| {
				use tokio::sync::broadcast::error::TryRecvError;
				match on_quit.try_recv() {
					Ok(()) => siv.quit(),
					Err(TryRecvError::Empty) => {}
					Err(TryRecvError::Closed) => siv.quit(),
					Err(TryRecvError::Lagged(_)) => siv.quit(),
				}
			});
			siv.add_global_callback(Key::Esc, move |siv| {
				let _ = quit.send(());
				// Update more rapidly so it gets closed quicker
				siv.set_fps(20);
			});
			info!("TUI started, disabling the loggers conditional `console` output while it draws");
			// Disable the logger while this runs
			ConditionalMap::get_or_create_by_id("console".to_owned(), false)
				.store(false, Ordering::SeqCst);
			#[tracing::instrument(name = "TUI RunLoop", target = "overbot::system", skip(siv))]
			fn tui_run_loop(siv: &mut CursiveRunnable) {
				siv.run()
			}
			tui_run_loop(&mut siv);
			// And re-enable logger after
			ConditionalMap::get_by_id("console")
				.unwrap()
				.store(true, Ordering::SeqCst);
		});
		Ok(Some(handle))
	}
}
