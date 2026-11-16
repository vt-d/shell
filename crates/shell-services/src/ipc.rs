//! Talking to the running shell from outside it: a line sent to a Unix socket, and a
//! line back.
//!
//! The shell [listens](listen) on `$XDG_RUNTIME_DIR/shell-$WAYLAND_DISPLAY.sock` (or the
//! path in `$SHELL_SOCKET`); `shell <command>` [sends](send) it the command and prints
//! what it answers. One shell listens per display: a second one leaves the socket alone.

use std::{
    env,
    error::Error,
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    time::Duration,
};

use calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic};

/// How long a peer gets to send its line or take its answer, so that a stuck one can't
/// stall the shell.
const PATIENCE: Duration = Duration::from_millis(200);
/// The longest message taken.
const LIMIT: u64 = 4096;

fn socket_path() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = env::var_os("SHELL_SOCKET").filter(|path| !path.is_empty()) {
        return Ok(path.into());
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let display = env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_owned());
    // A display may be named by a path.
    let display = display.rsplit('/').next().unwrap_or_default();
    Ok(PathBuf::from(runtime).join(format!("shell-{display}.sock")))
}

/// Sends `message` to the running shell and returns its answer.
pub fn send(message: &str) -> Result<String, Box<dyn Error>> {
    let path = socket_path()?;
    let mut socket = UnixStream::connect(&path)
        .map_err(|err| format!("no shell is listening on {}: {err}", path.display()))?;
    socket.write_all(format!("{message}\n").as_bytes())?;
    let mut answer = String::new();
    socket.read_to_string(&mut answer)?;
    Ok(answer.trim().to_owned())
}

/// Calls `on_message` with the loop's data and each message [sent](send), and answers
/// with what it returns.
pub fn listen<D: 'static>(
    handle: &LoopHandle<'static, D>,
    mut on_message: impl FnMut(&mut D, &str) -> String + 'static,
) -> Result<(), Box<dyn Error>> {
    let path = socket_path()?;
    if UnixStream::connect(&path).is_ok() {
        return Err(format!("another shell is listening on {}", path.display()).into());
    }
    // Nobody answers: it's what a shell that is gone left behind.
    match fs::remove_file(&path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err.into()),
        _ => {}
    }
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;

    let source = Generic::new(listener, Interest::READ, Mode::Level);
    handle
        .insert_source(source, move |_, listener, data| {
            loop {
                let socket = match listener.accept() {
                    Ok((socket, _)) => socket,
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Err(err),
                };
                if let Err(err) = answer(socket, |message| on_message(data, message)) {
                    eprintln!("ipc: {err}");
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|err| err.error)?;
    Ok(())
}

/// Reads one line from `socket` and writes back what `on_message` makes of it.
fn answer(mut socket: UnixStream, on_message: impl FnOnce(&str) -> String) -> io::Result<()> {
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(PATIENCE))?;
    socket.set_write_timeout(Some(PATIENCE))?;
    let mut message = String::new();
    BufReader::new((&socket).take(LIMIT)).read_line(&mut message)?;
    let answer = on_message(message.trim());
    socket.write_all(format!("{answer}\n").as_bytes())
}
