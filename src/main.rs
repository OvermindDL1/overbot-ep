use crate::system::System;

pub mod dash_type_map;
pub mod logger;
pub mod system;
pub mod system_tasks;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	if let Some(mut system) = System::new()? {
		system.run().await?;
	}
	Ok(())
}
