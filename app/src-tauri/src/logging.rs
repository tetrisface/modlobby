//! Where the logs go. Both halves of the app write to one rolling file beside
//! the settings, so a crash, a rebuild or a restart leaves something to read:
//! Rust through `tracing`, the webview through [`crate::commands::log_message`].
//!
//! The console keeps the human-readable format; the file is JSON per line, so
//! it can be searched with `rg` and sliced with `jq` without a log service.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// Dropping this stops the background writer, so it lives as long as the app.
pub struct Logging {
    _guard: WorkerGuard,
}

/// How many daily files are kept. A fortnight covers "it broke last week"
/// and stops the directory growing for as long as the app is installed.
pub const KEEP_DAYS: usize = 14;

/// `RUST_LOG` wins when set; otherwise the `logging.filter` setting does.
pub fn start(dir: &Path, filter: &str) -> Logging {
    let file = rolling::RollingFileAppender::builder()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("modlobby.jsonl")
        .max_log_files(KEEP_DAYS)
        .build(dir.join("logs"))
        .expect("a logs directory beside the settings");
    let (file, guard) = tracing_appender::non_blocking(file);

    let env = |fallback: &str| {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(fallback))
    };
    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true).with_filter(env(filter)))
        .with(
            fmt::layer()
                .json()
                .with_current_span(false)
                .with_writer(file)
                .with_filter(env(filter)),
        )
        .init();

    tracing::info!(dir = %dir.join("logs").display(), "logging to file");
    Logging { _guard: guard }
}
