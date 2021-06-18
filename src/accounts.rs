use crate::dash_type_map::DashTypeMap;
use crate::database::{DbPool, DbTransaction, Migration, Migrations};
use crate::system::{QuitOnError, System};
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::sync::Arc;
use time::{Duration, OffsetDateTime};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::*;
use uuid::Uuid;

#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct AccountsConfig {}

impl AccountsConfig {
	pub fn new() -> Self {
		Self {}
	}

	pub async fn runner(
		_config: AccountsConfig,
		_db_pool: DbPool,
		_data: Arc<DashTypeMap>,
		quit: broadcast::Sender<()>,
	) -> anyhow::Result<()> {
		quit.subscribe().recv().await?;
		Ok(())
	}

	pub async fn spawn(&self, system: &System) -> anyhow::Result<JoinHandle<anyhow::Result<()>>> {
		let db_pool = system.db_pool.clone();
		let quit = system.quit.clone();
		MIGRATIONS.migrate_up(&db_pool).await.quit_on_err(&quit)?;
		Ok(tokio::spawn(Self::runner(
			self.clone(),
			db_pool,
			system.registered_data.clone(),
			quit,
		)))
	}
}

pub struct Account {
	id: Uuid,
	login: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
	// Why isn't `argon2::password_hash::Error` or any other argon2 errors actually an Error type...
	#[error("password hashing error: {0}")]
	PasswordHash(argon2::password_hash::Error),
	#[error("password does not match")]
	PasswordDoesNotMatch,
	#[error("password cannot be the same as the current password and cannot be too short")]
	InvalidNewPassword,
	#[error("DatabaseError")]
	DatabaseError(#[from] sqlx::Error),
}

impl Account {
	fn new(id: Uuid, login: Option<String>) -> Self {
		Self { id, login }
	}

	pub fn hash_password(password: &str) -> Result<String, AccountError> {
		let salt = SaltString::generate(rand::thread_rng());
		let argon2 = Argon2::default();
		let hashed = argon2
			.hash_password_simple(password.as_bytes(), &salt)
			.map_err(AccountError::PasswordHash)?;
		Ok(hashed.to_string())
	}

	pub fn password_hash_matches(
		existing_password_hash: &PasswordHash,
		password: &str,
	) -> Result<(), AccountError> {
		Argon2::default()
			.verify_password(password.as_bytes(), &existing_password_hash)
			.map_err(|_| AccountError::PasswordDoesNotMatch)?;
		Ok(())
	}

	// Verifies the password is as it is passed in or it is NULL if None.
	pub async fn verify_password(
		&self,
		conn: &mut DbTransaction<'_>,
		password: Option<&str>,
	) -> Result<(), AccountError> {
		if let Some(password) = password {
			let existing_password_hash_string = sqlx::query_scalar::<_, Option<String>>(
				"SELECT password_hash FROM accounts_locals WHERE removed_at IS NULL AND id = $1",
			)
			.bind(self.id)
			.fetch_one(conn)
			.await?
			.ok_or(AccountError::PasswordDoesNotMatch)?;
			let existing_password_hash = PasswordHash::new(&existing_password_hash_string)
				.map_err(AccountError::PasswordHash)?;
			Argon2::default()
				.verify_password(password.as_bytes(), &existing_password_hash)
				.map_err(|_| AccountError::PasswordDoesNotMatch)?;
			Ok(())
		} else {
			sqlx::query("SELECT id FROM accounts_locals WHERE removed_at IS NULL AND id IS NULL")
				.fetch_one(conn)
				.await
				.map_err(|_| AccountError::PasswordDoesNotMatch)?;
			Ok(())
		}
	}

