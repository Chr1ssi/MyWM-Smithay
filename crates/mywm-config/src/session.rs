//! Session helpers shared with the River-based MyWM: the screen locker command, the
//! `--lock` client that asks the compositor to lock, and the idle daemon arguments.
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{net::UnixStream, process::CommandExt},
    process::Command,
    time::{Duration, Instant},
};

use crate::{Config, IdleConfig, keys::Result};

/// `swaylock` with the colors of the configured palette.
pub fn locker(config: &Config) -> Command {
    let mut command = Command::new("swaylock");
    command.args(["--config", "/dev/null", "--show-keyboard-layout", "--indicator-idle-visible"]);
    let a = &config.appearance;
    let hex = |color: crate::HexColor| color.css()[1..].to_string();
    for (flag, color) in [
        ("--color", a.background),
        ("--inside-color", a.surface),
        ("--ring-color", a.active_border),
        ("--text-color", a.text),
        ("--key-hl-color", a.text),
        ("--bs-hl-color", a.muted_text),
        ("--layout-bg-color", a.surface),
        ("--layout-text-color", a.text),
        ("--layout-border-color", a.inactive_border),
    ] {
        command.args([flag.to_string(), hex(color)]);
    }
    for state in ["clear", "caps-lock", "ver", "wrong"] {
        command.args([format!("--inside-{state}-color"), hex(a.surface)]);
        command.args([format!("--ring-{state}-color"), hex(a.active_border)]);
        command.args([format!("--text-{state}-color"), hex(a.text)]);
    }
    command.arg("--line-uses-ring");
    command
}

/// Ask the running compositor over `$MYWM_SOCKET` to lock and wait until it confirms.
/// Without a socket (another compositor), run the locker directly.
pub fn lock_and_wait(config: &Config) -> Result<()> {
    let Some(path) = std::env::var_os("MYWM_SOCKET") else {
        let status = locker(config).status()?;
        return status.success().then_some(()).ok_or_else(|| "swaylock exited unsuccessfully".into());
    };
    let mut socket = UnixStream::connect(path)?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    socket.write_all(b"v1 lock\n")?;
    let mut reader = BufReader::new(socket);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("the compositor did not confirm the session lock")?;
        reader.get_ref().set_read_timeout(Some(remaining))?;
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err("the compositor disconnected before confirming the session lock".into());
        }
        if line.trim() == "v1 locked 1" {
            return Ok(());
        }
        if line.starts_with("v1 error") {
            return Err(line.into());
        }
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Arguments for `swayidle`: lock after the first timeout, then lock and power the monitors off.
pub fn idle_args(config: &IdleConfig, executable: &str) -> Vec<String> {
    let lock = format!("{} --lock", quote(executable));
    let on = "wlopm --on '*'";
    let mut args = vec!["-w".into(), "-C".into(), "/dev/null".into()];
    if config.lock_after_seconds > 0 {
        args.extend(["timeout".into(), config.lock_after_seconds.to_string(), lock.clone()]);
    }
    if config.monitor_off_after_seconds > 0 {
        args.extend([
            "timeout".into(),
            config.monitor_off_after_seconds.to_string(),
            format!("{lock} && wlopm --off '*'"),
            "resume".into(),
            on.into(),
        ]);
    }
    args.extend(["before-sleep".into(), lock.clone(), "after-resume".into(), on.into(), "lock".into(), lock]);
    args
}

/// The `swayidle` command for this configuration, or `None` when both timeouts are off.
pub fn idle_command(config: &IdleConfig, executable: &str) -> Option<Command> {
    (config.lock_after_seconds > 0 || config.monitor_off_after_seconds > 0).then(|| {
        let mut command = Command::new("swayidle");
        command.args(idle_args(config, executable));
        command
    })
}

/// Replace this process with `swayidle` (the `--idle` mode of the River-based MyWM).
pub fn exec_idle(config: &IdleConfig) -> Result<()> {
    let executable = std::env::current_exe()?;
    let executable = executable.to_str().ok_or("executable path is not UTF-8")?;
    let mut command = Command::new("swayidle");
    command.args(idle_args(config, executable));
    Err(command.exec().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_commands_quote_paths_and_wait_for_lock_before_power_off() {
        let args = idle_args(&IdleConfig::default(), "/tmp/a'b/my wm");
        assert!(args.contains(&"'/tmp/a'\\''b/my wm' --lock && wlopm --off '*'".into()));
        assert!(args.contains(&"before-sleep".into()));
        let disabled = IdleConfig { lock_after_seconds: 0, monitor_off_after_seconds: 0 };
        disabled.validate().unwrap();
        assert!(!idle_args(&disabled, "mywm").contains(&"timeout".into()));
        assert!(idle_command(&disabled, "mywm").is_none());
        assert!(idle_command(&IdleConfig::default(), "mywm").is_some());
    }

    #[test]
    fn locker_uses_the_palette() {
        let args: Vec<_> = locker(&Config::default())
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let position = args.iter().position(|a| a == "--ring-color").unwrap();
        assert_eq!(args[position + 1], "89b4fa");
        assert!(args.contains(&"--line-uses-ring".to_string()));
    }
}
