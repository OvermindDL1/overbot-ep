use crate::system::{System, SystemTask};
use tokio::task::JoinHandle;
use tracing::*;

#[allow(clippy::upper_case_acronyms)]
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct Daemon {
	enabled: bool,
}

impl Daemon {
	pub fn new(enabled: bool) -> Self {
		Self { enabled }
	}
}

#[typetag::serde]
impl SystemTask for Daemon {
	fn spawn(&self, _self_name: &str, system: &System) -> anyhow::Result<Option<JoinHandle<()>>> {
		if !(!system.tui && (self.enabled || system.daemon)) {
			return Ok(None);
		}
		let do_quit = system.quit.clone();
		let mut on_quit = system.quit.subscribe();
		let handle = tokio::task::spawn(async move {
			info!("Daemon task has launched");
			// Just wait until quit is requested, and then exit, or if ctrl+c is pressed, then exit safely.
			loop {
				#[cfg(target_os = "linux")]
				let mut hangup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
					.expect("failed registering hangup signal stream");
				#[cfg(target_os = "linux")]
				let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
					.expect("failed registering interrupt signal stream");
				#[cfg(target_os = "linux")]
				let mut quit = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::quit())
					.expect("failed registering quit signal stream");
				#[cfg(target_os = "linux")]
				let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
					.expect("failed registering terminate signal stream");
				#[cfg(target_os = "linux")]
				let do_break = tokio::select! {
					_ = hangup.recv() => {
						info!("Hangup requested, daemon mode ignores it");
						false
					}
					_ = interrupt.recv() => {
						info!("Interrupt signal received, cleanly exiting...");
						let _ = do_quit.send(());
						true
					}
					_ = quit.recv() => {
						info!("Quit signal received, cleanly exiting...");
						let _ = do_quit.send(());
						true
					}
					_ = terminate.recv() => {
						info!("Terminate signal received, cleanly exiting...");
						let _ = do_quit.send(());
						true
					}
					_ = on_quit.recv() => {
						info!("Daemon task has received a quit requested, exiting");
						true
					}
				};

				#[cfg(not(target_os = "linux"))]
				let do_break = tokio::select! {
					_ = tokio::signal::ctrl_c() => {
						info!("Ctrl+C signal received, cleanly exiting...");
						let _ = do_quit.send(());
						true
					}
					_ = on_quit.recv() => {
						info!("Daemon task has received a quit requested, exiting");
						true
					}
				};

				if do_break {
					break;
				}
			}
		});
		Ok(Some(handle))
	}
}
