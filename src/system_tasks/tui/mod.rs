mod views;

use crate::dash_type_map::DashTypeMap;
use crate::logger::conditional_map::ConditionalMap;
use crate::system::{System, SystemTask};
use cursive::align::HAlign;
use cursive::event::Key;
use cursive::menu::MenuTree;
use cursive::view::*;
use cursive::views::*;
use cursive::{Cursive, CursiveRunnable};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::task::{spawn_blocking, JoinHandle};
use tracing::{log::Level, *};
use views::*;

#[allow(clippy::upper_case_acronyms)]
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct TUI {
	enabled: bool,
}

impl TUI {
	pub fn new(enabled: bool) -> Self {
		Self { enabled }
	}
}

#[typetag::serde]
impl SystemTask for TUI {
	fn spawn(&self, _self_name: &str, system: &System) -> anyhow::Result<Option<JoinHandle<()>>> {
		if !(!system.daemon && (self.enabled || system.tui)) {
			return Ok(None);
		}
		let registered_data = system.registered_data.clone();
		let quit = system.quit.clone();
		let on_quit = system.quit.subscribe();
		let handle = spawn_blocking(move || {
			info!("TUI is starting up");
			let mut siv = cursive::default();
			{
				siv.add_global_callback('l', |_siv| info!("Logging a loggy log by 'l'"));
			}
			setup_ui(&mut siv, registered_data, quit.clone());
			info!("TUI started, disabling the loggers conditional `console` output while it draws");
			// Disable the logger while this runs
			ConditionalMap::get_or_create_by_id("console".to_owned(), false)
				.store(false, Ordering::SeqCst);
			tui_run_loop(&mut siv, quit, on_quit);
			// And re-enable logger after
			ConditionalMap::get_by_id("console")
				.unwrap()
				.store(true, Ordering::SeqCst);
		});
		Ok(Some(handle))
	}
}

const LOG_VIEW_HIDER: &str = "log_view_hider";

fn toggle_named_hideable<V: View>(siv: &mut Cursive, name: &str) {
	if let Some(mut view) = siv.find_name::<HideableView<V>>(name) {
		if view.is_visible() {
			view.hide()
		} else {
			view.unhide()
		}
	} else {
		warn!("Attempted to toggle view `{}` but it was not found", name);
	}
}

fn setup_ui(
	siv: &mut CursiveRunnable,
	_registered_data: Arc<DashTypeMap>,
	quit: broadcast::Sender<()>,
) {
	// This is buggy as is doesn't appear "over" other things when focused... keep false
	siv.set_autohide_menu(false);
	siv.menubar()
		.add_subtree(
			"Overbot",
			MenuTree::new()
				// This is buggy as is doesn't appear "over" other things when focused... keep false
				// .leaf("Toggle Menubar Autohide", |siv| {
				// 	let autohide = !siv.menubar().autohide;
				// 	siv.set_autohide_menu(autohide)
				// })
				.delimiter()
				.leaf("Exit", move |_siv| {
					let _ = quit.send(());
				}),
		)
		.add_subtree(
			"Views",
			MenuTree::new()
				.subtree(
					"Toggle Visbility",
					MenuTree::new().leaf("Toggle Log", |siv| {
						toggle_named_hideable::<Panel<ResizedView<LogView>>>(siv, LOG_VIEW_HIDER)
					}),
				)
				.leaf("Set Max Log Level", |siv| {
					if let Some(mut log_view) =
						siv.find_name::<HideableView<Panel<ResizedView<LogView>>>>(LOG_VIEW_HIDER)
					{
						let log_view = log_view.get_inner_mut().get_inner_mut().get_inner_mut();
						let mut selector = SelectView::<Level>::new().h_align(HAlign::Left);
						for &level in &[
							Level::Error,
							Level::Warn,
							Level::Info,
							Level::Debug,
							Level::Trace,
						] {
							selector.add_item(level.as_str(), level)
						}
						selector.set_selection(log_view.max_level() as usize - 1);
						selector.set_on_submit(|siv, level| {
							siv.pop_layer();
							if let Some(mut log_view) = siv
								.find_name::<HideableView<Panel<ResizedView<LogView>>>>(
									LOG_VIEW_HIDER,
								) {
								let log_view =
									log_view.get_inner_mut().get_inner_mut().get_inner_mut();
								log_view.set_max_level(*level);
							} else {
								warn!(
								"Attempted to set log view `{}` filter level but it was not found",
								LOG_VIEW_HIDER
							);
							}
						});
						siv.add_layer(
							Dialog::around(
								LinearLayout::vertical()
									.child(
										LinearLayout::horizontal()
											.child(TextView::new("Current Level: "))
											.child(TextView::new(log_view.max_level().as_str())),
									)
									.child(selector),
							)
							.dismiss_button("Cancel"),
						);
					} else {
						warn!(
							"Attempted to set log view `{}` filter level but it was not found",
							LOG_VIEW_HIDER
						);
					}
				}),
		)
		.add_subtree(
			"Help",
			MenuTree::new().leaf("About", move |siv| {
				siv.add_layer(Dialog::info(&format!(
					"Cursive v{}\n{}",
					env!("CARGO_PKG_VERSION"),
					env!("CARGO_PKG_DESCRIPTION")
				)))
			}),
		);
	siv.add_global_callback(Key::Esc, |siv| siv.select_menubar());

	siv.add_fullscreen_layer(
		LinearLayout::vertical().child(
			HideableView::new(
				Panel::new(
					LogView::default().resized(SizeConstraint::Full, SizeConstraint::Fixed(6)),
				)
				.title("System Log"),
			)
			.with_name(LOG_VIEW_HIDER),
		),
	);
}

#[tracing::instrument(
	name = "TUI RunLoop",
	target = "overbot::system",
	skip(siv, quit, on_quit)
)]
fn tui_run_loop(
	siv: &mut CursiveRunnable,
	quit: broadcast::Sender<()>,
	mut on_quit: broadcast::Receiver<()>,
) {
	let mut runner = siv.runner();
	runner.refresh();

	// TODO: Read the primary event processor here
	while runner.is_running() {
		use tokio::sync::broadcast::error::TryRecvError;
		match on_quit.try_recv() {
			Ok(()) => runner.quit(),
			Err(TryRecvError::Empty) => {}
			Err(TryRecvError::Closed) => runner.quit(),
			Err(TryRecvError::Lagged(_)) => runner.quit(),
		}
		// Run this to refresh changes that came out of band: runner.on_event(cursive::event::Event::Refresh);
		// process events from main event pipe here, and then:
		let mut processed = 0;
		while processed < 100 && runner.is_running() && runner.process_events() {
			processed += 1;
			if !runner.is_running() {
				break;
			}
		}
		runner.refresh();
		// Unfortunately we have to poll changes in cursive, we can't just `await` it..  >.<
		// Which is annoying because it is stdin, so absolutely could await it...
		// But, we need to poll, so... don't busy wait, sleep a bit and try again...
		sleep(Duration::from_millis(100));
	}

	// TUI closed, let's go ahead and post a quit regardless of if it was (should) already sent
	let _ = quit.send(());
}
