//! File-based logging.
//!
//! Never stdout: it corrupts the alternate screen, and a TUI that scribbles on
//! itself when something goes wrong is worse than one that says nothing.

use anyhow::Result;
use tracing_subscriber::EnvFilter;

/// Returns a guard that must be held for the process lifetime — dropping it
/// stops the writer thread and silently loses buffered lines.
pub fn init(verbose: bool) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let dir = crate::paths::log_dir()?;
    std::fs::create_dir_all(&dir)?;

    let appender = tracing_appender::rolling::never(&dir, "staramp.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let default = if verbose {
        "staramp=debug"
    } else {
        "staramp=info"
    };
    let filter = EnvFilter::try_from_env("STARAMP_LOG").unwrap_or_else(|_| EnvFilter::new(default));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .init();

    Ok(guard)
}
