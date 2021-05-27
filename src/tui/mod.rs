mod views;

use crate::logger::conditional_map::ConditionalMap;
use crate::system::{System, SystemTask};
use cursive::event::{EventTrigger, Key};
use cursive::views::Dialog;
use cursive::CursiveRunnable;
use std::sync::atomic::Ordering;
use std::thread::sleep;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::task::{spawn_blocking, JoinHandle};
use tracing::*;
use views::*;

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
			{
				let quit = quit.clone();
				siv.add_global_callback(Key::Esc, move |siv| {
					let _ = quit.send(());
					// Update more rapidly so it gets closed quicker
					siv.set_fps(20);
				});
			}
			setup_ui(&mut siv, quit);
			info!("TUI started, disabling the loggers conditional `console` output while it draws");
			// Disable the logger while this runs
			ConditionalMap::get_or_create_by_id("console".to_owned(), false)
				.store(false, Ordering::SeqCst);
			tui_run_loop(&mut siv);
			// And re-enable logger after
			ConditionalMap::get_by_id("console")
				.unwrap()
				.store(true, Ordering::SeqCst);
		});
		Ok(Some(handle))
	}
}

fn setup_ui(siv: &mut CursiveRunnable, quit: broadcast::Sender<()>) {
	siv.add_layer(
		Dialog::around(LogView::default())
			.title("Logs")
			.button("Quit", move |_siv| {
				let _ = quit.send(());
			}),
	)
}

#[tracing::instrument(name = "TUI RunLoop", target = "overbot::system", skip(siv))]
fn tui_run_loop(siv: &mut CursiveRunnable) {
	siv.run();
	let mut runner = siv.runner();
	runner.refresh();

	// TODO: Read the primary event processor here
	loop {
		// Run this to refresh changes that came out of band: runner.on_event(cursive::event::Event::Refresh);
		// process events from main event pipe here, and then:
		let event_was_processed = runner.process_events();
		if !runner.is_running() {
			break;
		}
		if event_was_processed {
			// We did process something, loop again if not closing
		} else {
			// Unfortunately we have to poll changes in cursive, we can't just `await` it..  >.<
			// Which is annoying because it is stdin, so absolutely could await it...
			// But, we need to poll, so... don't busy wait, sleep a bit and try again...
			sleep(Duration::from_millis(100));
		}
	}

	// And the big event loop begins!
	while runner.is_running() {
		let received_something = runner.process_events();
		runner.post_events(received_something);
	}
}
