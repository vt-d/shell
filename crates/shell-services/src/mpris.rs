//! Media players, over MPRIS: what each is playing, and the means to control it.
//!
//! Every MPRIS player owns a D-Bus name under `org.mpris.MediaPlayer2`. `busctl monitor`
//! reports when one appears or goes, changes a property (its track, whether it's
//! playing, its volume...) or seeks, which triggers a fresh query of them all, so the
//! result is always what the players report. Players don't announce their position as it
//! advances; it is worked out from where it was when last asked ([`Player::position`]).
//!
//! Album art comes as a URL. Whatever it points at is copied (or, if remote, downloaded
//! with `curl`) into `$XDG_CACHE_HOME/shell/art`, under a name unique to its contents:
//! players such as Firefox rewrite one file for every track, and images are cached by
//! their path.

use std::{
    cell::RefCell,
    collections::HashSet,
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

use calloop::LoopHandle;
use serde_json::Value;

use crate::process;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// How long a player gets to answer, in seconds, before it's left out: the loop waits.
const TIMEOUT: &str = "--timeout=1";
/// Art not used for this long is deleted when the shell starts.
const ART_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Browsers: they report whatever a page plays, video included, so they come last.
const BROWSERS: &[&str] = &[
    "firefox",
    "librewolf",
    "zen",
    "chromium",
    "chrome",
    "brave",
    "vivaldi",
    "opera",
    "edge",
    "plasma-browser-integration",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    Off,
    /// The whole playlist.
    All,
    /// The current track.
    One,
}

impl Repeat {
    /// The next mode in the order a repeat button steps through.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Off => "None",
            Self::All => "Playlist",
            Self::One => "Track",
        }
    }
}

/// A media player and what it's playing.
#[derive(Clone, Debug)]
pub struct Player {
    /// Its D-Bus name, which identifies it.
    pub bus: String,
    /// What it calls itself: "Spotify", "Mozilla Firefox".
    pub identity: String,
    /// The name of its `.desktop` file, for finding its icon; may be empty.
    pub desktop_entry: String,
    pub status: Status,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The track's art, as a local file, once there is one.
    pub art: Option<PathBuf>,
    pub length: Option<Duration>,
    /// Loudness, 0–1, if the player has its own volume.
    pub volume: Option<f64>,
    pub shuffle: Option<bool>,
    pub repeat: Option<Repeat>,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
    pub can_control: bool,
    track_id: String,
    art_url: String,
    /// Where it was in the track when asked, and when that was.
    position: Duration,
    as_of: Instant,
    rate: f64,
}

impl Player {
    pub fn playing(&self) -> bool {
        self.status == Status::Playing
    }

    /// Where it is in the track at `now`.
    pub fn position(&self, now: Instant) -> Duration {
        let mut position = self.position;
        if self.playing() {
            position += now.saturating_duration_since(self.as_of).mul_f64(self.rate.max(0.0));
        }
        self.length.map_or(position, |length| position.min(length))
    }

    /// Whether the track playing is the one `other` was playing.
    pub fn same_track(&self, other: &Player) -> bool {
        self.bus == other.bus
            && self.title == other.title
            && self.artist == other.artist
            && self.album == other.album
    }

    /// Its place in line to be shown: Spotify first, then other music players, then
    /// browsers.
    pub fn rank(&self) -> u8 {
        let name = self.bus.strip_prefix(PREFIX).unwrap_or(&self.bus).to_ascii_lowercase();
        if name.starts_with("spotify") {
            0
        } else if BROWSERS.iter().any(|browser| name.starts_with(browser)) {
            2
        } else {
            1
        }
    }

