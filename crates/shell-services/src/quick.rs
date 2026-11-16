//! What a quick settings panel shows and switches: the radios, do not disturb, the
//! microphone.
//!
//! Nothing here is watched. The panel [asks](query) when it opens and again after each
//! [switch], which is when these can have changed while anyone is looking.
//!
//! - Radios are read from rfkill and switched with `nmcli` (Wi-Fi, so NetworkManager
//!   knows) and `rfkill`.
//! - Do not disturb is mako's mode of that name.
//! - The microphone is PipeWire's default source, through `wpctl`.

use calloop::LoopHandle;

use crate::process;

/// Mako's mode in which notifications aren't shown.
const DND_MODE: &str = "do-not-disturb";

/// Prints one line for each thing [`State`] holds, named by its first word.
const QUERY: &str = r#"
for radio in /sys/class/rfkill/rfkill*; do
    [ -r "$radio/type" ] && echo "radio $(cat "$radio/type") $(cat "$radio/soft")"
done
echo "ssid $(nmcli -t -f TYPE,NAME connection show --active 2>/dev/null \
    | grep '^802-11-wireless:' | head -n 1 | cut -d: -f2-)"
echo "modes $(makoctl mode 2>/dev/null | tr '\n' ' ')"
echo "mic $(wpctl get-volume @DEFAULT_AUDIO_SOURCE@ 2>/dev/null)"
"#;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// Whether Wi-Fi is on; `None` if there's no Wi-Fi to turn on.
    pub wifi: Option<bool>,
    /// The network Wi-Fi is connected to, if it is.
    pub ssid: Option<String>,
    /// As `wifi`, for Bluetooth.
    pub bluetooth: Option<bool>,
    /// Whether notifications are held back.
    pub dnd: bool,
    /// Whether the microphone is muted; `None` if there's none.
    pub mic_muted: Option<bool>,
}

impl State {
    /// Whether every radio there is is off.
    pub fn airplane(&self) -> bool {
        let radios = [self.wifi, self.bluetooth];
        radios.iter().any(Option::is_some) && !radios.contains(&Some(true))
    }

    fn parse(output: &str) -> Self {
        let mut state = Self::default();
        for line in output.lines() {
            let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
            let rest = rest.trim();
            match kind {
                "radio" => {
                    let mut words = rest.split_whitespace();
                    let (Some(radio), Some(blocked)) = (words.next(), words.next()) else {
                        continue;
                    };
                    let on = blocked == "0";
                    // On if any radio of its kind is.
                    let merge =
                        |known: &mut Option<bool>| *known = Some(known.unwrap_or(false) || on);
                    match radio {
                        "wlan" => merge(&mut state.wifi),
                        "bluetooth" => merge(&mut state.bluetooth),
                        _ => {}
                    }
                }
                "ssid" if !rest.is_empty() => state.ssid = Some(rest.to_owned()),
                "modes" => state.dnd = rest.split_whitespace().any(|mode| mode == DND_MODE),
                // "Volume: 0.50" or "Volume: 0.50 [MUTED]"; nothing if there's no source.
                "mic" if rest.starts_with("Volume:") => {
                    state.mic_muted = Some(rest.contains("[MUTED]"));
                }
                _ => {}
            }
        }
        state
    }
}

/// Finds the state out in the background and calls `on_done` with it.
pub fn query<D: 'static>(
    handle: &LoopHandle<'static, D>,
    on_done: impl FnOnce(&mut D, State) + 'static,
) {
    let result = process::run(handle, "sh", &["-c", QUERY], |data, result| match result {
        Ok(output) => on_done(data, State::parse(&String::from_utf8_lossy(&output))),
        Err(err) => eprintln!("quick settings: {err}"),
    });
    if let Err(err) = result {
        eprintln!("quick settings: {err}");
    }
}

/// Something to switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Wifi(bool),
    Bluetooth(bool),
    /// On turns every radio off.
    Airplane(bool),
    DoNotDisturb(bool),
    /// The default output.
    Mute(bool),
    /// The default microphone.
    MicMute(bool),
    Suspend,
    Restart,
    PowerOff,
}

/// Does `switch` in the background and calls `on_done` once it's done, whether or not
/// it worked: the time to [`query`] again.
pub fn switch<D: 'static>(
    handle: &LoopHandle<'static, D>,
    switch: Switch,
    on_done: impl FnOnce(&mut D) + 'static,
) {
    let flag = |on: bool| if on { "1" } else { "0" };
    let (program, args): (&'static str, Vec<&str>) = match switch {
        Switch::Wifi(on) => ("nmcli", vec!["radio", "wifi", if on { "on" } else { "off" }]),
        Switch::Bluetooth(on) => {
            ("rfkill", vec![if on { "unblock" } else { "block" }, "bluetooth"])
        }
        Switch::Airplane(on) => ("rfkill", vec![if on { "block" } else { "unblock" }, "all"]),
        Switch::DoNotDisturb(on) => {
            ("makoctl", vec!["mode", if on { "-a" } else { "-r" }, DND_MODE])
        }
        Switch::Mute(on) => ("wpctl", vec!["set-mute", "@DEFAULT_AUDIO_SINK@", flag(on)]),
        Switch::MicMute(on) => ("wpctl", vec!["set-mute", "@DEFAULT_AUDIO_SOURCE@", flag(on)]),
        Switch::Suspend => ("systemctl", vec!["suspend"]),
        Switch::Restart => ("systemctl", vec!["reboot"]),
        Switch::PowerOff => ("systemctl", vec!["poweroff"]),
    };
    let result = process::run(handle, program, &args, move |data, result| {
        if let Err(err) = result {
            eprintln!("quick settings: {err}");
        }
        on_done(data);
    });
    if let Err(err) = result {
        eprintln!("quick settings: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_radios_the_network_modes_and_the_microphone() {
        let state = State::parse(
            "radio bluetooth 1\nradio wlan 0\nssid home:net\nmodes default do-not-disturb \n\
             mic Volume: 1.00 [MUTED]\n",
        );
        assert_eq!(
            state,
            State {
                wifi: Some(true),
                ssid: Some("home:net".to_owned()),
                bluetooth: Some(false),
                dnd: true,
                mic_muted: Some(true),
            }
        );
        assert!(!state.airplane());
    }

    #[test]
    fn what_is_missing_is_unknown_and_airplane_mode_needs_a_radio() {
        let state = State::parse("ssid \nmodes default \nmic \n");
        assert_eq!(state, State::default());
        assert!(!state.airplane());
        assert!(State::parse("radio wlan 1\nradio bluetooth 1\n").airplane());
    }
}
