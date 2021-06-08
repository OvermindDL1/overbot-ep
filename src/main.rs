use crate::system::System;

pub mod accounts;
pub mod dash_type_map;
pub mod database;
pub mod logger;
pub mod system;
pub mod system_tasks;
pub mod web;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	System::run().await
}
