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
		let quit = system.quit.clone();
		let mut on_quit = system.quit.subscribe();
		let handle = tokio::task::spawn(async move {
			info!("Daemon task has launched");
			// Just wait until quit is requested, and then exit, or if ctrl+c is pressed, then exit safely.
			tokio::select! {
				_ = tokio::signal::ctrl_c() => {
					info!("Ctrl+C signal received, cleanly exiting...");
					let _ = quit.send(());
				}
				_ = on_quit.recv() => {
					info!("Daemon task has received a quit requested, exiting");
				}
			}
		});
		Ok(Some(handle))
	}
}
