use crate::dash_type_map::DashTypeMap;
use crate::database::{ConnectionLock, DbPool};
use ron::extensions::Extensions;
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use structopt::StructOpt;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::*;

pub trait QuitOnError {
	fn quit_on_err(self, quit: &broadcast::Sender<()>) -> Self;
}

impl<S, E> QuitOnError for Result<S, E> {
	fn quit_on_err(self, quit: &broadcast::Sender<()>) -> Self {
		if self.is_err() {
			error!("Error occurred, sending quit signal");
			let _ = quit.send(());
		}
		self
	}
}

#[typetag::serde()]
pub trait SystemPlugin {
	fn name(&self) -> Cow<str> {
		Cow::Borrowed(std::any::type_name::<Self>())
	}

	fn spawn(&self, system: &System) -> Option<JoinHandle<anyhow::Result<()>>>;
}

#[derive(Clone, Debug, StructOpt)]
#[structopt()]
pub struct SystemArgs {
	#[structopt(long, short = "m")]
	/// Override the run mode from the configuration file
	run_mode: Option<RunMode>,

	#[structopt(long, short, default_value = ".")]
	/// Path to the configuration files and every related external file
	root_dir: PathBuf,
}

// mod typetag_plugin_vec {
// 	use crate::system::SystemPlugin;
// 	use serde::de::value::*;
// 	use serde::de::*;
// 	use serde::*;
// 	use std::fmt;
// 	use std::marker::PhantomData;
//
// 	pub fn deserialize<'de, D: Deserializer<'de>>(
// 		deserializer: D,
// 	) -> Result<Vec<Box<dyn SystemPlugin>>, D::Error> {
// 		struct TraitObjectsVisitor<T>(PhantomData<T>);
//
// 		impl<'de, T> Visitor<'de> for TraitObjectsVisitor<T>
// 		where
// 			T: Deserialize<'de>,
// 		{
// 			type Value = Vec<T>;
//
// 			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
// 				f.write_str("a map in which each TypeName:Value pair specifies a trait object")
// 			}
//
// 			fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
// 			where
// 				M: MapAccess<'de>,
// 			{
// 				let mut trait_objects = Vec::new();
// 				while let Some(key) = map.next_key()? {
// 					trait_objects.push(T::deserialize(MapAccessDeserializer::new(MapEntry {
// 						key: Some(key),
// 						value: &mut map,
// 					}))?);
// 				}
// 				Ok(trait_objects)
// 			}
// 		}
//
// 		struct MapEntry<M> {
// 			key: Option<String>,
// 			value: M,
// 		}
//
// 		impl<'de, M> MapAccess<'de> for MapEntry<M>
// 		where
// 			M: MapAccess<'de>,
// 		{
// 			type Error = M::Error;
//
// 			fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, M::Error>
// 			where
// 				K: DeserializeSeed<'de>,
// 			{
// 				self.key
// 					.take()
// 					.map(|key| seed.deserialize(key.into_deserializer()))
// 					.transpose()
// 			}
//
// 			fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, M::Error>
// 			where
// 				V: DeserializeSeed<'de>,
// 			{
// 				self.value.next_value_seed(seed)
// 			}
// 		}
//
// 		let visitor = TraitObjectsVisitor(PhantomData);
// 		deserializer.deserialize_map(visitor)
// 	}
//
// 	pub fn serialize<S: Serializer>(
// 		plugins: &Vec<Box<dyn SystemPlugin>>,
// 		serializer: S,
// 	) -> Result<S::Ok, S::Error> {
// 		serializer.serialize_map()
// 		todo!()
// 	}
// }

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub enum RunMode {
	Foreground,
	Daemon,
	TUI,
}

impl FromStr for RunMode {
	type Err = &'static str;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.trim().to_lowercase().as_str() {
			"foreground" => Ok(RunMode::Foreground),
			"daemon" => Ok(RunMode::Daemon),
			"tui" => Ok(RunMode::TUI),
			_ => Err("unsupported run-mode, valid values:  Foreground, Daemon, TUI"),
		}
	}
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SystemConfig {
	run_mode: RunMode,
	database: crate::database::DatabaseConfig,
	web: Option<crate::web::WebConfig>,
	// #[serde(with = "typetag_plugin_vec")]
	// plugins: Vec<Box<dyn SystemPlugin>>,
}

const PASSWORD_CHARS: &[u8] =
	"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_".as_bytes();
fn gen_new_password(len: usize) -> String {
	use rand::prelude::*;
	PASSWORD_CHARS
		.choose_multiple(&mut thread_rng(), len)
		.cloned()
		.map(Into::<char>::into)
		.collect::<String>()
}

impl Default for SystemConfig {
	fn default() -> Self {
		Self {
			run_mode: RunMode::Foreground,
			database: crate::database::DatabaseConfig::new_embedded(
				5,
				"./",
				5433,
				"postgres",
				gen_new_password(32),
				true,
				Duration::from_secs(5),
				None,
			),
			web: Some(crate::web::WebConfig::default()),
			// plugins: vec![
			// 	Box::new(crate::system_tasks::daemon::Daemon::new(true)),
			// 	Box::new(crate::system_tasks::postgres::Postgres::new_embedded(
			// 		true,
			// 		5,
			// 		"./data",
			// 		5433,
			// 		"postgres",
			// 		gen_new_password(32),
			// 		true,
			// 		Duration::from_secs(5),
			// 		None,
			// 	)),
			// 	Box::new(crate::system_tasks::tui::TUI::new(false)),
			// 	Box::new(crate::system_tasks::web_ui_rocket::WebUiRocket::new(true)),
			// 	Box::new(crate::system_tasks::irc::IRC::new(true)),
			// ],
		}
	}
}

