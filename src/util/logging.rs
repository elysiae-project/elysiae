use anyhow::Result;
use log::{LevelFilter, Log, Metadata, Record};

struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

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

    fn flush(&self) {}
}

fn level(value: Option<&str>) -> Result<LevelFilter> {
    value
        .unwrap_or("info")
        .parse()
        .map_err(|_| anyhow::anyhow!("ELYSIAE_LOG must be off, error, warn, info, debug, or trace"))
}

pub fn init() -> Result<()> {
    static LOGGER: StderrLogger = StderrLogger;
    let filter = level(std::env::var("ELYSIAE_LOG").ok().as_deref())?;
    log::set_logger(&LOGGER).map_err(|error| anyhow::anyhow!("{error}"))?;
    log::set_max_level(filter);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_is_explicit_and_validated() {
        assert_eq!(level(None).unwrap(), LevelFilter::Info);
        assert_eq!(level(Some("debug")).unwrap(), LevelFilter::Debug);
        assert_eq!(level(Some("off")).unwrap(), LevelFilter::Off);
        assert!(level(Some("verbose")).is_err());
    }
}
