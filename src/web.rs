use crate::dash_type_map::DashTypeMap;
use crate::database::DbPool;
use crate::database::Migrations;
use crate::system::{QuitOnError, System};
use rocket::tokio::sync::broadcast;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::*;

#[derive(serde::Deserialize, serde::Serialize)]
pub struct WebConfig {
	url_root: String,
}

impl Default for WebConfig {
	fn default() -> Self {
		Self {
			url_root: "/".to_owned(),
		}
	}
}

impl WebConfig {
	pub fn new(url_root: impl Into<String>) -> Self {
		Self {
			url_root: url_root.into(),
		}
	}

	pub async fn runner(
		url_root: String,
		db_pool: DbPool,
		data: Arc<DashTypeMap>,
		quit: broadcast::Sender<()>,
	) -> anyhow::Result<()> {
		MIGRATIONS.migrate_up(&db_pool).await?;
		info!("Building the web UI");
		let rocket = rocket::build()
			.manage(db_pool)
			.manage(data)
			.mount(&url_root, rocket::routes![]);
		info!("Igniting the rocket web UI");
		let rocket = rocket.ignite().await.quit_on_err(&quit)?;
		let shutdown = rocket.shutdown();
		let mut on_quit = quit.subscribe();
		tokio::spawn(async move {
			let _ = on_quit.recv().await;
			info!("Shutdown requested, sending graceful shutdown request to the rocket web UI");
			shutdown.notify();
		});
		info!("Launching the rocket web ui");
		rocket.launch().await.quit_on_err(&quit)?;
		info!("Rocket Web UI had a successful shutdown");
		let _ = quit.send(());
		Ok(())
	}

	pub fn spawn(&self, system: &System) -> JoinHandle<anyhow::Result<()>> {
		tokio::spawn(Self::runner(
			self.url_root.clone(),
			system.db_pool.clone(),
			system.registered_data.clone(),
			system.quit.clone(),
		))
	}
}

const MIGRATIONS: Migrations = Migrations::new("Web", &[]);