    fn query(bus: &str) -> Result<Self, Box<dyn Error>> {
        let player = properties(bus, PLAYER)?;
        // Some players leave the root interface out; they still play.
        let root = properties(bus, ROOT).unwrap_or(Value::Null);
        let metadata = &player["Metadata"]["data"];
        let text = |value: &Value| value["data"].as_str().unwrap_or_default().to_owned();
        // An artist is a list of names.
        let artists: Vec<&str> = metadata["xesam:artist"]["data"]
            .as_array()
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let micros = |value: &Value| value["data"].as_i64().filter(|&us| us > 0);
        let flag = |name: &str| player[name]["data"].as_bool().unwrap_or(false);
        let art_url = text(&metadata["mpris:artUrl"]);
        Ok(Self {
            bus: bus.to_owned(),
            identity: text(&root["Identity"]),
            desktop_entry: text(&root["DesktopEntry"]),
            status: match player["PlaybackStatus"]["data"].as_str() {
                Some("Playing") => Status::Playing,
                Some("Paused") => Status::Paused,
                _ => Status::Stopped,
            },
            title: text(&metadata["xesam:title"]),
            artist: artists.join(", "),
            album: text(&metadata["xesam:album"]),
            art: local_art(&art_url),
            length: micros(&metadata["mpris:length"]).map(|us| Duration::from_micros(us as u64)),
            volume: player["Volume"]["data"].as_f64(),
            shuffle: player["Shuffle"]["data"].as_bool(),
            repeat: player["LoopStatus"]["data"].as_str().map(|status| match status {
                "Playlist" => Repeat::All,
                "Track" => Repeat::One,
                _ => Repeat::Off,
            }),
            can_next: flag("CanGoNext"),
            can_previous: flag("CanGoPrevious"),
            can_seek: flag("CanSeek"),
            can_control: flag("CanControl"),
            track_id: text(&metadata["mpris:trackid"]),
            art_url,
            position: Duration::from_micros(micros(&player["Position"]).unwrap_or(0) as u64),
            as_of: Instant::now(),
            rate: player["Rate"]["data"].as_f64().unwrap_or(1.0),
        })
    }
}

/// Every property of `interface` on `bus`'s player object, by name; each is
/// `{"type": ..., "data": ...}`.
fn properties(bus: &str, interface: &str) -> Result<Value, Box<dyn Error>> {
    const GET_ALL: [&str; 3] = ["org.freedesktop.DBus.Properties", "GetAll", "s"];
    let mut args = vec!["--user", "--json=short", TIMEOUT, "call", bus, PATH];
    args.extend(GET_ALL);
    args.push(interface);
    let reply: Value = serde_json::from_str(&process::output("busctl", &args)?)?;
    Ok(reply["data"][0].clone())
}

/// The media players running, the one most worth showing first.
pub fn query() -> Result<Vec<Player>, Box<dyn Error>> {
    const LIST: [&str; 4] =
        ["org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListNames"];
    let mut args = vec!["--user", "--json=short", TIMEOUT, "call"];
    args.extend(LIST);
    let reply: Value = serde_json::from_str(&process::output("busctl", &args)?)?;
    let names = reply["data"][0].as_array().ok_or("busctl: no names")?;
    let mut players: Vec<Player> = names
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| name.starts_with(PREFIX))
        // One that won't answer is as good as gone.
        .filter_map(|bus| Player::query(bus).ok())
        .collect();
    players.sort_by(|a, b| (a.rank(), &a.bus).cmp(&(b.rank(), &b.bus)));
    Ok(players)
}

/// Calls `on_change` with the loop's data and the players whenever one comes or goes or
/// anything about one changes (other than its position advancing as it plays).
pub fn watch_players<D: 'static>(
    handle: &LoopHandle<'static, D>,
    on_change: impl FnMut(&mut D, Vec<Player>) + 'static,
) -> Result<(), Box<dyn Error>> {
    prune_art();
    let on_change: OnChange<D> = Rc::new(RefCell::new(on_change));
    // Art being downloaded, by URL.
    let fetching = Rc::new(RefCell::new(HashSet::new()));
    let loop_handle = handle.clone();
    process::on_output(
        handle,
        "mpris",
        "busctl",
        &[
            "--user",
            "monitor",
            "--json=short",
            "--match",
            "type='signal',interface='org.freedesktop.DBus.Properties',\
             member='PropertiesChanged',path='/org/mpris/MediaPlayer2'",
            "--match",
            "type='signal',interface='org.mpris.MediaPlayer2.Player',member='Seeked'",
            "--match",
            "type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged',\
             arg0namespace='org.mpris.MediaPlayer2'",
        ],
        move |data, _| refresh(&loop_handle, &on_change, &fetching, data),
    )
}

