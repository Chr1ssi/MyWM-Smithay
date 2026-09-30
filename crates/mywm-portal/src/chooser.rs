//! Asks the compositor what the user wants to share (`v1 choose-source`, see `mywm-ipc`).
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
};

use mywm_ipc::{Chosen, SourceKinds};

fn socket_path() -> Option<PathBuf> {
    std::env::var_os("MYWM_SOCKET")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("mywm.sock")))
}

/// Show the compositor's chooser and wait for the answer (can take as long as the user needs).
pub fn choose(kinds: SourceKinds) -> Result<Chosen, String> {
    let path = socket_path().ok_or("no MYWM_SOCKET and no XDG_RUNTIME_DIR")?;
    let mut stream = UnixStream::connect(&path).map_err(|e| format!("cannot reach the compositor at {}: {e}", path.display()))?;
    let word = match kinds {
        SourceKinds::Monitor => "monitor",
        SourceKinds::Window => "window",
        SourceKinds::Both => "both",
    };
    stream.write_all(format!("v1 choose-source {word}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(stream).lines();
    let mut accepted = false;
    for line in &mut lines {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(chosen) = Chosen::parse(&line) {
            return Ok(chosen);
        }
        match line.as_str() {
            "v1 ok" => accepted = true,
            "v1 error invalid-command" if !accepted => return Err("the compositor is busy (another selection is open)".into()),
            _ => {}
        }
    }
    Err("the compositor closed the connection".into())
}
