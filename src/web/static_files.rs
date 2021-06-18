use rocket::http::ContentType;
use std::borrow::Cow;
use std::path::Path;

#[derive(rocket::Responder)]
pub struct StaticFile {
	pub data: Cow<'static, [u8]>,
	pub content_type: ContentType,
}

#[derive(rust_embed::RustEmbed)]
#[folder = "assets/web/dist/"]
pub struct StaticAssets;

pub struct Assets;

impl Assets {
	pub fn get(file_path: &str) -> Option<StaticFile> {
		// This can block in debug mode as it loads the file from the FS, but free in release
		let data = StaticAssets::get(file_path)?;
		let content_type =
			if let Some(extension) = Path::new(file_path).extension().and_then(|e| e.to_str()) {
				ContentType::from_extension(extension).unwrap_or(ContentType::Binary)
			} else {
				ContentType::Binary
			};
		Some(StaticFile { data, content_type })
	}
}
