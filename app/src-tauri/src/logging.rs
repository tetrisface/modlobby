//! Where the logs go. Both halves of the app write to one rolling file beside
//! the settings, so a crash, a rebuild or a restart leaves something to read:
//! Rust through `tracing`, the webview through [`crate::commands::log_message`].
//!
//! The console keeps the human-readable format; the file is JSON per line, so
//! it can be searched with `rg` and sliced with `jq` without a log service.
//!
//! Started before the settings are read, so that reading them is logged too;
//! their filter takes over once they are ([`Logging::set_filter`]).

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry, fmt, reload};

/// Dropping this stops the background writer, so it lives as long as the app.
pub struct Logging {
	_guard: Option<WorkerGuard>,
	filter: reload::Handle<EnvFilter, Registry>,
}

/// How many daily files are kept. A fortnight covers "it broke last week"
/// and stops the directory growing for as long as the app is installed.
pub const KEEP_DAYS: usize = 14;

/// Logs to the console and to `logs/` under `dir`, filtered by the setting's
/// default until [`Logging::set_filter`]. A logs directory that cannot be
/// made costs the file, not the app: the console is left.
pub fn start(dir: &Path) -> Logging {
	let first = settings::model::Logging::default().filter;
	let (filter, handle) = reload::Layer::new(env_filter(&first));
	let logs = dir.join("logs");
	let appender = rolling::RollingFileAppender::builder()
		.rotation(rolling::Rotation::DAILY)
		.filename_prefix("modlobby.jsonl")
		.max_log_files(KEEP_DAYS)
		.build(&logs);
	let (file, guard, failed) = match appender {
		Ok(appender) => {
			let (writer, guard) = tracing_appender::non_blocking(appender);
			let layer = fmt::layer()
				.json()
				.with_current_span(false)
				.with_writer(writer);
			(Some(layer), Some(guard), None)
		}
		Err(err) => (None, None, Some(err)),
	};
	tracing_subscriber::registry()
		.with(filter)
		.with(fmt::layer().with_target(true))
		.with(file)
		.init();
	match failed {
		None => tracing::info!(dir = %logs.display(), "logging to file"),
		Some(err) => tracing::warn!(%err, dir = %logs.display(), "no log file; the console only"),
	}
	Logging {
		_guard: guard,
		filter: handle,
	}
}

impl Logging {
	/// The `logging.filter` setting. `RUST_LOG` still wins when set.
	pub fn set_filter(&self, filter: &str) {
		if let Err(err) = self.filter.reload(env_filter(filter)) {
			tracing::warn!(%err, "the log filter was not changed");
		}
	}
}

fn env_filter(fallback: &str) -> EnvFilter {
	EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(fallback))
}
