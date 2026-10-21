//! `mywm-shell-services --authenticate`: one password check through PAM for the compositor's lock screen, in a
//! process of its own per attempt, so a module that hangs or crashes takes nothing with it but this attempt. The
//! password comes on stdin; the outcome is one word on stdout (`mywm_shell_protocol::AuthOutcome`). Like
//! Quickshell's `PamContext` (which lock.qml used): service `mywm-lock` of `$MYWM_PAM_DIR` (else `/etc/pam.d`),
//! the user this process runs as, only `pam_authenticate`, and every prompt answered with the password.
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::Read,
};

use mywm_shell_protocol::AuthOutcome;

const SERVICE: &str = "mywm-lock";

// Linux-PAM's values (security/_pam_types.h).
const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_AUTH_ERR: c_int = 7;
const PAM_MAXTRIES: c_int = 11;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type Conversation = unsafe extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Conversation,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start_confdir(service: *const c_char, user: *const c_char, conv: *const PamConv, confdir: *const c_char, handle: *mut *mut c_void) -> c_int;
    fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
}

/// Every prompt gets the password (`appdata`); messages are only read.
unsafe extern "C" fn answer(count: c_int, messages: *mut *const PamMessage, responses: *mut *mut PamResponse, appdata: *mut c_void) -> c_int {
    let Ok(count) = usize::try_from(count) else { return PAM_BUF_ERR };
    // SAFETY: PAM frees the array and the answers with free(), so they come from calloc and strdup. `messages`
    // holds `count` pointers to messages and `appdata` is the password given to pam_start below, alive until
    // pam_end.
    unsafe {
        let answers = libc::calloc(count.max(1), size_of::<PamResponse>()).cast::<PamResponse>();
        if answers.is_null() {
            return PAM_BUF_ERR;
        }
        let password = appdata.cast::<c_char>();
        for index in 0..count {
            let message = *messages.add(index);
            if !message.is_null() && matches!((*message).msg_style, PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON) {
                let copy = libc::strdup(password);
                if copy.is_null() {
                    for earlier in 0..index {
                        libc::free((*answers.add(earlier)).resp.cast());
                    }
                    libc::free(answers.cast());
                    return PAM_BUF_ERR;
                }
                (*answers.add(index)).resp = copy;
            }
        }
        *responses = answers;
    }
    PAM_SUCCESS
}

/// The name of the user this process runs as.
fn user_name() -> Option<CString> {
    let mut buffer = vec![0 as c_char; 16384];
    // SAFETY: getpwuid_r writes into `entry` and `buffer`, both valid for the sizes given; `entry` is read only
    // when it reported success.
    unsafe {
        let mut entry: libc::passwd = std::mem::zeroed();
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let result = libc::getpwuid_r(libc::getuid(), &mut entry, buffer.as_mut_ptr(), buffer.len(), &mut found);
        if result != 0 || found.is_null() || entry.pw_name.is_null() {
            return None;
        }
        Some(CStr::from_ptr(entry.pw_name).to_owned())
    }
}

fn authenticate(password: &CStr) -> AuthOutcome {
    let Some(user) = user_name() else {
        tracing::error!("cannot find out who this user is");
        return AuthOutcome::StartFailed;
    };
    let directory = std::env::var("MYWM_PAM_DIR").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/etc/pam.d".into());
    let (Ok(service), Ok(directory)) = (CString::new(SERVICE), CString::new(directory)) else { return AuthOutcome::StartFailed };
    let conversation = PamConv { conv: answer, appdata_ptr: password.as_ptr().cast_mut().cast() };
    let mut handle = std::ptr::null_mut();
    // SAFETY: every pointer is valid until pam_end; the conversation only reads the password through appdata.
    unsafe {
        let started = pam_start_confdir(service.as_ptr(), user.as_ptr(), &conversation, directory.as_ptr(), &mut handle);
        if started != PAM_SUCCESS {
            tracing::error!("cannot start PAM (code {started})");
            return AuthOutcome::StartFailed;
        }
        let result = pam_authenticate(handle, 0);
        let outcome = match result {
            PAM_SUCCESS => AuthOutcome::Success,
            PAM_AUTH_ERR => AuthOutcome::Wrong,
            PAM_MAXTRIES => AuthOutcome::MaxTries,
            other => {
                tracing::error!("PAM could not check the password (code {other})");
                AuthOutcome::TryFailed
            }
        };
        pam_end(handle, result);
        outcome
    }
}

/// Read the password from stdin, check it and say how it went. Anything but `success` keeps the session locked.
pub fn run() -> std::process::ExitCode {
    let mut input = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut input) {
        tracing::error!("cannot read the password: {error}");
        return std::process::ExitCode::FAILURE;
    }
    // A password ends at its first NUL, as it would in C.
    let end = input.iter().position(|b| *b == 0).unwrap_or(input.len());
    input.truncate(end);
    let Ok(password) = CString::new(input) else { return std::process::ExitCode::FAILURE };
    let outcome = authenticate(&password);
    // The password does not outlive the check in this process's memory.
    let mut bytes = password.into_bytes();
    bytes.fill(0);
    println!("{}", outcome.as_str());
    std::process::ExitCode::SUCCESS
}
