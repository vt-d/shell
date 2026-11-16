//! Network connectivity, from NetworkManager.
//!
//! `nmcli monitor` prints whenever a device or connection changes state, which triggers a
//! fresh query. Wi-Fi signal strength changes without such an event, so while on Wi-Fi
//! it is also re-read every half minute.

use std::{
    cell::{Cell, RefCell},
    error::Error,
    rc::Rc,
    time::Duration,
};

use calloop::{
    LoopHandle,
    timer::{TimeoutAction, Timer},
};

use crate::process;

const SIGNAL_REFRESH: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Network {
    #[default]
    Disconnected,
    /// A device is connecting (or getting an address).
    Connecting,
    Wired,
    /// Signal strength, 0–100.
    Wifi {
        signal: u8,
    },
}

impl Network {
    /// The current state, preferring a wired connection over Wi-Fi.
    pub fn query() -> Result<Self, Box<dyn Error>> {
        let devices = process::output("nmcli", &["-t", "-f", "TYPE,STATE", "device"])?;
        let mut state = Self::Disconnected;
        for line in devices.lines() {
            let Some((kind, device_state)) = line.split_once(':') else {
                continue;
            };
            let connected = device_state.starts_with("connected");
            let connecting = device_state.starts_with("connecting");
            match kind {
                "ethernet" if connected => return Ok(Self::Wired),
                "wifi" if connected => state = Self::Wifi { signal: wifi_signal() },
                "ethernet" | "wifi" if connecting && state == Self::Disconnected => {
                    state = Self::Connecting
                }
                _ => {}
            }
        }
        Ok(state)
    }
}

/// The signal of the access point in use, from NetworkManager's last scan.
fn wifi_signal() -> u8 {
    let list = process::output(
        "nmcli",
        &["-t", "-f", "ACTIVE,SIGNAL", "device", "wifi", "list", "--rescan", "no"],
    )
    .unwrap_or_default();
    list.lines()
        .find_map(|line| line.strip_prefix("yes:"))
        .and_then(|signal| signal.parse().ok())
        .unwrap_or(0)
}

/// Calls `on_change` with the loop's data and the new state whenever connectivity
/// changes.
pub fn watch_network<D: 'static>(
    handle: &LoopHandle<'static, D>,
    on_change: impl FnMut(&mut D, Network) + 'static,
) -> Result<(), Box<dyn Error>> {
    let on_change = Rc::new(RefCell::new(on_change));
    let last = Rc::new(Cell::new(Network::query().unwrap_or_default()));
    let refresh = {
        let (on_change, last) = (on_change.clone(), last.clone());
        move |data: &mut D| match Network::query() {
            Ok(state) if state != last.get() => {
                last.set(state);
                (on_change.borrow_mut())(data, state);
            }
            Ok(_) => {}
            Err(err) => eprintln!("network: {err}"),
        }
    };

    let on_event = refresh.clone();
    process::on_output(handle, "network", "nmcli", &["monitor"], move |data, _| on_event(data))?;

    let on_timer = refresh;
    handle
        .insert_source(Timer::from_duration(SIGNAL_REFRESH), move |_, _, data| {
            if matches!(last.get(), Network::Wifi { .. }) {
                on_timer(data);
            }
            TimeoutAction::ToDuration(SIGNAL_REFRESH)
        })
        .map_err(|e| e.error)?;
    Ok(())
}
