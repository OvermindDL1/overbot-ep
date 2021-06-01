pub mod cache_appender;
pub mod conditional_append_appender;
pub mod conditional_map;
pub mod launch_roll_file_appender;

use log4rs::config::runtime::ConfigErrors;
use log4rs::config::{Config, Deserializers, RawConfig};
use std::path::{Path, PathBuf};
use tracing::log::SetLoggerError;

const DEFAULT_LOGGING_DEFINITION_RON: &'static str = r#"(
	// Default values for loggers
	root: Root(
		// Default log filter level unless otherwise specified in the loggers section.
		level: Trace,
		// The appenders to enable by default from the appenders section, can be overridden or
		// added to this in the loggers section.
		appenders: ["console", "log_file", "tui_log_view"],
	),

	// List of appenders, these receive a log event and do whatever they wish to do with it.
	// Each appender has a `kind` that defines what code it runs, the rest of the key/values are
	// passed to that kind to initialize it.
	appenders: {
		// These keys are just a name of the appender to refer to in the `appenders` sections in
		// loggers.
		"console": {
			// `kind`s specify which code to run for this appender.  The `conditional_appender` can
			// be enabled or disabled at runtime by other code based on the given `id`.  If it's
			// enabled at the given time then it passes the log messages to the `appender` value,
			// which is just another appender, `kind` and all.
			"kind": "conditional_appender",
			"id": "console",
			"appender": {
				// The `console` appender code type just prints messages to the given `target` using
				// the given `encoder`
				"kind": "console",
				// Optional `target`, can be missing or `None` to use the default of "stdout".
				// Values can be:
				// * Some("stdout")
				// * Some("stderr")
				"target": Some("stderr"),
				// An encoder specifies how a given log entry is printed out, can be a `pattern`
				// encoder or a `json` encoder (json is great for making parseable log files!)
				"encoder": Some({
					// The `pattern` encoder just takes a `pattern` option that specifies how to
					// format the log entry as text.
					"kind": "pattern",
					// The `pattern` argument is a string to define how to format the log entry as
					// text.  Defaults to `"{d} {l} {t} - {m}{n}"`.
					// The format syntax uses `{`, `}`, `(`, `)`, and `\` as part of the pattern
					// syntax, so to use them directly then they should be escaped  by typing it
					// twice like `{{` or `\\` or so forth.  They can also be escaped by prefixing a
					// single character with `\` like `\{` or `\\`.
					// A formatter is a command like `{a}` or `{a(...)}` where the `...` are
					// arguments to that formatter.  If there are no arguments then there is no
					// `(..)`.
					//
					// The following formatters are currently supported. Unless otherwise stated,
					// a formatter does not accept any argument.
					//
					// * `d`, `date` - The current time. By default, the ISO 8601 format is used.
					//     A custom format may be provided in the syntax accepted by `chrono`.
					//     The timezone defaults to local, but can be specified explicitly by
					//     passing a second argument of `utc` for UTC or `local` for local time.
					//     * `{d}` - `2016-03-20T14:22:20.644420340-08:00`
					//     * `{d(%Y-%m-%d %H:%M:%S)}` - `2016-03-20 14:22:20`
					//     * `{d(%Y-%m-%d %H:%M:%S %Z)(utc)}` - `2016-03-20 22:22:20 UTC`
					// * `f`, `file` - The source file that the log message came from, or `???` if
					//     not provided.
					// * `h`, `highlight` - Styles its argument according to the log level. The
					//     style is intense red for errors, red for warnings, blue for info, and
					//     the default style for all other levels.
					//     * `{h(the level is {l})}` -
					//         <code style="color: red; font-weight: bold">the level is ERROR</code>
					// * `l`, `level` - The log level.
					// * `L`, `line` - The line that the log message came from, or `???` if not
					//     provided.
					// * `m`, `message` - The log message.
					// * `M`, `module` - The module that the log message came from, or `???` if not
					//     provided.
					// * `P`, `pid` - The current process id.
					// * `n` - A platform-specific newline.
					// * `t`, `target` - The target of the log message.
					// * `T`, `thread` - The name of the current thread.
					// * `I`, `thread_id` - The ID of the current thread.
					// * `X`, `mdc` - A value from the [MDC][MDC]. The first argument specifies
					//     the key, and the second argument specifies the default value if the
					//     key is not present in the MDC. The second argument is optional, and
					//     defaults to the empty string.
					//     * `{X(user_id)}` - `123e4567-e89b-12d3-a456-426655440000`
					//     * `{X(nonexistent_key)(no mapping)}` - `no mapping`
					// * An "unnamed" formatter simply formats its argument, applying the format
					//     specification.
					//     * `{({l} {m})}` - `INFO hello`
					//
					// # Format Specification
					//
					// The format specification determines how the output of a formatter is
					// adjusted before being returned.
					//
					// ## Fill/Alignment
					//
					// The fill and alignment values are used in conjunction with a minimum width
					// value (see below) to control the behavior when a formatter's output is less
					// than the minimum. While the default behavior is to pad the output to the
					// right with space characters (i.e. left align it), the fill value specifies
					// the character used, and the alignment value is one of:
					//
					// * `<` - Left align by appending the fill character to the formatter output
					// * `>` - Right align by prepending the fill character to the formatter
					//     output.
					//
					// ## Width
					//
					// By default, the full contents of a formatter's output will be inserted into
					// the pattern output, but both the minimum and maximum lengths can be
					// configured. Any output over the maximum length will be truncated, and
					// output under the minimum length will be padded (see above).
					//
					// # Examples
					//
					// The default pattern is `{d} {l} {t} - {m}{n}` which produces output like
					// `2016-03-20T22:22:20.644420340+00:00 INFO module::path - this is a log
					// message`.
					//
					// The pattern `{m:>10.15}` will right-align the log message to a minimum of
					// 10 bytes, filling in with space characters, and truncate output after 15
					// bytes. The message `hello` will therefore be displayed as
					// <code>     hello</code>, while the message `hello there, world!` will be
					// displayed as `hello there, wo`.
					//
					// The pattern `{({l} {m}):15.15}` will output the log level and message
					// limited to exactly 15 bytes, padding with space characters on the right if
					// necessary. The message `hello` and log level `INFO` will be displayed as
					// <code>INFO hello     </code>, while the message `hello, world!` and log
					// level `DEBUG` will be truncated to `DEBUG hello, wo`.
					//
					// [MDC]: https://crates.io/crates/log-mdc
					"pattern": "{d} [{t}:{I}:{T}] {h({l})} {M}: {m}{n}",
				}),
			},
		},
		"log_file": {
			// The `launch_roll_file` runs a Roller type upon load time, good to roll to a new log
			// file for example.
			"kind": "launch_roll_file",
			// This is the path to the file to roll.
			"path": "log/current.log",
			// This is the roller to run on launch.
			"launch_roller": {
				"kind": "fixed_window",
				"count": 5,
				"pattern": "log/previous-{}.log",
			},
			// This is the appender that is then run after the `launch_roller` is run.
			"appender": {
				// A file appender just outputs to a file.
				"kind": "file",
				// This is the path of the file to log to.
				"path": "log/current.log",
				// An encoder such as described above.  If this is set as a `json` encode kind then
				// it takes no other arguments, instead the entire log message is logged as
				// structured json.
				"encoder": {
					"kind": "pattern",
					"pattern": "{d} [{t}:{I}:{T}] {h({l})} {M}: {m}{n}",
				},
			},
		},
		"tui_log_view": {
			// This is a named cache logger, used for the TUI log view, so don't remove this if you
			// might ever use the TUI log view.
			"kind": "cache_logger",
			// Internal lookup name of this cache, the TUI log_view uses "tui_log_view".
			"name": "tui_log_view",
			// How many entries to cache
			"count": 1024,
			// Standard encoder as described above
			"encoder": {
				"kind": "pattern",
				"pattern": "{d(%H:%M:%S%.3f)} {h({l})} {M}: {m}",
			},
		},
	},

	loggers: {
		// List of loggers from module IDs.  If not specified here then it inherits from root above.
		// This first one shows all possible options.  Loggers inherit from their parents, so for
		// example if you set a logger named `tracing` and one named `tracing::span` then
		// `tracing::span` inherits `tracing`s values unless overridden.
		"tracing::span": (
			// Max Log level, can be any kind of LevelFilter: Off, Error, Warn, Info, Debug, Trace
			// Default: parent logger's level
			level: Info,
			// The list of appenders attached to the logger.  It uses the string name of an existing
			// appender as a list element.
			// Default: empty list
			appenders: [],
			// The additivity of the logger. If true, appenders attached to the logger's parent will
			// also be attached to this logger, otherwise it fully overrides
			additive: true,
		),
		"mio::poll": ( level: Info ),
		"cursive_core": ( level: Info ),
		"hyper": ( level: Info ),
		"reqwest": ( level: Info ),
		"want": ( level: Info ),
	},
)
"#;

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("Unable to create configuration directory at: {0:?}")]
	CreateDirError(PathBuf, #[source] std::io::Error),
	#[error("Unable to write missing default `log4rs.yml` file at: {0:?}")]
	UnableToWriteDefaultConfig(PathBuf, #[source] std::io::Error),
	#[error("Unable to initialize logging system from configuration file")]
	UnableToInitializeLoggingSystem(#[from] anyhow::Error),
	#[error("Unable to configure logging system")]
	ConfigFailure(#[from] ConfigErrors),
	#[error("Unable to initialize logging system from configuration")]
	ConfigurationInit(#[from] SetLoggerError),
	#[error("failed parsing configuration file in ron format")]
	RonParseFailure(#[from] ron::Error),
	#[error("failed reading file")]
	FileReadFailure(#[from] std::io::Error),
}

/// Initializes the logging system, panics on failure
pub fn init_logging(config_dir: Option<&Path>) -> Result<(), Error> {
	match config_dir {
		Some(path) => {
			if !path.is_dir() {
				std::fs::create_dir_all(&path)
					.map_err(|e| Error::CreateDirError(path.into(), e))?;
			}
			let logger_config_path = {
				let mut path: PathBuf = path.into();
				path.push("log4rs.ron");
				if !path.is_file() {
					std::fs::write(&path, DEFAULT_LOGGING_DEFINITION_RON)
						.map_err(|e| Error::UnableToWriteDefaultConfig(path.clone(), e))?;
				}
				path
			};
			let config = config_from_ron_file(&logger_config_path, &deserializers())?;
			log4rs::init_config(config)?;
		}
		None => {
			let config = config_from_ron_string(DEFAULT_LOGGING_DEFINITION_RON, &deserializers())?;
			log4rs::init_config(config)?;
		}
	};
	Ok(())
}

fn deserializers() -> Deserializers {
	let mut deserializers = Deserializers::new();
	deserializers.insert(
		"launch_roll_file",
		launch_roll_file_appender::RollFileOnLaunchAppenderDeserializer,
	);
	deserializers.insert(
		"conditional_appender",
		conditional_append_appender::ConditionallyAppendAppenderDeserializer,
	);
	deserializers.insert("cache_logger", cache_appender::CacheAppenderDeserializer);
	deserializers
}

fn config_from_ron_file(
	ron_path: impl AsRef<Path>,
	deserializers: &Deserializers,
) -> Result<Config, Error> {
	let ron = std::fs::read_to_string(ron_path)?;
	config_from_ron_string(&ron, deserializers).map_err(Into::into)
}

fn config_from_ron_string(ron: &str, deserializers: &Deserializers) -> Result<Config, ron::Error> {
	let raw_config: RawConfig = ron::from_str(ron)?;

	let (appenders, mut errors) = raw_config.appenders_lossy(&deserializers);
	errors.handle();

	let (config, mut errors) = Config::builder()
		.appenders(appenders)
		.loggers(raw_config.loggers())
		.build_lossy(raw_config.root());

	errors.handle();

	Ok(config)
}
