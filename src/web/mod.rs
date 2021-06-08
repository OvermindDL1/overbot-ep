pub mod auth;
pub mod macros;

use crate::accounts::Accounts;
use crate::dash_type_map::DashTypeMap;
use crate::database::DbPool;
use crate::database::Migrations;
use crate::system::{QuitOnError, System};
use crate::web::auth::{AuthControl, AuthSession};
use rocket::form::Form;
use rocket::http::{CookieJar, Status};
use rocket::State;
use sqlx::prelude::*;
use sqlx::{Column, TypeInfo, ValueRef};
use std::fmt::Write;
use std::sync::Arc;
use tokio::sync::broadcast;
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

#[derive(Debug, PartialEq, rocket::FromForm)]
struct RegisterData<'r> {
	login: &'r str,
	password: &'r str,
	password_check: &'r str,
}

#[rocket::get("/account")]
fn account(auth: AuthSession<'_>) -> String {
	format!("Things: {}", auth.user_session_ids)
}

#[rocket::get("/auth/login")]
fn login(auth_control: AuthControl<'_>, cookies: &CookieJar<'_>) -> String {
	if auth_control.is_logged_in() {
		format!("Already logged in: {:#?}", auth_control)
	} else {
		auth_control.login(cookies, "username", "super-secret-password", 60 * 60);
		format!("Now logged in: {:#?}", auth_control)
	}
}

#[rocket::get("/auth/register?<register>")]
async fn register(
	register: RegisterData<'_>,
	auth_control: AuthControl<'_>,
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
		}
	}

	pub async fn runner(
		url_root: String,
		db_pool: DbPool,
		data: Arc<DashTypeMap>,
		quit: broadcast::Sender<()>,
	) -> anyhow::Result<()> {
		MIGRATIONS.migrate_up(&db_pool).await.quit_on_err(&quit)?;

		info!("Building the web UI");
		let rocket = rocket::build().manage(db_pool).manage(data).mount(
			&url_root,
			rocket::routes![account, login, register, show_table],
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
		tokio::spawn(Self::runner(
			self.url_root.clone(),
			system.db_pool.clone(),
			system.registered_data.clone(),
			system.quit.clone(),
		))
	}
}

const MIGRATIONS: Migrations = Migrations::new("Web", &[]);