type OnChange<D> = Rc<RefCell<dyn FnMut(&mut D, Vec<Player>)>>;

/// Queries the players and reports them, first starting to fetch any art not yet here;
/// once it is, they're reported again with it.
fn refresh<D: 'static>(
    handle: &LoopHandle<'static, D>,
    on_change: &OnChange<D>,
    fetching: &Rc<RefCell<HashSet<String>>>,
    data: &mut D,
) {
    let players = match query() {
        Ok(players) => players,
        Err(err) => return eprintln!("mpris: {err}"),
    };
    for player in &players {
        let url = &player.art_url;
        let remote = url.starts_with("http://") || url.starts_with("https://");
        if !remote || player.art.is_some() || fetching.borrow().contains(url) {
            continue;
        }
        let Some(path) = art_path(url, None) else { continue };
        let partial = path.with_extension("part");
        let (Some(to), Some(part)) = (path.to_str(), partial.to_str()) else { continue };
        let (to, part, url) = (to.to_owned(), part.to_owned(), url.clone());
        let (handle_again, on_change, fetching_again) =
            (handle.clone(), on_change.clone(), fetching.clone());
        let fetched = url.clone();
        let (part_arg, url_arg) = (part.clone(), url.clone());
        let args = ["-sfL", "--max-time", "15", "-o", &part_arg, &url_arg];
        let started = process::run(handle, "curl", &args, move |data, result| {
            fetching_again.borrow_mut().remove(&fetched);
            // Renamed into place whole, so a half-written image is never shown.
            match result.and_then(|_| Ok(fs::rename(&part, &to)?)) {
                Ok(()) => refresh(&handle_again, &on_change, &fetching_again, data),
                Err(err) => eprintln!("mpris: can't fetch art: {err}"),
            }
        });
        match started {
            Ok(()) => drop(fetching.borrow_mut().insert(player.art_url.clone())),
            Err(err) => eprintln!("mpris: can't fetch art: {err}"),
        }
    }
    (on_change.borrow_mut())(data, players);
}

/// Something to tell a player to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    /// Go to this point in the track.
    Seek(Duration),
    /// Set the player's own volume, 0–1.
    Volume(f64),
    Shuffle(bool),
    Repeat(Repeat),
}

/// Sends `command` to `player`, in the background. What it changes comes back through
/// [`watch_players`].
pub fn send<D: 'static>(handle: &LoopHandle<'static, D>, player: &Player, command: Command) {
    let value;
    let mut args = vec!["--user", TIMEOUT];
    let call = |args: &mut Vec<&'static str>, method: &'static str| {
        args.extend(["call", "", PATH, PLAYER, method]);
    };
    let set = |args: &mut Vec<&'static str>, property: &'static str, kind: &'static str| {
        args.extend(["set-property", "", PATH, PLAYER, property, kind]);
    };
    let mut args: Vec<&str> = {
        match command {
            Command::PlayPause => call(&mut args, "PlayPause"),
            Command::Next => call(&mut args, "Next"),
            Command::Previous => call(&mut args, "Previous"),
            Command::Seek(_) => {
                call(&mut args, "SetPosition");
                args.push("ox");
            }
            Command::Volume(_) => set(&mut args, "Volume", "d"),
            Command::Shuffle(_) => set(&mut args, "Shuffle", "b"),
            Command::Repeat(_) => set(&mut args, "LoopStatus", "s"),
        }
        args.into_iter().map(|arg| if arg.is_empty() { player.bus.as_str() } else { arg }).collect()
    };
    match command {
        Command::Seek(position) => {
            value = position.as_micros().to_string();
            args.extend([player.track_id.as_str(), value.as_str()]);
        }
        Command::Volume(volume) => {
            value = format!("{:.3}", volume.clamp(0.0, 1.0));
            args.push(&value);
        }
        Command::Shuffle(on) => args.push(if on { "true" } else { "false" }),
        Command::Repeat(repeat) => args.push(repeat.name()),
        Command::PlayPause | Command::Next | Command::Previous => {}
    }
    let result = process::run(handle, "busctl", &args, |_, result| {
        if let Err(err) = result {
            eprintln!("mpris: {err}");
        }
    });
    if let Err(err) = result {
        eprintln!("mpris: {err}");
    }
}