impl SystemConfig {
	fn get_or_create(path: &Path) -> anyhow::Result<Option<Self>> {
		if path.is_file() {
			let ron = std::fs::read_to_string(path)?;
			let config = ron::from_str(&ron)?;
			Ok(Some(config))
		} else {
			let config = SystemConfig::default();
			let ron = ron::ser::to_string_pretty(
				&config,
				PrettyConfig::new()
					.with_new_line("\n".to_owned())
					.with_enumerate_arrays(true)
					.with_indentor("\t".to_owned())
					.with_extensions(Extensions::all()),
			)?;
			let mut file = std::fs::File::create(path)?;
			file.write_all(ron.as_bytes())?;
			file.write_all("\n".as_bytes())?;
			file.flush()?;
			drop(file);
			Ok(None)
		}
	}
}

pub struct System {
	config: SystemConfig,
	pub root_path: PathBuf,
	db_lock: ConnectionLock,
	pub db_pool: DbPool,
	/// These tasks are ones that keep the system running, useful for daemon's, TUI's, network, etc.
	/// These tasks should *ALWAYS* quit when `quit` is broadcast on or the system may not ever die.
	pub system_tasks: Arc<crossbeam::queue::SegQueue<JoinHandle<anyhow::Result<()>>>>,
	// pub tui: bool,
	// pub daemon: bool,
	pub quit: broadcast::Sender<()>,
	pub registered_data: Arc<DashTypeMap>,
}

impl System {
	pub async fn run() -> anyhow::Result<()> {
		Self::run_with_args(SystemArgs::from_args()).await
	}

	pub async fn run_with_args(args: SystemArgs) -> anyhow::Result<()> {
		let config_path = args.root_dir.join("overbot.ron");
		if let Some(mut config) = SystemConfig::get_or_create(&config_path)? {
			if let Some(run_mode) = args.run_mode {
				config.run_mode = run_mode
			}
			Self::run_with_config(args.root_dir.clone(), config).await
		} else {
			println!(
				"No configuration found, wrote out new configuration file at: {:?}, please make edits as necessary and launch again",
				config_path
			);
			Ok(())
		}
	}

	pub async fn run_with_config(root_path: PathBuf, config: SystemConfig) -> anyhow::Result<()> {
		crate::logger::init_logging(Some(&root_path))?;
		info!("Initialized logging system");
		let (quit, _recv_quit) = broadcast::channel(1);
		let (db_lock, db_pool) = config.database.create_database_pool().await?;
		let mut system = System {
			root_path,
			config,
			db_lock,
			db_pool,
			system_tasks: Default::default(),
			quit,
			registered_data: Default::default(),
		};
		system.startup_systems().await?;
		info!(
			"Running system, {} system tasks upon startup",
			system.system_tasks.len()
		);
		system.run_loop().await?;
		info!("System running completed, no system tasks remaining, shutting down database");
		system.db_pool.close().await;
		drop(system.db_pool);
		drop(system.db_lock);
		info!("Database shut down, exiting");
		Ok(())
	}

	pub async fn startup_systems(&mut self) -> anyhow::Result<()> {
		anyhow::ensure!(self.system_tasks.is_empty(), "systems already exist");
		if let Some(web) = &self.config.web {
			self.system_tasks.push(web.spawn(self));
		}
		match self.config.run_mode {
			RunMode::Foreground => {
				if let Some(handle) = crate::system_tasks::daemon::Daemon::new(false).spawn(self) {
					self.system_tasks.push(handle);
				}
			}
			RunMode::Daemon => {
				if let Some(handle) = crate::system_tasks::daemon::Daemon::new(true).spawn(self) {
					self.system_tasks.push(handle);
				}
			}
			RunMode::TUI => {
				if let Some(handle) = crate::system_tasks::tui::TUI::new(true).spawn(self) {
					self.system_tasks.push(handle);
				}
			}
		}
		for plugin in &[&crate::system_tasks::irc::IRC::new(true)] {
			if let Some(handle) = plugin.spawn(self) {
				self.system_tasks.push(handle);
			}
		}
		// for plugin in &self.config.plugins {
		// 	info!("Processing system task: {}", plugin.name());
		// 	if let Some(handle) = plugin.spawn(self) {
		// 		self.system_tasks.push(handle);
		// 	}
		// }
		info!("System startup complete");
		Ok(())
	}

	#[tracing::instrument(name = "System RunLoop", skip(self))]
	pub async fn run_loop(&mut self) -> anyhow::Result<()> {
		while let Some(task) = self.system_tasks.pop() {
			match task.await {
				Ok(Ok(())) => (),
				Ok(Err(e)) => {
					error!("System Task returned an error result: {}", e);
				}
				Err(e) => {
					error!("System Task Join Error: {}", e);
				}
			}
		}
		Ok(())
	}
}
