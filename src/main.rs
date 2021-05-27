mod logger;
mod system;
mod tui;

use crate::system::System;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	if let Some(mut system) = System::new()? {
		system.run().await?;
	}
	Ok(())
}
