//! This is an appender that rolls a file on launch and then delegates to another appender

use log4rs::append::Append;
use log4rs::config::{Deserialize, Deserializers};
use log4rs::encode::{Encode, EncoderConfig, Write};
use serde_value::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use tracing::log::{Level, Record};

#[derive(Clone, Eq, PartialEq, Hash, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheAppenderConfig {
	name: String,
	count: usize,
	encoder: Option<EncoderConfig>,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
struct Appender {
	kind: String,
	config: Value,
}

impl<'de> serde::Deserialize<'de> for Appender {
	fn deserialize<D>(d: D) -> Result<Appender, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let mut map = BTreeMap::<Value, Value>::deserialize(d)?;

		let kind = match map.remove(&Value::String("kind".to_owned())) {
			Some(kind) => kind.deserialize_into().map_err(|e| e.to_error())?,
			None => return Err(serde::de::Error::missing_field("kind")),
		};

		Ok(Appender {
			kind,
			config: Value::Map(map),
		})
	}
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct CacheAppenderDeserializer;

impl Deserialize for CacheAppenderDeserializer {
	type Trait = dyn Append;

	type Config = CacheAppenderConfig;

	fn deserialize(
		&self,
		config: CacheAppenderConfig,
		deserializers: &Deserializers,
	) -> anyhow::Result<Box<dyn Append>> {
		let cache = Cache::get_or_create(config.name);
		{
			let mut cache = cache.write().expect("poisoned lock");
			cache.reserve_exact(config.count);
		}
		let encoder: Box<dyn Encode> = if let Some(encoder) = config.encoder {
			deserializers.deserialize(&encoder.kind, encoder.config)?
		} else {
			Box::new(log4rs::encode::pattern::PatternEncoder::default())
		};
		Ok(Box::new(CacheAppender {
			cache,
			count: config.count,
			encoder,
		}))
	}
}

#[derive(Debug)]
pub struct CacheAppender {
	cache: Arc<RwLock<VecDeque<CachedLogRecord>>>,
	count: usize,
	encoder: Box<dyn Encode>,
}

impl Append for CacheAppender {
	fn append(&self, record: &Record) -> anyhow::Result<()> {
		let mut cache = self.cache.write().expect("poisoned lock");
		let mut last = None;
		while cache.len() >= self.count {
			last = cache.pop_front();
		}
		let mut last = last.unwrap_or_default();
		last.0 = record.level();
		last.1.clear();
		self.encoder
			.encode(&mut StringEncoder(&mut last.1), record)?;
		cache.push_back(last);
		Ok(())
	}

	fn flush(&self) {}
}

struct StringEncoder<'a>(&'a mut String);
impl<'a> std::io::Write for StringEncoder<'a> {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		self.0.push_str(&String::from_utf8_lossy(buf));
		Ok(buf.len())
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}
impl<'a> Write for StringEncoder<'a> {}

#[derive(Debug)]
pub struct CachedLogRecord(Level, String);

impl Default for CachedLogRecord {
	fn default() -> Self {
		Self(Level::max(), String::new())
	}
}

impl CachedLogRecord {
	pub fn level(&self) -> Level {
		self.0
	}

	pub fn msg(&self) -> &str {
		&self.1
	}
}

#[derive(Default)]
pub struct Cache {
	map: RwLock<HashMap<String, Arc<RwLock<VecDeque<CachedLogRecord>>>>>,
}

lazy_static::lazy_static! {
	static ref CACHE_MAP: Cache = Cache::default();
}

impl Cache {
	pub fn get_or_create(name: String) -> Arc<RwLock<VecDeque<CachedLogRecord>>> {
		let mut cache_map = CACHE_MAP.map.write().expect("poisoned lock");
		cache_map
			.entry(name)
			.or_insert_with(|| Arc::new(RwLock::new(VecDeque::new())))
			.clone()
	}
}
