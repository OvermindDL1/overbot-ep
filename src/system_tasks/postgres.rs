use crate::system::{System, SystemTask};
use anyhow::*;
use pg_embed::fetch::{Architecture, FetchSettings, OperationSystem, PG_V13};
use pg_embed::postgres::{PgEmbed, PgSettings};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::*;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub enum ConnectionType {
	External(String),
	Embedded {
		root_path: PathBuf,
		port: i16,
		username: String,
		password: String,
		persistent: bool,
		start_timeout: Duration,
		host: String,
	},
}

enum ConnectionWrapper {
	External(String),
	Embedded(Box<PgEmbed>),
}

impl ConnectionWrapper {
	fn as_uri(&self) -> &str {
		match self {
			ConnectionWrapper::External(conn_string) => &conn_string,
			ConnectionWrapper::Embedded(pg) => &pg.db_uri,
		}
	}
}

impl Drop for ConnectionWrapper {
	fn drop(&mut self) {
		match self {
			ConnectionWrapper::External(_) => (),
			ConnectionWrapper::Embedded(pg) => {
				info!("Shutting down embedded database...");
				if let Err(e) = pg.stop_db() {
					error!("error upon shutting down embedded postgres: {:?}", e);
				} else {
					info!("Embedded database is shut down");
				}
			}
		}
	}
}

fn get_fetch_settings(host: String) -> anyhow::Result<FetchSettings> {
	let operating_system = if cfg!(target_os = "linux") {
		OperationSystem::Linux
	} else if cfg!(target_os = "macos") {
		OperationSystem::Darwin
	} else if cfg!(target_os = "windows") {
		OperationSystem::Windows
	} else {
		bail!("unsupported `target_os`");
	};

	let architecture = if cfg!(target_arch = "x86_64") {
		Architecture::Amd64
	} else if cfg!(target_arch = "aarch64") {
		Architecture::Arm64v8
	} else {
		bail!("unsupported `target_arch`");
	};

	Ok(FetchSettings {
		host: host.clone(),
		operating_system,
		architecture,
		// Yay hardcoding because can't select a custom version that PgEmbed doesn't have hardcoded in for some reason, 13 is old but may as well as its the latest for pg_embed...
		version: PG_V13,
	})
}

impl ConnectionType {
	async fn init_conn_string(&self) -> anyhow::Result<ConnectionWrapper> {
		match self {
			ConnectionType::External(conn_string) => {
				Ok(ConnectionWrapper::External(conn_string.clone()))
			}
			ConnectionType::Embedded {
				root_path,
				port,
				username,
				password,
				persistent,
				start_timeout,
				host,
			} => {
				info!("initializing an embedded postgresql database");
				let executables_dir: String = root_path
					.join("postgres")
					.to_str()
					.with_context(|| {
						format!(
							"unable to map executable path to a utf8 string: {:?}",
							root_path.join("postgres")
						)
					})?
					.to_owned();
				let database_dir = root_path
					.join("db")
					.to_str()
					.with_context(|| {
						format!(
							"unable to map database path to a utf8 string: {:?}",
							root_path.join("db")
						)
					})?
					.to_owned();

				let pg_settings = PgSettings {
					// Why are these utf-8 strings instead of `Path`/`PathBuf`'s?!?
					executables_dir: executables_dir.clone(),
					database_dir: database_dir.clone(),
					// Why is port an `i16` instead of a `u16`?!?
					port: *port,
					user: username.to_owned(),
					password: password.to_owned(),
					persistent: *persistent,
					start_timeout: *start_timeout,
					migration_dir: None,
				};

				info!("Initializing embedded postgresql database");
				let mut pg = PgEmbed::new(pg_settings, get_fetch_settings(host.clone())?);

				info!("Setting up embedded postgresql database");
				// Download, unpack, create password file and database cluster
				// This is the next 3 lines: pg.setup().await?;
				pg.aquire_postgres().await?;
				pg.create_password_file().await?;
				{
					// Workaround for PgEmbed bug of trying to use the password as the authentication type...  >.<
					let pg_settings = PgSettings {
						// Why are these utf-8 strings instead of `Path`/`PathBuf`'s?!?
						executables_dir,
						database_dir,
						// Why is port an `i16` instead of a `u16`?!?
						port: *port,
						user: username.to_owned(),
						password: "scram-sha-256".to_owned(),
						persistent: *persistent,
						start_timeout: *start_timeout,
						migration_dir: None,
					};
					let pg = PgEmbed::new(pg_settings, get_fetch_settings(host.clone())?);
					pg.init_db().await?;
				}

				info!("Starting embedded postgresql database");
				// start postgresql database
				pg.start_db().await?;

				info!("Embedded postgresql database successfully started");
				info!("Database connection URI: {}", &pg.db_uri);
				Ok(ConnectionWrapper::Embedded(Box::new(pg)))
			}
		}
	}
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct Postgres {
	enabled: bool,
	connection: ConnectionType,
	max_connections: u8,
}

impl Postgres {
	#[allow(clippy::too_many_arguments)]
	pub fn new_embedded(
		enabled: bool,
		max_connections: u8,
		root_path: impl Into<PathBuf>,
		port: i16,
		username: impl Into<String>,
		password: impl Into<String>,
		persistent: bool,
		start_timeout: Duration,
		host: Option<String>,
	) -> Self {
		let connection = ConnectionType::Embedded {
			root_path: root_path.into(),
			port,
			username: username.into(),
			password: password.into(),
			persistent,
			start_timeout,
			host: host
				.map(Into::into)
				.unwrap_or_else(|| "https://repo1.maven.org".to_owned()),
		};
		Self {
			enabled,
			connection,
			max_connections,
		}
	}

