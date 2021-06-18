pub mod auth;
pub mod macros;
pub mod static_files;

use crate::accounts::Accounts;
use crate::dash_type_map::DashTypeMap;
use crate::database::DbPool;
use crate::database::Migrations;
use crate::system::{QuitOnError, System};
use crate::web::auth::{AuthControl, AuthSession};
use crate::web::static_files::{Assets, StaticFile};
use rocket::config::{Ident, SecretKey, TlsConfig};
use rocket::data::Limits;
use rocket::http::{CookieJar, Status};
use rocket::State;
use serde::Serializer;
use sqlx::prelude::*;
use sqlx::{Column, TypeInfo, ValueRef};
use std::fmt::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::*;

fn secret_key_serialize_zero<S>(_secret_key: &SecretKey, ser: S) -> Result<S::Ok, S::Error>
where
	S: Serializer,
{
	ser.serialize_bytes(&[0; 32][..])
}

fn ident_as_string_serializer<S>(ident: &Ident, ser: S) -> Result<S::Ok, S::Error>
where
	S: Serializer,
{
	if let Some(s) = ident.as_str() {
		ser.serialize_str(s)
	} else {
		ser.serialize_bool(false)
	}
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct WebConfig {
	/// Root path, useful to change if hosted at a non root URL, **(default: "/")**
	pub url_root: String,
	/// IP address to serve on. **(default: `0.0.0.0`)**
	pub address: IpAddr,
	/// Port to serve on. **(default: `8000`)**
	pub port: u16,
	/// Number of threads to use for executing futures. **(default: `(num_cores+1)/2`)**
	pub workers: usize,
	/// Keep-alive timeout in seconds; disabled when `0`. **(default: `5`)**
	pub keep_alive: u32,
	/// Streaming read size limits. **(default: [`Limits::default()`])**
	pub limits: Limits,
	/// The TLS configuration, if any. **(default: `None`)**
	pub tls: Option<TlsConfig>,
	/// How, if at all, to identify the server via the `Server` header, or
	/// `false` for no server header.
	/// **(default: `"Overbot"`)**
	///
	/// # Errors
	///
	/// The string `ident` must be non-empty and may only contain visible ASCII
	/// characters. The first character cannot be whitespace. The only
	/// whitespace characters allowed are ` ` (space) and `\t` (horizontal tab).
	#[serde(serialize_with = "ident_as_string_serializer")]
	pub ident: Ident,
	/// The 256-bit key for signing and encrypting. **(default: `0`)**
	///
	/// If it's zero then it is randomly generated every load, good for testing,
	/// bad for actual production, so generate something random and set it.
	///
	/// This will never actually write out a non-zero key for security reasons,
	/// it's up to the user to set it in the config file
	#[serde(serialize_with = "secret_key_serialize_zero")]
	pub secret_key: SecretKey,
	/// Directory to store temporary files in. **(default:
	/// [`std::env::temp_dir()`])**
	pub temp_dir: PathBuf,
	/// Max level to log, `normal` or `critical`. **(default: `critical`)**
	pub log_level: rocket::config::LogLevel,
	/// The grace period: number of seconds to continue to try to finish
	/// outstanding _server_ I/O for before forcibly terminating it.
	///
	/// **default: `2`**
	pub grace: u32,
	/// The mercy period: number of seconds to continue to try to finish
	/// outstanding _connection_ I/O for before forcibly terminating it.
	///
	/// **default: `3`**
	pub mercy: u32,
	/// Whether to use colors and emoji when logging. **(default: `true`)**
	pub cli_colors: bool,
}

impl Default for WebConfig {
	fn default() -> Self {
		Self {
			url_root: "/".to_owned(),
			address: Ipv4Addr::new(0, 0, 0, 0).into(),
			port: 8000,
			workers: (rocket::Config::default().workers + 1) / 2,
			keep_alive: 5,
			limits: Limits::default(),
			tls: None,
			ident: Ident::try_new("Overbot").unwrap(),
			secret_key: rocket::Config::default().secret_key,
			temp_dir: std::env::temp_dir(),
			log_level: rocket::config::LogLevel::Critical,
			grace: 2,
			mercy: 3,
			cli_colors: true,
		}
	}
}

#[rocket::get("/<path..>", rank = 100)]
fn static_file(path: PathBuf) -> Option<StaticFile> {
	// Static files are urf-8 only:
	let path = path.to_str()?;
	Assets::get(path)
}

#[derive(Debug, PartialEq, rocket::FromForm)]
struct RegisterData<'r> {
	login: &'r str,
	password: &'r str,
	password_check: &'r str,
}

#[rocket::get("/account")]
fn account(auth: AuthSession<'_>) -> String {
	format!("Things: {}", auth.user_session)
}

