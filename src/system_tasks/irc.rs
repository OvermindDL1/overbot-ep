use crate::database::Migrations;
use crate::system::{QuitOnError, System, SystemPlugin};
use tokio::task::JoinHandle;
use tracing::*;

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct IrcConnections {}

#[allow(clippy::upper_case_acronyms)]
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct IRC {
	enabled: bool,
	data_path: String,
}

impl IRC {
	pub fn new(enabled: bool) -> Self {
		Self {
			enabled,
			data_path: "irc".to_owned(),
			// connections: Default::default(),
		}
	}

	pub fn irc_data(self, data_path: String) -> Self {
		Self { data_path, ..self }
	}
}

#[typetag::serde]
impl SystemPlugin for IRC {
	fn spawn(&self, system: &System) -> Option<JoinHandle<anyhow::Result<()>>> {
		if !self.enabled {
			return None;
		}
		let db_pool = system.db_pool.clone();
		// let registered_data = system.registered_data.clone();
		let do_quit = system.quit.clone();
		// let mut on_quit = system.quit.subscribe();
		let handle = tokio::task::spawn(async move {
			info!("IRC Handler task has launched");
			MIGRATIONS
				.migrate_up(&db_pool)
				.await
				.quit_on_err(&do_quit)?;
			// let pg_pool = MIGRATIONS
			// 	.migrate_up_and_get_pool(&registered_data, Duration::from_secs(60))
			// 	.await
			// 	.quit_on_err(&do_quit)?;
			Ok(())
		});
		Some(handle)
	}
}

const MIGRATIONS: Migrations = Migrations::new("IRC", &[]);