	pub async fn set_password(
		&self,
		conn: &mut DbTransaction<'_>,
		existing_password: Option<&str>,
		new_password: Option<&str>,
	) -> Result<(), AccountError> {
		if existing_password == new_password {
			// return Err(AccountError::InvalidNewPassword);
		}
		if let Some(new_password) = new_password {
			if new_password.len() <= 12 {
				return Err(AccountError::InvalidNewPassword);
			}
			// TODO:  Maybe a basic dictionary or DB dump check here as well?
		}
		info!(
			"Updating password for {}({})",
			self.id,
			self.login.as_ref().map(|s| s.as_ref()).unwrap_or("")
		);
		if let Some(new_password) = new_password {
			info!("Changing password for: {}", self.id);
			let hashed_new_password = Self::hash_password(new_password)?;
			// TODO:  Update the data field with history perhaps?
			sqlx::query(
				r#"
					UPDATE accounts_locals
					SET password_hash = $2, updated_at = now()
					WHERE removed_at IS NULL AND id = $1
					RETURNING 1;
				"#,
			)
			.bind(self.id)
			.bind(hashed_new_password)
			.fetch_one(conn)
			.await?;
			info!("Changed password for: {}", self.id);
			Ok(())
		} else {
			info!(
				"Removing password to make the account unable to be logged in to: {}",
				self.id
			);
			// TODO:  Update the data field with history perhaps?
			sqlx::query(
				r#"
					UPDATE accounts_locals
					SET password_hash = NULL, updated_at = now()
					WHERE removed_at IS NULL AND id = $1
					RETURNING 1;
				"#,
			)
			.bind(self.id)
			.fetch_one(conn)
			.await?;
			info!("Password removed: {}", self.id);
			Ok(())
		}
	}
}

pub struct Accounts {}

#[derive(Debug, thiserror::Error)]
pub enum AccountsError {
	#[error("given login name does not follow an allowed format: {0}")]
	InvalidLoginName(String),
	#[error("account already exists")]
	AccountAlreadyExists,
	#[error("invalid login or password")]
	InvalidLoginOrPassword,
	#[error("database error")]
	DatabaseError(#[source] sqlx::Error),
}

impl Accounts {
	fn is_valid_name(login: &str) -> Result<(), AccountsError> {
		if login.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
			Ok(())
		} else {
			Err(AccountsError::InvalidLoginName(login.to_owned()))
		}
	}

	async fn create_new_base_account_single(
		conn: &mut DbTransaction<'_>,
	) -> Result<Uuid, AccountsError> {
		Ok(
			sqlx::query_scalar::<_, Uuid>("INSERT INTO accounts DEFAULT VALUES RETURNING id;")
				.fetch_one(conn)
				.await
				.map_err(AccountsError::DatabaseError)?,
		)
	}

	async fn create_new_base_account(conn: &mut DbTransaction<'_>) -> Result<Uuid, AccountsError> {
		if let Ok(result) = Self::create_new_base_account_single(conn).await {
			Ok(result)
		} else {
			warn!("Actually got a UUIDv4 collision on `create_new_base_account`!");
			// Retry just once in the amazingly unlikely case there's a UUID collision
			Self::create_new_base_account_single(conn).await
		}
	}

	pub async fn create_account(
		conn: &mut DbTransaction<'_>,
		login: &str,
	) -> Result<Account, AccountsError> {
		info!("Creating account: {}", login);
		Self::is_valid_name(login)?;
		let id = Self::create_new_base_account(conn).await?;
		let _result =
			sqlx::query("INSERT INTO accounts_locals (id, login) VALUES ($1, $2) RETURNING id;")
				.bind(id)
				.bind(login)
				.fetch_one(conn)
				.await
				.map_err(|_| AccountsError::AccountAlreadyExists)?;
		info!("Created account: {}", login);
		Ok(Account::new(id, Some(login.to_owned())))
	}

	pub async fn login_account(
		conn: &mut DbTransaction<'_>,
		login: &str,
		password: &str,
	) -> Result<Account, AccountsError> {
		let (id, password_hash) = sqlx::query_as::<_, (Uuid, String)>(
			"SELECT id, password_hash FROM accounts_locals WHERE removed_at IS NULL AND login = $1",
		)
		.bind(login)
		.fetch_one(conn)
		.await
		.map_err(|_| AccountsError::InvalidLoginOrPassword)?;
		let existing_password_hash =
			PasswordHash::new(&password_hash).map_err(|_| AccountsError::InvalidLoginOrPassword)?;
		Account::password_hash_matches(&existing_password_hash, password)
			.map_err(|_| AccountsError::InvalidLoginOrPassword)?;
		Ok(Account::new(id, Some(login.to_owned())))
	}

