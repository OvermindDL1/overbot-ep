use crate::database::Migrations;
use crate::system::{QuitOnError, System, SystemPlugin};
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
impl SystemPlugin for WebUiRocket {
	fn spawn(&self, system: &System) -> Option<JoinHandle<anyhow::Result<()>>> {
		if !self.enabled {
			return None;
		}
		let url_root = self.url_root.clone();
		let registered_data = system.registered_data.clone();
		let db_pool = system.db_pool.clone();
		let do_quit = system.quit.clone();
		let mut on_quit = system.quit.subscribe();
		let handle = tokio::spawn(async move {
			info!("Building the rocket web UI, waiting up to 60 seconds for the database pool to initialize");
			MIGRATIONS.migrate_up(&db_pool).await?;
			info!("Rocket Web UI got the database pool");
			let rocket = rocket::build()
				.manage(registered_data)
				.manage(db_pool)
				.mount(&url_root, rocket::routes![]);
			info!("Igniting the rocket web UI");
			let rocket = rocket.ignite().await.quit_on_err(&do_quit)?;
			let shutdown = rocket.shutdown();
			tokio::spawn(async move {
				let _ = on_quit.recv().await;
				info!("Shutdown requested, sending graceful shutdown request to the rocket web UI");
				shutdown.notify();
			});
			info!("Launching the rocket web ui");
			rocket.launch().await.quit_on_err(&do_quit)?;
			info!("Rocket Web UI had a successful shutdown");
			let _ = do_quit.send(());
			Ok(())
		});
		Some(handle)
	}
}

const MIGRATIONS: Migrations = Migrations::new("RocketWebUI", &[]);
