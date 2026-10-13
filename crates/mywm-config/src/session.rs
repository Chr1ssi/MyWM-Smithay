//! Session helpers: the screen locker command, the `--lock` client that asks the
//! compositor to lock, and the idle daemon arguments.
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{net::UnixStream, process::CommandExt},
    process::Command,
    time::{Duration, Instant},
};

use crate::{Config, IdleConfig, keys::Result};

/// The lock screen of mywm-shell (`lock.qml`) with the theme's colors. It unlocks through PAM (service `mywm-lock`).
pub fn locker(config: &Config) -> Command {
    let mut command = Command::new("qs");
    command.arg("--path").arg(crate::shell_qml("lock.qml")).envs(config.theme_env());
    command
}

/// Ask the running compositor over `$MYWM_SOCKET` to lock and wait until it confirms.
/// Without a socket (another compositor), run the locker directly.
pub fn lock_and_wait(config: &Config) -> Result<()> {
    let Some(path) = std::env::var_os("MYWM_SOCKET") else {
        let status = locker(config).status()?;
        return status.success().then_some(()).ok_or_else(|| "the screen locker exited unsuccessfully".into());
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

/// Replace this process with `swayidle` (the `--idle` mode).
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
    fn locker_runs_the_shell_lock_screen_with_the_palette() {
        let locker = locker(&Config::default());
        let args: Vec<_> = locker.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(locker.get_program(), "qs");
        assert!(args[1].ends_with("lock.qml"), "{args:?}");
        let accent = locker.get_envs().find(|(name, _)| *name == "MYWM_COLOR_ACCENT").and_then(|(_, value)| value);
        assert_eq!(accent.map(|v| v.to_string_lossy().into_owned()), Some("#89b4fa".into()));
    }
}
