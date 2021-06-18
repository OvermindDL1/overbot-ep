use crate::accounts::{AccountSession, Accounts};
use crate::database::{DbPool, DbTransaction};
use anyhow::Context;
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::outcome::try_outcome;
use rocket::outcome::IntoOutcome;
use rocket::request::{FromRequest, Outcome};
use rocket::Request;
use std::convert::TryInto;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::str::FromStr;
use time::Duration;
use tracing::*;

const COOKIE_USER_SESSION: &str = "user_session";

#[derive(Debug)]
pub struct AuthControl<'r> {
	_phantom: PhantomData<&'r ()>,
	pub auth_session: Option<AuthSession<'r>>,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthControl<'r> {
	type Error = ();

	async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
		let auth_session = if let Outcome::Success(auth_session) = request.guard().await {
			Some(auth_session)
		} else {
			None
		};
		let control = Self {
			_phantom: Default::default(),
			auth_session,
		};
		Outcome::Success(control)
	}
}

impl<'r> AuthControl<'r> {
	pub fn is_logged_in(&self) -> bool {
		self.auth_session.is_some()
	}

	pub async fn login(
		&self,
		db_pool: &DbPool,
		cookies: &CookieJar<'_>,
		username: &str,
		password: &str,
		age_secs: u64,
	) -> anyhow::Result<()> {
		info!("Login being attempted: {} - {}", username, age_secs);
		let age_secs: i64 = age_secs.try_into().context("invalid possible age")?;
		let mut conn = db_pool.begin().await?;
		let user_session =
			Accounts::login_session(&mut conn, username, password, Duration::seconds(age_secs))
				.await?;
		drop(conn);
		let mut cookie = Cookie::named(COOKIE_USER_SESSION);
		cookie.set_http_only(true);
		cookie.set_max_age(Some(Duration::seconds(age_secs).try_into().unwrap()));
		cookie.set_value(user_session.to_string());
		// cookie.set_secure(true);
		cookie.set_same_site(SameSite::Strict);
		cookies.add_private(cookie);
		Ok(())
	}

	pub async fn register(
		&self,
		conn: &mut DbTransaction<'_>,
		username: &str,
		password: &str,
	) -> anyhow::Result<()> {
		info!("Registration being attempted: {}", username);
		let account = Accounts::create_account(conn, username).await?;
		account.set_password(conn, None, Some(password)).await?;
		Ok(())
	}
}

#[derive(Debug)]
pub struct AuthSession<'r> {
	_phantom: PhantomData<&'r ()>,
	pub user_session: AccountSession,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthSession<'r> {
	type Error = ();

	async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
		let db_pool = try_outcome!(request
			.rocket()
			.state::<DbPool>()
			.into_outcome((Status::InternalServerError, ())));
		let user_session_cookie = try_outcome!(request
			.cookies()
			.get_private(COOKIE_USER_SESSION)
			.into_outcome((Status::Unauthorized, ())));
		let user_session_string: &str = user_session_cookie.value();
		let user_session = try_outcome!(AccountSession::from_str(user_session_string)
			.map_err(|_| ())
			.into_outcome(Status::Unauthorized));
		{
			let mut conn = try_outcome!(db_pool
				.begin()
				.await
				.map_err(|e| error!("{}", e))
				.into_outcome(Status::InternalServerError));
			try_outcome!(user_session
				.validate(&mut conn)
				.await
				.map_err(|_| ())
				.into_outcome(Status::Unauthorized));
		}
		debug!("AuthSession found in cookie: {}", user_session);
		Outcome::Success(Self {
			_phantom: Default::default(),
			user_session,
		})
	}
}