	pub fn new_external(enabled: bool, max_connections: u8, uri: impl Into<String>) -> Self {
		Self {
			enabled,
			connection: ConnectionType::External(uri.into()),
			max_connections,
		}
	}
}

#[typetag::serde]
impl SystemTask for Postgres {
	fn spawn(&self, _self_name: &str, system: &System) -> anyhow::Result<Option<JoinHandle<()>>> {
		if !self.enabled {
			return Ok(None);
		}
		let registered_data = system.registered_data.clone();
		let connection = self.connection.clone();
		let max_connections = self.max_connections as u32;
		let do_quit = system.quit.clone();
		let mut on_quit = system.quit.subscribe();
		let handle = tokio::task::spawn(async move {
			info!("Initializing postgresql database connection");
			let conn_string = match connection.init_conn_string().await {
				Ok(conn_string) => conn_string,
				Err(e) => {
					error!(
						"Unable to initialize database connection string due to: {:?}",
						e
					);
					let _ = do_quit.send(());
					return;
				}
			};

			let uri = conn_string.as_uri();

			match PgPoolOptions::new()
				.max_connections(max_connections)
				.connect(uri)
				.await
			{
				Ok(pool) => {
					let pool: Arc<PgPool> = Arc::new(pool);
					match registered_data.insert(pool) {
						Err(e) => {
							error!("Cannot register multiple database instances into the registered_data: {}", e);
						}
						Ok(()) => {
							info!("Successfully initialized the database connection pool");
							// Fully registered, now wait until quit is requested
							let _ = on_quit.recv().await;
							info!("Shutdown request, attempting to stop the postgresql database connection pool");
							let pg_pool = Arc::downgrade(&registered_data.remove::<Arc<PgPool>>().expect("Nothing should remove the database pool except the pool manager task"));
							for _ in 0..50usize {
								if pg_pool.upgrade().is_none() {
									break;
								}
								tokio::time::sleep(Duration::from_millis(100)).await;
							}
							if pg_pool.upgrade().is_some() {
								error!("Database pool is still open by something after 5 seconds");
							} else {
								info!("Database pool is now shut down");
							}
						}
					}
				}
				Err(e) => {
					error!("Error connecting to database: {:?}", e);
				}
			}

			let _ = do_quit.send(());
			drop(conn_string);
		});
		Ok(Some(handle))
	}
}
