//! Clean shutdown (removing the bar socket) on SIGTERM and SIGINT.
//!
//! A signal handler writes to a pipe that the event loop watches. Reading the signals through
//! signalfd instead (calloop's `Signals`) needs them blocked, and a blocked mask is inherited by
//! every program the compositor starts (Xwayland, the session script, the bar, apps), which then
//! ignore SIGTERM and Ctrl+C. A handler is reset to the default by `exec`; nothing leaks.
use std::{
    io,
    os::fd::{FromRawFd, OwnedFd},
    sync::atomic::{AtomicI32, Ordering},
};

use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic};

use crate::State;

/// Write end of the wake-up pipe, for the signal handler.
static WAKE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(_: libc::c_int) {
    // Only async-signal-safe calls here; `write` may change errno, which the interrupted code may read.
    // SAFETY: plain syscalls on a descriptor that stays open for the life of the process.
    unsafe {
        let errno = *libc::__errno_location();
        let fd = WAKE.load(Ordering::Relaxed);
        if fd >= 0 {
            let byte = 1u8;
            libc::write(fd, (&raw const byte).cast(), 1);
        }
        *libc::__errno_location() = errno;
    }
}

/// Stop the event loop on SIGTERM or SIGINT.
pub fn install(handle: &LoopHandle<'static, State>) -> io::Result<()> {
    let mut fds = [0; 2];
    // SAFETY: `fds` has room for both descriptors.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: freshly created and owned by nobody else. The write end stays open on purpose: the
    // handler may run until the process ends.
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    WAKE.store(fds[1], Ordering::Relaxed);
    handle
        .insert_source(Generic::new(read, Interest::READ, Mode::Level), |_, _, state| {
            state.loop_signal.stop();
            Ok(PostAction::Remove)
        })
        .map_err(|e| io::Error::other(e.error))?;
    for signal in [libc::SIGTERM, libc::SIGINT] {
        // SAFETY: a zeroed sigaction with an empty mask is valid; the handler is async-signal-safe.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_signal as *const () as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}
