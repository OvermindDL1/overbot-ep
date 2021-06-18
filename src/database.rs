use anyhow::{bail, Context};
use pg_embed::fetch::{Architecture, FetchSettings, OperationSystem, PG_V13};
use pg_embed::postgres::{PgEmbed, PgSettings};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool, Transaction};
use std::convert::TryInto;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::*;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type")]
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

pub enum ConnectionLock {
	External(String),
	Embedded(Box<PgEmbed>),
}

impl ConnectionLock {
	fn as_uri(&self) -> &str {
		match self {
			ConnectionLock::External(conn_string) => &conn_string,
			ConnectionLock::Embedded(pg) => &pg.db_uri,
		}
	}
}

impl Drop for ConnectionLock {
	fn drop(&mut self) {
		match self {
			ConnectionLock::External(_) => (),
			ConnectionLock::Embedded(pg) => {
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
	async fn init_conn_string(&self) -> anyhow::Result<ConnectionLock> {
		match self {
			ConnectionType::External(conn_string) => {
				Ok(ConnectionLock::External(conn_string.clone()))
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
					.join("embedded_postgres")
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
				Ok(ConnectionLock::Embedded(Box::new(pg)))
			}
		}
	}
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct DatabaseConfig {
	connection: ConnectionType,
	max_connections: u8,
}

impl DatabaseConfig {
	#[allow(clippy::too_many_arguments)]
	pub fn new_embedded(
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
			connection,
			max_connections,
		}
	}

	pub fn new_external(max_connections: u8, uri: impl Into<String>) -> Self {
		Self {
			connection: ConnectionType::External(uri.into()),
			max_connections,
		}
	}

	pub async fn create_database_pool(&self) -> anyhow::Result<(ConnectionLock, DbPool)> {
		info!("Initializing postgresql database connection");
		let connection = self.connection.init_conn_string().await?;

		let pool = PgPoolOptions::new()
			.max_connections(self.max_connections as u32)
			.connect(connection.as_uri())
			.await?;

		let pool: DbPool = Arc::new(pool);
		migrate_migration_table(&pool)
			.await
			.expect("failed migrating the migration table");

		info!("Successfully initialized the database connection pool");
		Ok((connection, pool))
	}
}

pub type DbPool = Arc<PgPool>;
pub type DbTransaction<'a> = Transaction<'a, sqlx::Postgres>;

async fn migrate_migration_table(pool: &PgPool) -> anyhow::Result<()> {
	pool.execute(
		r#"
		CREATE TABLE IF NOT EXISTS _migrations (
			module text NOT NULL,
			version bigint NOT NULL,
			checksum bytea NOT NULL,
			description text NOT NULL,
			inserted_at timestamp without time zone NOT NULL DEFAULT now(),
			CONSTRAINT _migrations_pkey PRIMARY KEY (module, version)
		) WITH (
			OIDS=FALSE
		);
	"#,
	)
	.await?;
	info!("Migration Table loaded");
	Ok(())
}

#[derive(Clone)]
pub struct Migration<'d, 'su, 'sd> {
	pub description: &'d str,
	pub sql_up: &'su str,
	pub sql_down: &'sd str,
}

pub struct Migrations<'n, 'm, 'd, 'su, 'sd> {
	pub module: &'n str,
	pub migrations: &'m [Migration<'d, 'su, 'sd>],
}

impl<'d, 'su, 'sd> Migration<'d, 'su, 'sd> {
	pub const fn new(description: &'d str) -> Self {
		Self {
			description,
			sql_up: "",
			sql_down: "",
		}
	}

	pub const fn up(self, sql_up: &'su str) -> Self {
		// const-time panics are not yet in Rust:
		// assert!(self.sql_up.is_empty(), "cannot replace existing up sql");
		Self { sql_up, ..self }
	}

	pub const fn down(self, sql_down: &'sd str) -> Self {
		// const-time panics are not yet in Rust:
		// assert!(self.sql_down.is_empty(), "cannot replace existing down sql");
		Self { sql_down, ..self }
	}

	pub const fn sql(self, sql_up: &'su str, sql_down: &'sd str) -> Self {
		// const-time panics are not yet in Rust:
		// assert!(self.sql_up.is_empty(), "cannot replace existing up sql");
		// assert!(self.sql_down.is_empty(), "cannot replace existing down sql");
		Self {
			sql_up,
			sql_down,
			..self
		}
	}

	pub fn checksum(&self) -> [u8; 64] {
		use sha2::Digest;
		sha2::Sha512::default()
			.chain(self.sql_up.as_bytes())
			.chain(self.sql_down.as_bytes())
			.finalize()
			.as_slice()
			.try_into()
			.expect("somehow SHA512 is suddenly no longer returning 512 bits?!?")
	}

	async fn migrate_up(
		&self,
		module: &str,
		conn: &mut Transaction<'_, sqlx::Postgres>,
	) -> anyhow::Result<()> {
		info!("Migrate up {}", module);
		conn.execute(self.sql_up.as_ref()).await?;
		Ok(())
	}
}

impl<'n, 'm, 'd, 'su, 'sd> Migrations<'n, 'm, 'd, 'su, 'sd> {
	pub const fn new(module: &'n str, migrations: &'m [Migration<'d, 'su, 'sd>]) -> Self {
		Self { module, migrations }
	}

	pub async fn migrate_up(&self, pool: &PgPool) -> anyhow::Result<()> {
		if !self.migrations.is_empty() {
			info!("Migrating all up on {}", &self.module);
			// Why is the `conn.transaction` call wrapper boxing a future?!?  Wasteful...
			let mut conn = pool.begin().await?;
			// Why doesn't sqlx support decoding to unsigned integers?!
			// Why doesn't sqlx support decoding to a constant length array or reference thereof?!
			let mut current = sqlx::query_as::<_, (i64, Vec<u8>)>(
				"SELECT version, checksum FROM _migrations WHERE module = $1 ORDER BY version DESC",
			)
			.bind(self.module)
			.fetch_all(&mut conn)
			.await?;
			for (mig_version, mig) in self.migrations.iter().enumerate() {
				let mig_version = mig_version as i64;
				if let Some((version, checksum)) = current.pop() {
					if checksum.len() != 64 {
						bail!("Migration database checksum length is invalid for module {} with version {}", &self.module, version);
					} else if version != mig_version {
						bail!(
							"Version mismatch in {}: {} -> {}",
							&self.module,
							version,
							mig_version
						);
					} else if checksum != mig.checksum() {
						bail!(
							"Checksum mismatch in {} for version {}: {:?} -> {:?}",
							&self.module,
							version,
							&checksum,
							mig.checksum()
						);
					}
				} else {
					mig.migrate_up(&self.module, &mut conn).await?;
					sqlx::query("INSERT INTO _migrations(module, version, checksum, description) VALUES ($1, $2, $3, $4)")
						.bind(self.module)
						.bind(mig_version)
						.bind(mig.checksum().as_ref()) // Why can't sqlx accept an array directly to bind?
						.bind(mig.description)
						.execute(&mut conn)
						.await?;
				}
			}
			conn.commit().await?;
		}
		Ok(())
	}
}
