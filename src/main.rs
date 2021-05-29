use crate::system::System;

mod logger;
mod system;
mod system_tasks;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	if let Some(mut system) = System::new()? {
		system.run().await?;
	}
	Ok(())
}
