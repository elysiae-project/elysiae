use anyhow::Result;
use log::{LevelFilter, Log, Metadata, Record};

/// Writes launcher logs to stderr using the configured severity filter.
struct StderrLogger;

impl Log for StderrLogger {
    /// Checks whether a message is within the active log level.
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    /// Prints an enabled message with its severity and logging target.
    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    /// Completes the logger interface; this logger keeps no buffered messages.
    fn flush(&self) {}
}

/// Parses the requested log level, defaulting to info when none is provided.
fn level(value: Option<&str>) -> Result<LevelFilter> {
    value
        .unwrap_or("info")
        .parse()
        .map_err(|_| anyhow::anyhow!("ELYSIAE_LOG must be off, error, warn, info, debug, or trace"))
}

/// Installs the stderr logger and reads its level from ELYSIAE_LOG.
pub fn init() -> Result<()> {
    static LOGGER: StderrLogger = StderrLogger;
    let filter = level(std::env::var("ELYSIAE_LOG").ok().as_deref())?;
    log::set_logger(&LOGGER).map_err(|error| anyhow::anyhow!("{error}"))?;
    log::set_max_level(filter);
    Ok(())
}
