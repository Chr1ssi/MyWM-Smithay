//! `splice-shell-services --login`: one login through greetd for the compositor's login screen (`--greeter`), in a
//! process of its own per attempt, like `--authenticate`. The request (`LoginRequest`) comes as JSON on stdin;
//! what greetd answers goes to stdout as `LoginEvent` lines. Like Quickshell's `Greetd` (which greeter.qml used):
//! every prompt gets the password, error messages that ask nothing are shown, and once greetd accepts, the
//! session is started with the command of the chosen `.desktop` file.
//!
//! greetd's IPC (`$GREETD_SOCK`): JSON messages, each after its length as a 32-bit integer in native byte order.
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
};

use splice_shell_protocol::{LoginEvent, LoginRequest};
use serde_json::{Value, json};

/// greetd's messages are small; anything larger is not greetd.
const MAX_MESSAGE: usize = 1 << 20;

/// Send `data` (a JSON message) and zero it: it may hold the password.
fn send(stream: &mut UnixStream, mut data: Vec<u8>) -> std::io::Result<()> {
    let written = u32::try_from(data.len())
        .map_err(std::io::Error::other)
        .and_then(|length| stream.write_all(&length.to_ne_bytes()))
        .and_then(|()| stream.write_all(&data));
    data.fill(0);
    written
}

/// The answer to a prompt, built by hand so that no copy of the password is left unzeroed.
fn password_answer(password: &str) -> std::io::Result<Vec<u8>> {
    let mut quoted = serde_json::to_vec(password)?;
    let mut data = br#"{"type":"post_auth_message_response","response":"#.to_vec();
    data.extend_from_slice(&quoted);
    data.push(b'}');
    quoted.fill(0);
    Ok(data)
}

fn receive(stream: &mut UnixStream) -> std::io::Result<Value> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_ne_bytes(length) as usize;
    if length > MAX_MESSAGE {
        return Err(std::io::Error::other("greetd's answer is too long"));
    }
    let mut data = vec![0; length];
    stream.read_exact(&mut data)?;
    Ok(serde_json::from_slice(&data)?)
}

/// Send a message (or the error of building it) and read greetd's answer.
fn exchange(stream: &mut UnixStream, data: std::io::Result<Vec<u8>>) -> Result<Value, String> {
    data.and_then(|data| send(stream, data)).and_then(|()| receive(stream)).map_err(|e| format!("greetd: {e}"))
}

fn ask(stream: &mut UnixStream, message: &Value) -> Result<Value, String> {
    exchange(stream, serde_json::to_vec(message).map_err(std::io::Error::from))
}

fn tell(event: &LoginEvent) {
    if let Ok(line) = serde_json::to_string(event) {
        println!("{line}");
    }
}

/// Log in as `request` says; every way it ends is told (the last event is `Started`, `Wrong` or `Error`).
fn login(request: &LoginRequest) -> Result<(), String> {
    let path = std::env::var_os("GREETD_SOCK").ok_or("GREETD_SOCK is not set")?;
    let mut stream = UnixStream::connect(&path).map_err(|e| format!("cannot reach greetd: {e}"))?;
    let stream = &mut stream;
    let create = json!({ "type": "create_session", "username": request.user });
    let mut answer = ask(stream, &create)?;
    // Like Quickshell: a session left over (an attempt whose process died) is cancelled and begun anew.
    if answer["type"] == "error" && answer["description"] == "a session is already being configured" {
        ask(stream, &json!({ "type": "cancel_session" }))?;
        answer = ask(stream, &create)?;
    }
    loop {
        match answer["type"].as_str() {
            Some("auth_message") => {
                let kind = answer["auth_message_type"].as_str().unwrap_or_default();
                let text = answer["auth_message"].as_str().unwrap_or_default();
                answer = match kind {
                    "secret" | "visible" => exchange(stream, password_answer(&request.password))?,
                    _ => {
                        if kind == "error" {
                            tell(&LoginEvent::Message { text: text.to_owned() });
                        }
                        ask(stream, &json!({ "type": "post_auth_message_response" }))?
                    }
                };
            }
            Some("success") => break,
            Some("error") if answer["error_type"] == "auth_error" => {
                tell(&LoginEvent::Wrong);
                // As Quickshell does; greetd may answer with an error, as the failed session is over.
                let _ = ask(stream, &json!({ "type": "cancel_session" }));
                return Ok(());
            }
            _ => {
                let description = answer["description"].as_str().unwrap_or("unexpected answer").to_owned();
                // Leave greetd ready for the next attempt; its answer does not matter any more.
                let _ = ask(stream, &json!({ "type": "cancel_session" }));
                return Err(description);
            }
        }
    }
    tell(&LoginEvent::Launching);
    let answer = ask(stream, &json!({ "type": "start_session", "cmd": request.command, "env": [] }))?;
    if answer["type"] == "success" {
        tell(&LoginEvent::Started);
        Ok(())
    } else {
        Err(answer["description"].as_str().unwrap_or("unexpected answer").to_owned())
    }
}

/// Read the request from stdin, log in and say how it went.
pub fn run() -> std::process::ExitCode {
    let mut input = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut input) {
        tracing::error!("cannot read the login: {error}");
        return std::process::ExitCode::FAILURE;
    }
    let request: Result<LoginRequest, _> = serde_json::from_slice(&input);
    // The password does not outlive the login in this process's memory (the parsed copy is wiped below).
    input.fill(0);
    let Ok(mut request) = request else {
        tracing::error!("the login request does not parse");
        return std::process::ExitCode::FAILURE;
    };
    if let Err(description) = login(&request) {
        tracing::error!("login failed: {description}");
        tell(&LoginEvent::Error { description });
    }
    // SAFETY: zero bytes are valid UTF-8.
    unsafe { request.password.as_mut_vec() }.fill(0);
    std::process::ExitCode::SUCCESS
}
