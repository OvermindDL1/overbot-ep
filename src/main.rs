mod system;
use crate::system::System;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	System::new().run().await
}