#[rocket::get("/auth/login")]
async fn login(
	db_pool: &State<DbPool>,
	auth_control: AuthControl<'_>,
	cookies: &CookieJar<'_>,
) -> Result<String, (Status, &'static str)> {
	if auth_control.is_logged_in() {
		Ok(format!("Already logged in: {:#?}", auth_control))
	} else {
		auth_control
			.login(
				db_pool,
				cookies,
				"username",
				"super-secret-password",
				60 * 60,
			)
			.await
			.map_err(|_| (Status::Unauthorized, "invalid username or password"))?;
		Ok(format!("Now logged in: {:#?}", auth_control))
	}
}

#[rocket::get("/auth/register?<register>")]
async fn register(
	register: RegisterData<'_>,
	db_pool: &State<DbPool>,
) -> Result<String, (Status, String)> {
	let mut conn = db_pool.begin().await.map_err(|_e| {
		(
			Status::InternalServerError,
			"unable to access database".to_owned(),
		)
	})?;
	if register.password != register.password_check {
		return Err((Status::BadRequest, "passwords don't match".to_owned()));
	}
	let account = Accounts::create_account(&mut conn, register.login)
		.await
		.map_err(|e| (Status::BadRequest, e.to_string()))?;
	account
		.set_password(&mut conn, None, Some(register.password))
		.await
		.map_err(|e| (Status::BadRequest, e.to_string()))?;
	conn.commit().await.map_err(|_e| {
		(
			Status::InternalServerError,
			"database transaction failed".to_owned(),
		)
	})?;
	Ok("test".to_owned())
}

#[rocket::get("/db/tables/<table>")]
async fn show_table(
	table: &str,
	_auth: AuthSession<'_>,
	db_pool: &State<DbPool>,
) -> Result<String, String> {
	let query = format!(
		"SELECT * FROM {}",
		table
			.chars()
			.filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
			.collect::<String>()
	);
	// Have to use raw sql via a connection directly so binary encoding isn't used, which breaks on
	// some PGSQL types and sqlx isn't accounting for that...
	let rows = db_pool
		.inner()
		.acquire()
		.await
		.map_err(|e| e.to_string())?
		.fetch_all(query.as_str())
		.await
		.map_err(|e| e.to_string())?;
	// // Using a string query because sqlx isn't capable of interpolating a table name into position
	// let rows = sqlx::query(query.as_str())
	// 	.bind(table)
	// 	.fetch_all(db_pool.inner().as_ref())
	// 	.await
	// 	.map_err(|e| e.to_string())?;
	let mut ret = String::new();
	if let Some(row) = rows.first() {
		for column in row.columns() {
			ret.write_fmt(format_args!(
				"{}:{}\t",
				column.name(),
				column.type_info().name()
			))
			.map_err(|e| e.to_string())?;
		}
	}
	ret.push('\n');
	rows.into_iter().for_each(|row| {
		for column in row.columns() {
			let raw = row.try_get_raw(column.ordinal()).unwrap();
			if raw.is_null() {
				ret.push_str("{null}");
			} else if let Ok(data) = <&str as Decode<_>>::decode(raw.clone()) {
				ret.push_str(data);
			// } else if let Ok(data) = <i32 as Decode<_>>::decode(raw.clone()) {
			// 	let _ = ret.write_fmt(format_args!("{}", data));
			// } else if let Ok(data) = <bool as Decode<_>>::decode(raw.clone()) {
			// 	let _ = ret.write_fmt(format_args!("{}", data));
			} else {
				ret.push_str("{unsupported-type}");
			}
			ret.push('\t');
		}
		ret.push('\n');
	});
	Ok(ret)
}

impl WebConfig {
	pub fn new(url_root: impl Into<String>) -> Self {
		Self {
			url_root: url_root.into(),
			..Self::default()
		}
	}

	pub async fn runner(
		url_root: String,
		rocket_config: rocket::Config,
		db_pool: DbPool,
		data: Arc<DashTypeMap>,
		quit: broadcast::Sender<()>,
	) -> anyhow::Result<()> {
		MIGRATIONS.migrate_up(&db_pool).await.quit_on_err(&quit)?;

		info!("Building the web UI");
		let rocket = rocket::custom(rocket_config)
			.manage(db_pool)
			.manage(data)
			.mount(
				&url_root,
				rocket::routes![static_file, account, login, register, show_table],
			);

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
		let rocket_config = rocket::Config {
			address: self.address,
			port: self.port,
			workers: self.workers,
			keep_alive: self.keep_alive,
			limits: self.limits.clone(),
			tls: self.tls.clone(),
			ident: self.ident.clone(),
			secret_key: self.secret_key.clone(),
			temp_dir: self.temp_dir.clone(),
			log_level: self.log_level,
			shutdown: rocket::config::Shutdown {
				// `ctrlc` and signals are already handled by the bot system
				ctrlc: false,
				signals: Default::default(),
				grace: self.grace,
				mercy: self.mercy,
				force: false,
				..Default::default()
			},
			cli_colors: self.cli_colors,
			..Default::default()
		};

		tokio::spawn(Self::runner(
			self.url_root.clone(),
			rocket_config,
			system.db_pool.clone(),
			system.registered_data.clone(),
			system.quit.clone(),
		))
	}
}

const MIGRATIONS: Migrations = Migrations::new("Web", &[]);
