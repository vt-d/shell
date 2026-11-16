//! Running commands: long-running ones that print when something changes (`nmcli
//! monitor`, `pw-dump --monitor`), and one-off ones, in the foreground or background.

use std::{
    error::Error,
    io::{self, Read},
    os::fd::AsRawFd,
    process::{Command, Stdio},
};

use calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic};

/// Runs `program` and calls `on_output` with what it printed whenever it prints, once
/// per burst of output (which may end mid-line). If the program exits, the watch ends
/// and `name` is reported on stderr.
pub(crate) fn on_output<D: 'static>(
    handle: &LoopHandle<'static, D>,
    name: &'static str,
    program: &'static str,
    args: &[&str],
    mut on_output: impl FnMut(&mut D, &[u8]) + 'static,
) -> Result<(), Box<dyn Error>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    set_nonblocking(&stdout)?;

    let source = Generic::new(stdout, Interest::READ, Mode::Level);
    let mut output = Vec::new();
    handle
        .insert_source(source, move |_, stdout, data| {
            // SAFETY: the pipe is only read from, never replaced or dropped.
            let stdout = unsafe { stdout.get_mut() };
            let mut buffer = [0; 16384];
            output.clear();
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => {
                        let _ = child.wait();
                        eprintln!("{name}: {program} exited");
                        return Ok(PostAction::Remove);
                    }
                    Ok(n) => output.extend_from_slice(&buffer[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
            if !output.is_empty() {
                on_output(data, &output);
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| e.error)?;
    Ok(())
}

/// Runs `program` in the background and, once it exits, calls `on_done` with what it
/// printed, or an error if it failed. The loop carries on meanwhile, so it suits slow
/// commands such as network requests.
pub(crate) fn run<D: 'static>(
    handle: &LoopHandle<'static, D>,
    program: &'static str,
    args: &[&str],
    on_done: impl FnOnce(&mut D, Result<Vec<u8>, Box<dyn Error>>) + 'static,
) -> Result<(), Box<dyn Error>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    set_nonblocking(&stdout)?;

    let source = Generic::new(stdout, Interest::READ, Mode::Level);
    let mut output = Vec::new();
    let mut on_done = Some(on_done);
    handle
        .insert_source(source, move |_, stdout, data| {
            // SAFETY: the pipe is only read from, never replaced or dropped.
            let stdout = unsafe { stdout.get_mut() };
            let mut buffer = [0; 16384];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => output.extend_from_slice(&buffer[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        return Ok(PostAction::Continue);
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
            // End of output: it has exited, or is about to.
            let result = match child.wait() {
                Ok(status) if status.success() => Ok(std::mem::take(&mut output)),
                Ok(status) => Err(format!("{program} failed: {status}").into()),
                Err(e) => Err(e.into()),
            };
            if let Some(on_done) = on_done.take() {
                on_done(data, result);
            }
            Ok(PostAction::Remove)
        })
        .map_err(|e| e.error)?;
    Ok(())
}

fn set_nonblocking(fd: &impl AsRawFd) -> io::Result<()> {
    let fd = fd.as_raw_fd();
    // SAFETY: fcntl on a file descriptor we own, with flags it accepts.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Runs `program` to completion and returns what it printed.
pub(crate) fn output(program: &str, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = Command::new(program).args(args).stderr(Stdio::null()).output()?;
    if !output.status.success() {
        return Err(format!("{program} failed: {}", output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}
