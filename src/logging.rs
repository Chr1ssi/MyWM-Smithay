//! Logging to stderr and to a file, so a session started from a greeter still leaves a trail.
//!
//! The file is `$MYWM_LOG_FILE`, else `$XDG_STATE_HOME/mywm/compositor.log`
//! (`~/.local/state/mywm/compositor.log`); the previous session's log is kept as `compositor.log.1`.
//! `MYWM_LOG_FILE=off` turns the file off. `RUST_LOG` sets the level (default `info`).
use std::{fs::File, io, path::PathBuf, sync::Mutex};

use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Info level, except for X11 protocol errors about windows that vanished (Steam does this all
/// the time and they are harmless).
const DEFAULT_FILTER: &str = "info,smithay::xwayland::xwm=warn";

fn log_path() -> Option<PathBuf> {
    match std::env::var_os("MYWM_LOG_FILE") {
        Some(value) if value == "off" => None,
        Some(value) if !value.is_empty() => Some(value.into()),
        _ => {
            let state = std::env::var_os("XDG_STATE_HOME")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
            Some(state.join("mywm/compositor.log"))
        }
    }
}

fn open_log(path: &PathBuf) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut previous = path.clone().into_os_string();
    previous.push(".1");
    let _ = std::fs::rename(path, previous);
    File::create(path)
}

/// Set up logging; returns the log file's path when there is one.
pub fn init() -> Option<PathBuf> {
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let path = log_path();
    let mut opened = None;
    let file = path.as_ref().and_then(|p| match open_log(p) {
        Ok(file) => {
            opened = Some(p.clone());
            Some(file)
        }
        Err(error) => {
            eprintln!("cannot write the log file {}: {error}", p.display());
            None
        }
    });
    let registry = tracing_subscriber::registry().with(fmt::layer().with_writer(io::stderr).with_filter(filter()));
    match file {
        Some(file) => registry
            .with(fmt::layer().with_ansi(false).with_writer(Mutex::new(file)).with_filter(filter()))
            .init(),
        None => registry.init(),
    }
    // A crash must leave a line in the log, not only on a terminal nobody watches.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture());
        default_hook(info);
    }));
    opened
}

/// Log that `what` took `took` if that is more than `limit`: at most one line a second per `what`, the
/// others are counted. For finding what holds up the compositor's thread.
pub fn note_slow(what: &'static str, took: std::time::Duration, limit: std::time::Duration) {
    use std::{sync::Mutex, time::{Duration, Instant}};
    if took < limit {
        return;
    }
    static LAST: Mutex<Vec<(&'static str, Instant, u32)>> = Mutex::new(Vec::new());
    let mut last = LAST.lock().unwrap();
    let now = Instant::now();
    let slot = match last.iter().position(|(w, _, _)| *w == what) {
        Some(index) => index,
        None => {
            last.push((what, now - Duration::from_secs(2), 0));
            last.len() - 1
        }
    };
    let (_, at, skipped) = &mut last[slot];
    if now.duration_since(*at) < Duration::from_secs(1) {
        *skipped += 1;
        return;
    }
    let more = std::mem::take(skipped);
    *at = now;
    tracing::info!("{what} took {:.1} ms{}", took.as_secs_f64() * 1000.0, if more > 0 { format!(" ({more} more slow ones since the last line)") } else { String::new() });
}
