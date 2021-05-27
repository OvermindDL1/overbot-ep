use cursive::event::EventTrigger;
use cursive::event::Key::Esc;
use cursive::CursiveRunnable;
use structopt::StructOpt;
use tokio::sync::broadcast;
use tokio::task::{spawn_blocking, JoinHandle};

#[derive(StructOpt, Debug)]
#[structopt()]
pub struct SystemArgs {
	#[structopt(long)]
	tui: bool,
}

pub struct System {
	args: SystemArgs,
	system_tasks: Vec<JoinHandle<()>>,
	quit: broadcast::Sender<()>,
}

impl System {
	pub fn new() -> Self {
		Self::new_with_args(SystemArgs::from_args())
	}

	pub fn new_with_args(args: SystemArgs) -> Self {
		let (quit, _recv_quit) = broadcast::channel(1);
		System {
			args,
			system_tasks: Vec::new(),
			quit,
		}
	}

	pub async fn startup_system(&mut self) -> anyhow::Result<()> {
		Ok(())
	}

	pub async fn run(&mut self) -> anyhow::Result<()> {
		self.startup_system().await?;
		if self.args.tui {
			let quit = self.quit.clone();
			let mut on_quit = self.quit.subscribe();
			self.system_tasks.push(spawn_blocking(move || {
				let mut siv = cursive::default();
				siv.set_fps(1);
				siv.set_on_post_event(EventTrigger::any(), move |siv| {
					use broadcast::error::TryRecvError;
					match on_quit.try_recv() {
						Ok(()) => siv.quit(),
						Err(TryRecvError::Empty) => {}
						Err(TryRecvError::Closed) => siv.quit(),
						Err(TryRecvError::Lagged(_)) => siv.quit(),
					}
				});
				siv.add_global_callback(Esc, move |_siv| {
					let _ = quit.send(());
				});
				siv.run();
			}));
		}
		while let Some(task) = self.system_tasks.pop() {
			task.await?;
		}
		Ok(())
	}
}
