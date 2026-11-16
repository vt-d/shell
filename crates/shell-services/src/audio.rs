//! The default output's volume, from PipeWire (via WirePlumber's `wpctl`).
//!
//! `pw-dump --monitor` prints every PipeWire object that changes. Volume lives on nodes
//! (and their devices' routes) and the default output in metadata, so only changes to
//! those trigger a fresh query. Anything else, clients above all, is ignored: every
//! `wpctl` run is a client coming and going, and re-querying on those would loop.

use std::{collections::HashMap, error::Error};

use calloop::LoopHandle;
use serde_json::Value;

use crate::process;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Volume {
    /// 0 is silent, 1 full; above 1 is amplified.
    pub level: f32,
    pub muted: bool,
}

impl Volume {
    pub fn query() -> Result<Self, Box<dyn Error>> {
        // "Volume: 0.50" or "Volume: 0.50 [MUTED]".
        let reply = process::output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
        let mut words = reply.split_whitespace().skip(1);
        let level = words.next().ok_or("wpctl: no volume")?.parse()?;
        Ok(Self { level, muted: words.any(|word| word == "[MUTED]") })
    }
}

/// Turns the default output up (or, if `change` is negative, down) by `change` of full
/// volume, to no more than full, in the background. The new volume comes back through
/// [`watch_volume`].
pub fn change_volume<D: 'static>(handle: &LoopHandle<'static, D>, change: f32) {
    let percent = (change.abs() * 100.0).round();
    if percent < 1.0 {
        return;
    }
    let step = format!("{percent}%{}", if change < 0.0 { '-' } else { '+' });
    let args = ["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &step];
    let result = process::run(handle, "wpctl", &args, |_, result| {
        if let Err(err) = result {
            eprintln!("audio: {err}");
        }
    });
    if let Err(err) = result {
        eprintln!("audio: {err}");
    }
}

/// Sets the default output's volume to `level` (0–1), in the background. It comes back
/// through [`watch_volume`].
pub fn set_volume<D: 'static>(handle: &LoopHandle<'static, D>, level: f32) {
    let level = format!("{:.2}", level.clamp(0.0, 1.0));
    let args = ["set-volume", "@DEFAULT_AUDIO_SINK@", &level];
    let result = process::run(handle, "wpctl", &args, |_, result| {
        if let Err(err) = result {
            eprintln!("audio: {err}");
        }
    });
    if let Err(err) = result {
        eprintln!("audio: {err}");
    }
}

/// Calls `on_change` with the loop's data and the new volume whenever it changes.
pub fn watch_volume<D: 'static>(
    handle: &LoopHandle<'static, D>,
    mut on_change: impl FnMut(&mut D, Volume) + 'static,
) -> Result<(), Box<dyn Error>> {
    let mut last = Volume::query().ok();
    let mut changes = Changes::default();
    process::on_output(
        handle,
        "audio",
        "pw-dump",
        &["--monitor", "--no-colors"],
        move |data, output| {
            if !changes.feed(output) {
                return;
            }
            match Volume::query() {
                Ok(volume) if Some(volume) != last => {
                    last = Some(volume);
                    on_change(data, volume);
                }
                Ok(_) => {}
                Err(err) => eprintln!("audio: {err}"),
            }
        },
    )
}

/// Reads `pw-dump --monitor`'s stream of JSON arrays of changed objects.
#[derive(Default)]
struct Changes {
    /// Output not yet parsed: the start of an array that hasn't all arrived.
    pending: Vec<u8>,
    /// Whether each live object could affect the volume, by id. Removals name only the id.
    relevant: HashMap<u64, bool>,
}

impl Changes {
    /// Takes more output and returns whether any object that could affect the volume
    /// changed.
    fn feed(&mut self, output: &[u8]) -> bool {
        self.pending.extend_from_slice(output);
        let mut changed = false;
        let mut consumed = 0;
        let mut arrays = serde_json::Deserializer::from_slice(&self.pending).into_iter::<Value>();
        loop {
            match arrays.next() {
                Some(Ok(Value::Array(objects))) => {
                    for object in &objects {
                        changed |= track(&mut self.relevant, object);
                    }
                    consumed = arrays.byte_offset();
                }
                Some(Ok(_)) => consumed = arrays.byte_offset(),
                Some(Err(err)) if err.is_eof() => break,
                Some(Err(err)) => {
                    // Lost track of the stream; re-query to be safe.
                    eprintln!("audio: unexpected pw-dump output: {err}");
                    consumed = self.pending.len();
                    changed = true;
                    break;
                }
                None => break,
            }
        }
        self.pending.drain(..consumed);
        changed
    }
}

/// Records an object's change and returns whether it could affect the volume.
fn track(relevant_ids: &mut HashMap<u64, bool>, object: &Value) -> bool {
    let Some(id) = object["id"].as_u64() else {
        return false;
    };
    match object["type"].as_str() {
        Some(kind) => {
            let kind = kind.strip_prefix("PipeWire:Interface:").unwrap_or(kind);
            let relevant = matches!(kind, "Node" | "Device" | "Metadata");
            relevant_ids.insert(id, relevant);
            relevant
        }
        // `{"id": 42, "info": null}`: removed.
        None => relevant_ids.remove(&id).unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_volume_related_objects_count_as_changes() {
        let mut changes = Changes::default();
        let client = br#"[{"id": 7, "type": "PipeWire:Interface:Client", "info": {}}]"#;
        let node = br#"[{"id": 9, "type": "PipeWire:Interface:Node", "info": {}}]"#;
        assert!(!changes.feed(client));
        assert!(changes.feed(node));
        assert!(!changes.feed(br#"[{"id": 7, "info": null}]"#));
        assert!(changes.feed(br#"[{"id": 9, "info": null}]"#));
    }

    #[test]
    fn arrays_may_arrive_in_pieces() {
        let mut changes = Changes::default();
        let node = br#"[{"id": 9, "type": "PipeWire:Interface:Node", "info": {}}]"#;
        let (start, end) = node.split_at(20);
        assert!(!changes.feed(start));
        assert!(changes.feed(end));
        assert!(changes.pending.is_empty());
    }
}
