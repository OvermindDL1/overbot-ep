use crate::accounts::Accounts;
use crate::database::{DbPool, DbTransaction};
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::outcome::try_outcome;
use rocket::outcome::IntoOutcome;
use rocket::request::{FromRequest, Outcome};
use rocket::tokio::time::Duration;
use rocket::Request;
use std::convert::TryInto;
use std::fmt::{Debug, Display, Formatter};
use std::marker::PhantomData;
use std::str::FromStr;
use tracing::*;
use uuid::Uuid;

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

fn get_fake_login_session() -> AuthUserSessionIds {
	AuthUserSessionIds {
		user_id: Uuid::from_u128(0),
		session_id: Uuid::from_u128(1),
	}
}

impl<'r> AuthControl<'r> {
	pub fn is_logged_in(&self) -> bool {
		self.auth_session.is_some()
	}

	pub fn login(&self, cookies: &CookieJar, username: &str, password: &str, age_secs: u64) {
		warn!(
			"OhNoesAUsernamePasswordSoInsecure: `{}` `{}`",
			username, password
		);
		let user_session = get_fake_login_session();
		let mut cookie = Cookie::named(COOKIE_USER_SESSION);
		cookie.set_http_only(true);
		cookie.set_max_age(Some(Duration::from_secs(age_secs).try_into().unwrap()));
		cookie.set_value(user_session.to_string());
		// cookie.set_secure(true);
		cookie.set_same_site(SameSite::Strict);
		cookies.add_private(cookie);
	}

	pub async fn register(
		&self,
		conn: &mut DbTransaction<'_>,
		username: &str,
		password: &str,
	) -> anyhow::Result<()> {
		let account = Accounts::create_account(conn, username).await?;
		account.set_password(conn, None, Some(password)).await?;
		Ok(())
	}
}

#[derive(Debug)]
pub struct AuthUserSessionIds {
	pub user_id: Uuid,
	pub session_id: Uuid,
}

impl Display for AuthUserSessionIds {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.write_fmt(format_args!("{}|{}", self.user_id, self.session_id))
	}
}

impl FromStr for AuthUserSessionIds {
	type Err = uuid::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (user_id_str, session_id_str) = s
			.split_once('|')
			.ok_or_else(|| Uuid::from_str(s).unwrap_err())?;
		let user_id = Uuid::from_str(user_id_str)?;
		let session_id = Uuid::from_str(session_id_str)?;
		Ok(Self {
			user_id,
			session_id,
		})
	}
}

#[derive(Debug)]
pub struct AuthSession<'r> {
	_phantom: PhantomData<&'r ()>,
	pub user_session_ids: AuthUserSessionIds,
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
		let user_session_ids = try_outcome!(AuthUserSessionIds::from_str(user_session_string)
			.map_err(|_| ())
			.into_outcome(Status::Unauthorized));
		Outcome::Success(Self {
			_phantom: Default::default(),
			user_session_ids,
		})
	}
}
