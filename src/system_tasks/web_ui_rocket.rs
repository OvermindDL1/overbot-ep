use crate::system::{System, SystemTask};
use sqlx::PgPool;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::*;

#[allow(clippy::upper_case_acronyms)]
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct WebUiRocket {
	enabled: bool,
	url_root: String,
}

impl WebUiRocket {
	pub fn new(enabled: bool) -> Self {
		Self {
			enabled,
			url_root: "/".to_owned(),
		}
	}

	pub fn url_root(self, url_root: impl Into<String>) -> Self {
		Self {
			url_root: url_root.into(),
			..self
		}
	}
}

#[typetag::serde]
impl SystemTask for WebUiRocket {
	fn spawn(&self, _self_name: &str, system: &System) -> anyhow::Result<Option<JoinHandle<()>>> {
		if !self.enabled {
			return Ok(None);
		}
		let url_root = self.url_root.clone();
		let registered_data = system.registered_data.clone();
		let do_quit = system.quit.clone();
		let mut on_quit = system.quit.subscribe();
		let handle = tokio::spawn(async move {
			info!("Building the rocket web UI, waiting up to 60 seconds for the database pool to initialize");
			let pg_pool = match registered_data
				.wait_clone_if_arc::<PgPool>(Duration::from_secs(60))
				.await
			{
				Ok(pg_pool) => pg_pool,
				Err(e) => {
					error!("Rocket Web UI waited 60 seconds for the database pool to load, however: {}", e);
					let _ = do_quit.send(());
					return;
				}
			};
			info!("Rocket Web UI got the database pool");
			let rocket = rocket::build()
				.manage(registered_data)
				.manage(pg_pool)
				.mount(&url_root, rocket::routes![]);
			info!("Igniting the rocket web UI");
			match rocket.ignite().await {
				Ok(rocket) => {
					let shutdown = rocket.shutdown();
					tokio::spawn(async move {
						let _ = on_quit.recv().await;
						info!("Shutdown requested, sending graceful shutdown request to the rocket web UI");
						shutdown.notify();
					});
					info!("Launching the rocket web ui");
					if let Err(e) = rocket.launch().await {
						error!("Failed to launch rocket web UI: {}", e);
					} else {
						info!("Rocket Web UI had a successful shutdown");
					}
				}
				Err(e) => {
					error!("Failed to ignite the rocket web UI: {}", e);
				}
			}
			let _ = do_quit.send(());
		});
		Ok(Some(handle))
	}
}
