use ron::extensions::Extensions;
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use structopt::StructOpt;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::*;

#[typetag::serde(tag = "_type")]
pub trait SystemTask {
	fn spawn(&self, self_name: &str, system: &System) -> anyhow::Result<Option<JoinHandle<()>>>;
}

#[derive(Clone, Debug, StructOpt)]
#[structopt()]
pub struct SystemArgs {
	#[structopt(long, short)]
	/// Run with a terminal user interface, otherwise it runs as a daemon.
	tui: bool,

	#[structopt(long, short)]
	/// Run as a daemon, only prints logs, forces the `TUI` to be disabled.
	daemon: bool,

	#[structopt(long, short, default_value = ".")]
	/// Path to the configuration files and every related external file
	root_dir: PathBuf,
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SystemConfig {
	system_tasks: HashMap<String, Box<dyn SystemTask>>,
}

impl Default for SystemConfig {
	fn default() -> Self {
		let mut system_tasks: HashMap<String, Box<dyn SystemTask>> = Default::default();
		system_tasks.insert(
			"TUI".to_owned(),
			Box::new(crate::system_tasks::tui::TUI::default()),
		);
		system_tasks.insert(
			"Daemon".to_owned(),
			Box::new(crate::system_tasks::daemon::Daemon::new(false)),
		);
		Self { system_tasks }
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
			std::fs::write(path, ron)?;
			Ok(None)
		}
	}
}

pub struct System {
	config: SystemConfig,
	/// These tasks are ones that keep the system running, useful for daemon's, TUI's, network, etc.
	pub system_tasks: Arc<crossbeam::queue::SegQueue<JoinHandle<()>>>,
	pub tui: bool,
	pub daemon: bool,
	pub quit: broadcast::Sender<()>,
	pub registered_modules: Arc<dashmap::DashMap<String, ()>>,
}

impl System {
	pub fn new() -> anyhow::Result<Option<Self>> {
		Self::new_with_args(SystemArgs::from_args())
	}

	pub fn new_with_args(args: SystemArgs) -> anyhow::Result<Option<Self>> {
		anyhow::ensure!(
			!args.tui || !args.daemon,
			"cannot be both a TUI and a daemon"
		);
		let config_path = args.root_dir.join("overbot.ron");
		if let Some(config) = SystemConfig::get_or_create(&config_path)? {
			Self::new_with_config(args.root_dir.clone(), config).map(|mut system| {
				system.enable_tui(args.tui);
				system.enable_daemon(args.daemon);
				Some(system)
			})
		} else {
			println!(
				"No configuration found, wrote out new configuration file at: {:?}, please make edits as necessary and launch again",
				config_path
			);
			Ok(None)
		}
	}

	pub fn new_with_config(root_path: PathBuf, config: SystemConfig) -> anyhow::Result<Self> {
		crate::logger::init_logging(Some(&root_path))?;
		info!("Initialized logging system");
		let (quit, _recv_quit) = broadcast::channel(1);
		Ok(System {
			config,
			system_tasks: Default::default(),
			daemon: false,
			tui: false,
			quit,
			registered_modules: Default::default(),
		})
	}

	pub fn enable_tui(&mut self, tui: bool) -> &mut Self {
		self.tui = tui;
		self
	}

	pub fn enable_daemon(&mut self, daemon: bool) -> &mut Self {
		self.daemon = daemon;
		self
	}

	pub async fn startup_systems(&mut self) -> anyhow::Result<()> {
		anyhow::ensure!(self.system_tasks.is_empty(), "systems already exist");
		for (system_task_name, system_task) in &self.config.system_tasks {
			info!("Processing system task: {}", system_task_name);
			if let Some(handle) = system_task.spawn(system_task_name, self)? {
				self.system_tasks.push(handle);
			}
		}
		info!("System startup complete");
		Ok(())
	}

	#[tracing::instrument(name = "System RunLoop", skip(self))]
	pub async fn run_loop(&mut self) -> anyhow::Result<()> {
		while let Some(task) = self.system_tasks.pop() {
			task.await?;
		}
		Ok(())
	}

	pub async fn run(&mut self) -> anyhow::Result<()> {
		self.startup_systems().await?;
		info!(
			"Running system, {} system tasks upon startup",
			self.system_tasks.len()
		);
		self.run_loop().await?;
		info!("System running completed, no system tasks remaining");
		Ok(())
	}
}