	pub async fn login_session(
		conn: &mut DbTransaction<'_>,
		login: &str,
		password: &str,
		valid_duration: Duration,
	) -> Result<AccountSession, AccountsError> {
		let account = Self::login_account(conn, login, password).await?;
		let valid_until = OffsetDateTime::now_utc() + valid_duration;
		let session = sqlx::query_scalar(
			"INSERT INTO accounts_sessions (id, valid_until) VALUES ($1, $2) RETURNING token;",
		)
		.bind(account.id)
		.bind(valid_until)
		.fetch_one(conn)
		.await
		.map_err(AccountsError::DatabaseError)?;
		Ok(AccountSession {
			id: account.id,
			token: session,
		})
	}
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountSession {
	id: Uuid,
	token: Uuid,
}

impl AccountSession {
	pub fn id(&self) -> Uuid {
		self.id
	}

	pub fn token(&self) -> Uuid {
		self.token
	}

	pub async fn validate(&self, conn: &mut DbTransaction<'_>) -> anyhow::Result<()> {
		sqlx::query(
			"SELECT 1 FROM accounts_sessions WHERE id = &1 AND token = $2 AND valid_until <= now()",
		)
		.bind(self.id)
		.bind(self.token)
		.execute(conn)
		.await?;
		Ok(())
	}
}

impl Display for AccountSession {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.write_fmt(format_args!("{}|{}", self.id, self.token))
	}
}

impl FromStr for AccountSession {
	type Err = uuid::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (user_id_str, session_id_str) = s
			.split_once('|')
			.ok_or_else(|| Uuid::from_str(s).unwrap_err())?;
		let id = Uuid::from_str(user_id_str)?;
		let session = Uuid::from_str(session_id_str)?;
		Ok(Self { id, token: session })
	}
}

const MIGRATIONS: Migrations = Migrations::new(
	"Accounts",
	&[
		Migration::new("Create accounts table")
			.up(r#"
				CREATE EXTENSION IF NOT EXISTS "pgcrypto";
				CREATE TABLE accounts (
					id uuid NOT NULL DEFAULT gen_random_uuid(),
					data jsonb,
					inserted_at timestamp without time zone NOT NULL DEFAULT now(),
					CONSTRAINT accounts_pkey PRIMARY KEY (id)
				) WITH ( OIDS=FALSE );
				"#)
			.down(
				r#"
				DROP TABLE accounts;
				"#,
			),
		Migration::new("Create accounts_locals table")
			.up(r#"
				CREATE TABLE accounts_locals (
					id uuid NOT NULL,
					login text NOT NULL,
					password_hash text,
					data jsonb,
					inserted_at timestamp without time zone NOT NULL DEFAULT now(),
					updated_at timestamp without time zone NOT NULL DEFAULT now(),
					removed_at timestamp without time zone,
					CONSTRAINT accounts_local_pkey PRIMARY KEY (id, updated_at),
					CONSTRAINT accounts_local_id_fkey FOREIGN KEY (id) REFERENCES accounts (id) MATCH SIMPLE ON DELETE RESTRICT
				) WITH ( OIDS=FALSE );
				CREATE UNIQUE INDEX accounts_locals_id_index ON accounts_locals USING btree (id) WHERE removed_at IS NULL;
				CREATE UNIQUE INDEX accounts_locals_login_lower_index ON accounts_locals USING btree (lower(login) COLLATE pg_catalog."default") WHERE removed_at IS NULL;
				"#)
			.down(
				r#"
				DROP INDEX accounts_locals_login_lower_index;
				DROP INDEX accounts_locals_id_index;
				DROP TABLE accounts_locals;
				"#,
			),
		Migration::new("Create accounts_sessions table").up(r#"
				CREATE TABLE accounts_sessions (
					id uuid NOT NULL,
					token uuid NOT NULL,
					inserted_at timestamp without time zone NOT NULL DEFAULT now(),
					valid_until timestamp without time zone NOT NULL,
					CONSTRAINT accounts_sessions_pkey PRIMARY KEY (id, token),
					CONSTRAINT accounts_sessions_id_fkey FOREIGN KEY (id) REFERENCES accounts (id) MATCH SIMPLE ON UPDATE CASCADE ON DELETE CASCADE
				) WITH ( OIDS=FALSE );
				"#).down(r#"
				DROP TABLE accounts_sessions;
				"#)
	],
);
