//! Hyprland's IPC: workspace state, and notification when it changes.
//!
//! Hyprland has two sockets in `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/`:
//! `.socket.sock` answers one hyprctl-style request per connection, and `.socket2.sock`
//! streams events as `EVENT>>DATA` lines. Rather than track state from events, each
//! relevant event triggers a fresh query, so the result is always what Hyprland reports.

use std::{
    collections::HashMap,
    env,
    error::Error,
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
};

use calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic};
use serde_json::Value;

/// Events after which the set of workspaces, which are occupied, or which is active may
/// differ.
const WORKSPACE_EVENTS: &[&str] = &[
    "workspace",
    "workspacev2",
    "focusedmon",
    "focusedmonv2",
    "createworkspace",
    "createworkspacev2",
    "destroyworkspace",
    "destroyworkspacev2",
    "moveworkspace",
    "moveworkspacev2",
    "renameworkspace",
    "openwindow",
    "closewindow",
    "movewindow",
    "movewindowv2",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    pub windows: u64,
    /// Its windows, in the order they opened. Focus doesn't reorder them.
    pub clients: Vec<Client>,
}

/// A window.
#[derive(Clone, Debug, PartialEq)]
pub struct Client {
    /// Hyprland's id for it, unique while it's open.
    pub address: String,
    /// Its app id.
    pub class: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workspaces {
    /// Regular workspaces (special ones are left out), by id.
    pub all: Vec<Workspace>,
    /// The focused monitor's active workspace.
    pub active: i64,
}

impl Workspaces {
    pub fn query() -> Result<Self, Box<dyn Error>> {
        let list: Value = serde_json::from_str(&request("j/workspaces")?)?;
        let active: Value = serde_json::from_str(&request("j/activeworkspace")?)?;
        let clients: Value = serde_json::from_str(&request("j/clients")?)?;

        // Hyprland lists clients in the order they were created.
        let mut by_workspace: HashMap<i64, Vec<Client>> = HashMap::new();
        for client in clients.as_array().into_iter().flatten() {
            let (Some(workspace), Some(address), Some(class)) = (
                client["workspace"]["id"].as_i64(),
                client["address"].as_str(),
                client["class"].as_str(),
            ) else {
                continue;
            };
            if !client["mapped"].as_bool().unwrap_or(true) {
                continue;
            }
            by_workspace
                .entry(workspace)
                .or_default()
                .push(Client { address: address.to_owned(), class: class.to_owned() });
        }

        let mut all: Vec<Workspace> = list
            .as_array()
            .ok_or("workspaces: expected an array")?
            .iter()
            .filter_map(|ws| {
                let id = ws.get("id")?.as_i64()?;
                Some(Workspace {
                    id,
                    name: ws.get("name")?.as_str()?.to_owned(),
                    windows: ws.get("windows")?.as_u64()?,
                    clients: by_workspace.remove(&id).unwrap_or_default(),
                })
            })
            // Special (scratchpad) workspaces have negative ids.
            .filter(|ws| ws.id > 0)
            .collect();
        all.sort_by_key(|ws| ws.id);
        let active = active.get("id").and_then(Value::as_i64).ok_or("activeworkspace: no id")?;
        Ok(Self { all, active })
    }
}

/// Switches to workspace `id`.
pub fn focus_workspace(id: i64) -> Result<(), Box<dyn Error>> {
    // Hyprland configured in Lua takes its dispatchers as Lua; before that, by name.
    let reply = request(&format!("dispatch hl.dsp.focus({{ workspace = \"{id}\" }})"))?;
    if reply.trim() == "ok" {
        return Ok(());
    }
    match request(&format!("dispatch workspace {id}"))?.trim() {
        "ok" => Ok(()),
        reply => Err(reply.into()),
    }
}

/// Has the compositor run `command` with a shell, as it runs what its own binds start.
pub fn exec(command: &str) -> Result<(), Box<dyn Error>> {
    let reply = request(&format!("dispatch hl.dsp.exec_cmd({})", lua_string(command)))?;
    if reply.trim() == "ok" {
        return Ok(());
    }
    match request(&format!("dispatch exec {command}"))?.trim() {
        "ok" => Ok(()),
        reply => Err(reply.into()),
    }
}

/// `text` as a Lua string literal.
fn lua_string(text: &str) -> String {
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => literal.extend(['\\', c]),
            '\n' => literal.push_str("\\n"),
            c => literal.push(c),
        }
    }
    literal.push('"');
    literal
}

/// Calls `on_change` with the loop's data and fresh workspace state whenever it changes.
pub fn watch_workspaces<D: 'static>(
    handle: &LoopHandle<'static, D>,
    mut on_change: impl FnMut(&mut D, Workspaces) + 'static,
) -> Result<(), Box<dyn Error>> {
    let events = UnixStream::connect(socket_dir()?.join(".socket2.sock"))?;
    events.set_nonblocking(true)?;
    let mut pending = Vec::new();
    let mut last = Workspaces::query().ok();

    let source = Generic::new(events, Interest::READ, Mode::Level);
    handle
        .insert_source(source, move |_, events, data| {
            // SAFETY: the stream is only read from, never replaced or dropped.
            let events = unsafe { events.get_mut() };
            let mut buffer = [0; 4096];
            loop {
                match events.read(&mut buffer) {
                    Ok(0) => {
                        eprintln!("hyprland: event socket closed");
                        return Ok(PostAction::Remove);
                    }
                    Ok(n) => pending.extend_from_slice(&buffer[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }

            // Only complete lines; a partial one waits for the rest.
            let complete = pending.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
            let relevant = pending[..complete]
                .split(|&b| b == b'\n')
                .filter_map(|line| line.split(|&b| b == b'>').next())
                .any(|event| WORKSPACE_EVENTS.iter().any(|e| e.as_bytes() == event));
            pending.drain(..complete);

            // A burst of events (moving a window fires several) costs a single query.
            if relevant {
                match Workspaces::query() {
                    Ok(state) if last.as_ref() != Some(&state) => {
                        last = Some(state.clone());
                        on_change(data, state);
                    }
                    Ok(_) => {}
                    Err(err) => eprintln!("hyprland: {err}"),
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| e.error)?;
    Ok(())
}

fn socket_dir() -> Result<PathBuf, Box<dyn Error>> {
    let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let instance =
        env::var_os("HYPRLAND_INSTANCE_SIGNATURE").ok_or("not running under Hyprland")?;
    Ok(PathBuf::from(runtime).join("hypr").join(instance))
}

/// One request on the request socket. Hyprland serves it synchronously and stalls while
/// a connection stays open, so each request gets its own short-lived connection.
fn request(command: &str) -> Result<String, Box<dyn Error>> {
    let mut socket = UnixStream::connect(socket_dir()?.join(".socket.sock"))?;
    socket.write_all(command.as_bytes())?;
    let mut reply = String::new();
    socket.read_to_string(&mut reply)?;
    Ok(reply)
}