/// `$XDG_CACHE_HOME/shell/art` (or `~/.cache/...`).
fn art_dir() -> Option<PathBuf> {
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(cache.join("shell/art"))
}

/// Where the art at `url` is kept; `modified` tells versions of a rewritten file apart.
fn art_path(url: &str, modified: Option<SystemTime>) -> Option<PathBuf> {
    let dir = art_dir()?;
    fs::create_dir_all(&dir).ok()?;
    // FNV-1a.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let stamp = modified
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    for byte in url.bytes().chain(stamp.to_le_bytes()) {
        hash = (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3);
    }
    Some(dir.join(format!("{hash:016x}")))
}

/// The art at `url` as a file in the cache, if it's there (remote art) or can be copied
/// there now (local art).
fn local_art(url: &str) -> Option<PathBuf> {
    if url.is_empty() {
        return None;
    }
    let Some(file) = url.strip_prefix("file://") else {
        return art_path(url, None).filter(|path| path.exists());
    };
    let file = percent_decoded(file);
    let modified = fs::metadata(&file).ok()?.modified().ok()?;
    let path = art_path(url, Some(modified))?;
    if !path.exists() {
        fs::copy(&file, &path).ok()?;
    }
    Some(path)
}

/// Deletes art that has gone unused for [`ART_LIFETIME`].
fn prune_art() {
    let Some(entries) = art_dir().and_then(|dir| fs::read_dir(dir).ok()) else { return };
    for entry in entries.flatten() {
        let age = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok());
        if age.is_some_and(|age| age > ART_LIFETIME) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// A URL's path with its `%xx` escapes decoded.
fn percent_decoded(path: &str) -> PathBuf {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escape = (bytes[i] == b'%')
            .then(|| path.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escape {
            Some(byte) => {
                decoded.push(byte);
                i += 3;
            }
            None => {
                decoded.push(bytes[i]);
                i += 1;
            }
        }
    }
    Path::new(&String::from_utf8_lossy(&decoded).into_owned()).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(bus: &str) -> Player {
        Player {
            bus: format!("{PREFIX}{bus}"),
            identity: String::new(),
            desktop_entry: String::new(),
            status: Status::Playing,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            art: None,
            length: Some(Duration::from_secs(100)),
            volume: None,
            shuffle: None,
            repeat: None,
            can_next: false,
            can_previous: false,
            can_seek: false,
            can_control: false,
            track_id: String::new(),
            art_url: String::new(),
            position: Duration::from_secs(10),
            as_of: Instant::now(),
            rate: 1.0,
        }
    }

    #[test]
    fn spotify_comes_before_other_players_and_browsers_last() {
        let ranks = ["spotify", "mpd", "vlc.instance7", "firefox.instance_1_44", "chromium.x"]
            .map(|bus| player(bus).rank());
        assert_eq!(ranks, [0, 1, 1, 2, 2]);
    }

    #[test]
    fn position_advances_while_playing_up_to_the_length() {
        let mut player = player("mpd");
        let later = player.as_of + Duration::from_secs(5);
        assert_eq!(player.position(later), Duration::from_secs(15));
        assert_eq!(player.position(later + Duration::from_secs(500)), Duration::from_secs(100));
        player.status = Status::Paused;
        assert_eq!(player.position(later), Duration::from_secs(10));
    }

    #[test]
    fn escapes_in_file_urls_are_decoded() {
        assert_eq!(percent_decoded("/a%20b/c%2Fd.png"), Path::new("/a b/c/d.png"));
        assert_eq!(percent_decoded("/100%/x"), Path::new("/100%/x"));
    }

    #[test]
    fn repeat_steps_through_its_modes() {
        assert_eq!(Repeat::Off.next().next().next(), Repeat::Off);
    }
}
