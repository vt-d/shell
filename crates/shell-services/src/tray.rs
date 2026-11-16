//! The system tray: the icons apps keep there, over the StatusNotifierItem protocol,
//! and their menus, over `com.canonical.dbusmenu`.
//!
//! Apps announce their items to whoever owns `org.kde.StatusNotifierWatcher` on the
//! session bus. If nobody does (no desktop under Hyprland provides one), the shell is
//! the watcher itself; if somebody does, it asks them what there is. Either way it then
//! reads each item's properties, and again whenever the item says something changed.
//!
//! Unlike the other services this one holds a D-Bus connection, since a name has to be
//! owned and calls answered. It lives on two threads of its own: one hears the bus's
//! signals, the other does everything else in turn (reading items, finding their icons,
//! calling them), so a slow app never stalls the shell. What they find reaches the
//! caller's loop through a channel.
//!
//! An item's icon is named, to be found in its own theme directory or the system's, or
//! sent as pixels, which are written to `$XDG_CACHE_HOME/shell/tray` as a PNG.

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    env,
    error::Error,
    fs,
    hash::{Hash, Hasher},
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender},
    },
    thread,
    time::Duration,
};

use calloop::{LoopHandle, channel};
use zbus::{
    MatchRule,
    blocking::{Connection, MessageIterator, connection::Builder},
    interface,
    message::{Header, Type},
    object_server::SignalEmitter,
    zvariant::{OwnedValue, Structure, Value},
};

use crate::apps;

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
/// Where an item is when it registers by its bus name alone.
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// How long an app gets to answer a call.
const TIMEOUT: Duration = Duration::from_secs(2);
/// The size icons sent as pixels are wanted at; the nearest at least as big is kept.
const PIXELS: i32 = 48;

/// An item in the tray.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What tells it from the others, for as long as it's there.
    pub key: String,
    /// What it's called, to show beside it.
    pub title: String,
    /// Its icon's file (PNG or SVG), if one was found.
    pub icon: Option<PathBuf>,
    /// Whether the icon is a single-color shape, to be drawn in the color of the text
    /// around it.
    pub symbolic: bool,
    /// Whether it wants attention.
    pub urgent: bool,
    /// Whether it has a menu, and whether that is all a click on it should open.
    pub has_menu: bool,
    pub menu_only: bool,
}

/// An entry of an item's menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuEntry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    /// A line between groups of entries, with nothing on it.
    pub separator: bool,
    /// Whether it's ticked, if it's one that can be.
    pub checked: Option<bool>,
    /// The entries of the menu it opens, if it opens one.
    pub children: Vec<MenuEntry>,
}

/// What the tray reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// The items there are now, in the order they came.
    Items(Vec<Item>),
    /// The menu [asked for](Tray::open_menu) of the item with this key.
    Menu { key: String, entries: Vec<MenuEntry> },
}

/// The means to act on the tray's items.
#[derive(Clone)]
pub struct Tray {
    events: Sender<Event>,
}

impl Tray {
    fn send(&self, command: Command) {
        // The worker only goes with the shell.
        let _ = self.events.send(Event::Command(command));
    }

    /// What a click on the item does: usually, shows or hides its app's window. `x` and
    /// `y` are where on the screen the click was.
    pub fn activate(&self, key: &str, x: i32, y: i32) {
        self.send(Command::Call { key: key.to_owned(), method: "Activate", x, y });
    }

    /// What a middle click on the item does.
    pub fn secondary_activate(&self, key: &str, x: i32, y: i32) {
        self.send(Command::Call { key: key.to_owned(), method: "SecondaryActivate", x, y });
    }

    /// Has the item show a menu of its own making, for one without a menu to ask for.
    pub fn context_menu(&self, key: &str, x: i32, y: i32) {
        self.send(Command::Call { key: key.to_owned(), method: "ContextMenu", x, y });
    }

    /// Turns the wheel over the item by `steps`; positive is downward.
    pub fn scroll(&self, key: &str, steps: i32) {
        self.send(Command::Scroll { key: key.to_owned(), steps });
    }

    /// Asks for the item's menu, which comes as an [`Update::Menu`].
    pub fn open_menu(&self, key: &str) {
        self.send(Command::OpenMenu { key: key.to_owned() });
    }

    /// Chooses the entry with this id from the item's menu.
    pub fn choose(&self, key: &str, id: i32) {
        self.send(Command::Choose { key: key.to_owned(), id });
    }
}

enum Command {
    Call { key: String, method: &'static str, x: i32, y: i32 },
    Scroll { key: String, steps: i32 },
    OpenMenu { key: String },
    Choose { key: String, id: i32 },
}

/// What the worker is told of.
enum Event {
    /// An item registered as `service`, by the connection `sender`.
    Registered {
        service: String,
        sender: String,
    },
    /// The connection with this unique name said one of its items changed.
    Changed(String),
    /// This bus name lost its owner.
    Gone(String),
    /// Another watcher said the item it knows as this left.
    Unregistered(String),
    Command(Command),
}

/// Starts keeping the tray, and calls `on_update` with the loop's data and what it finds,
/// whenever that changes.
pub fn watch<D: 'static>(
    handle: &LoopHandle<'static, D>,
    mut on_update: impl FnMut(&mut D, Update) + 'static,
) -> Result<Tray, Box<dyn Error>> {
    let connection = Builder::session()?.method_timeout(TIMEOUT).build()?;
    let (events, inbox) = mpsc::channel();
    let (updates, source) = channel::channel();
    handle
        .insert_source(source, move |event, _, data| {
            if let channel::Event::Msg(update) = event {
                on_update(data, update);
            }
        })
        .map_err(|err| err.error)?;

    // The names of the items registered here, as a watcher lists them.
    let registered = Arc::new(Mutex::new(Vec::new()));
    let watcher = Watcher { registered: Arc::clone(&registered), events: events.clone() };
    connection.object_server().at(WATCHER_PATH, watcher)?;
    // Whether the shell is the watcher. If another is, it knows what there is.
    let watching = connection.request_name(WATCHER).is_ok();
    let known = if watching { Vec::new() } else { register_host(&connection)? };

    let signals = MessageIterator::for_match_rule(
        MatchRule::builder().msg_type(Type::Signal).build(),
        &connection,
        Some(256),
    )?;
    let heard = events.clone();
    thread::Builder::new().name("tray-signals".into()).spawn(move || listen(signals, &heard))?;

    for service in known {
        let _ = events.send(Event::Registered { service, sender: String::new() });
    }
    let mut worker = Worker {
        connection,
        watching,
        registered,
        items: Vec::new(),
        icons: HashMap::new(),
        updates,
    };
    thread::Builder::new().name("tray".into()).spawn(move || {
        for event in inbox {
            worker.handle(event);
        }
    })?;
    Ok(Tray { events })
}

/// Tells the watcher there is that the shell shows items, and returns the ones it has.
fn register_host(connection: &Connection) -> Result<Vec<String>, Box<dyn Error>> {
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    connection.request_name(host.as_str())?;
    connection.call_method(
        Some(WATCHER),
        WATCHER_PATH,
        Some(WATCHER),
        "RegisterStatusNotifierHost",
        &(host.as_str()),
    )?;
    let reply = connection.call_method(
        Some(WATCHER),
        WATCHER_PATH,
        Some(PROPERTIES),
        "Get",
        &(WATCHER, "RegisteredStatusNotifierItems"),
    )?;
    let items: OwnedValue = reply.body().deserialize()?;
    Ok(strings(&items))
}

/// Passes on the signals that concern the tray, until the connection closes.
fn listen(signals: MessageIterator, events: &Sender<Event>) {
    for message in signals.flatten() {
        let header = message.header();
        let (Some(interface), Some(member)) = (header.interface(), header.member()) else {
            continue;
        };
        let sender = header.sender().map(|sender| sender.to_string()).unwrap_or_default();
        let body = message.body();
        let event = match (interface.as_str(), member.as_str()) {
            (ITEM, _) => Event::Changed(sender),
            (WATCHER, "StatusNotifierItemRegistered") => match body.deserialize::<String>() {
                Ok(service) => Event::Registered { service, sender: String::new() },
                Err(_) => continue,
            },
            (WATCHER, "StatusNotifierItemUnregistered") => match body.deserialize::<String>() {
                Ok(service) => Event::Unregistered(service),
                Err(_) => continue,
            },
            ("org.freedesktop.DBus", "NameOwnerChanged") => {
                match body.deserialize::<(String, String, String)>() {
                    Ok((name, _, new)) if new.is_empty() => Event::Gone(name),
                    _ => continue,
                }
            }
            _ => continue,
        };
        if events.send(event).is_err() {
            return;
        }
    }
}

/// The watcher: where apps register their items.
struct Watcher {
    registered: Arc<Mutex<Vec<String>>>,
    events: Sender<Event>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    /// `service` is the item's bus name, or the path of its object on the caller's
    /// connection.
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let sender = header.sender().map(|sender| sender.to_string()).unwrap_or_default();
        let name = if service.starts_with('/') {
            format!("{sender}{service}")
        } else {
            service.to_owned()
        };
        let _ = Self::status_notifier_item_registered(&emitter, &name).await;
        let _ = self.events.send(Event::Registered { service: service.to_owned(), sender });
    }

    async fn register_status_notifier_host(
        &self,
        _service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let _ = Self::status_notifier_host_registered(&emitter).await;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.registered.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// An item, and where it is on the bus.
struct Tracked {
    /// The name it registered as, if not its connection's own.
    name: Option<String>,
    /// Its connection's unique name, and its object's path there.
    owner: String,
    path: String,
    /// Its menu's path, if it has one.
    menu: Option<String>,
    item: Item,
}

struct Worker {
    connection: Connection,
    watching: bool,
    registered: Arc<Mutex<Vec<String>>>,
    items: Vec<Tracked>,
    /// Icons found by name, by the name and the item's theme directory.
    icons: HashMap<(String, String), Option<PathBuf>>,
    updates: channel::Sender<Update>,
}

impl Worker {
    fn handle(&mut self, event: Event) {
        match event {
            Event::Registered { service, sender } => self.register(&service, &sender),
            Event::Changed(owner) => {
                let mut changed = false;
                for index in 0..self.items.len() {
                    if self.items[index].owner == owner {
                        changed |= self.read(index);
                    }
                }
                if changed {
                    self.report();
                }
            }
            Event::Gone(name) => {
                self.remove(|item| item.owner == name || item.name.as_deref() == Some(&name));
            }
            Event::Unregistered(service) => {
                self.remove(|item| {
                    service == format!("{}{}", item.owner, item.path)
                        || item.name.as_deref() == Some(&service)
                });
            }
            Event::Command(command) => self.command(command),
        }
    }

    fn register(&mut self, service: &str, sender: &str) {
        // A path on the caller's connection, a bus name, or the two run together (as a
        // watcher lists them).
        let (name, path) = match service.find('/') {
            Some(0) => (sender, service),
            Some(slash) => service.split_at(slash),
            None => (service, ITEM_PATH),
        };
        let owner = if name.starts_with(':') {
            name.to_owned()
        } else {
            let reply = self.connection.call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(name),
            );
            match reply.and_then(|reply| reply.body().deserialize::<String>()) {
                Ok(owner) => owner,
                Err(_) => return,
            }
        };
        if self.items.iter().any(|item| item.owner == owner && item.path == path) {
            return;
        }
        let key = format!("{owner}{path}");
        self.items.push(Tracked {
            name: (name != owner).then(|| name.to_owned()),
            owner,
            path: path.to_owned(),
            menu: None,
            item: Item {
                key: key.clone(),
                title: String::new(),
                icon: None,
                symbolic: false,
                urgent: false,
                has_menu: false,
                menu_only: false,
            },
        });
        if !self.read(self.items.len() - 1) {
            // Nothing answered there.
            self.items.pop();
            return;
        }
        if self.watching {
            self.registered.lock().unwrap().push(key);
        }
        self.report();
    }

    fn remove(&mut self, gone: impl Fn(&Tracked) -> bool) {
        let before = self.items.len();
        for item in self.items.iter().filter(|item| gone(item)) {
            if self.watching {
                self.registered.lock().unwrap().retain(|key| *key != item.item.key);
                let _ = self.connection.emit_signal(
                    None::<&str>,
                    WATCHER_PATH,
                    WATCHER,
                    "StatusNotifierItemUnregistered",
                    &(item.item.key.as_str()),
                );
            }
        }
        self.items.retain(|item| !gone(item));
        if self.items.len() != before {
            self.report();
        }
    }

    fn report(&self) {
        let items = self.items.iter().map(|item| item.item.clone()).collect();
        let _ = self.updates.send(Update::Items(items));
    }

    /// Reads an item's properties. Returns whether it answered and anything changed.
    fn read(&mut self, index: usize) -> bool {
        let tracked = &self.items[index];
        let reply = self.connection.call_method(
            Some(tracked.owner.as_str()),
            tracked.path.as_str(),
            Some(PROPERTIES),
            "GetAll",
            &(ITEM),
        );
        let properties = match reply
            .and_then(|reply| reply.body().deserialize::<HashMap<String, OwnedValue>>())
        {
            Ok(properties) => properties,
            Err(err) => {
                eprintln!("tray: {}: {err}", tracked.item.key);
                return false;
            }
        };
        let text = |key: &str| match properties.get(key).map(|value| &**value) {
            Some(Value::Str(text)) => text.as_str(),
            Some(Value::ObjectPath(path)) => path.as_str(),
            _ => "",
        };
        let flag = |key: &str| matches!(properties.get(key).map(|v| &**v), Some(Value::Bool(true)));

        let urgent = text("Status") == "NeedsAttention";
        let id = text("Id");
        // The icon that wants attention, while it's wanted and there is one.
        let (name, pixels) = if urgent && !text("AttentionIconName").is_empty() {
            (text("AttentionIconName"), "AttentionIconPixmap")
        } else {
            (text("IconName"), "IconPixmap")
        };
        let theme = text("IconThemePath");
        let named = (!name.is_empty())
            .then(|| {
                self.icons
                    .entry((name.to_owned(), theme.to_owned()))
                    .or_insert_with(|| named_icon(name, theme))
                    .clone()
            })
            .flatten();
        let symbolic = named.is_some() && name.ends_with("-symbolic");
        let icon = named
            .or_else(|| properties.get(pixels).and_then(|pixmaps| pixmap_icon(pixmaps)))
            .or_else(|| apps::find(id));
        let menu = Some(text("Menu")).filter(|menu| menu.len() > 1).map(str::to_owned);
        let title = [text("Title"), id].into_iter().find(|title| !title.is_empty());
        let item = Item {
            key: tracked.item.key.clone(),
            title: title.unwrap_or_default().to_owned(),
            symbolic,
            icon,
            urgent,
            has_menu: menu.is_some(),
            menu_only: flag("ItemIsMenu"),
        };
        let tracked = &mut self.items[index];
        let changed = tracked.item != item || tracked.item.title.is_empty();
        tracked.item = item;
        tracked.menu = menu;
        changed
    }

    fn command(&mut self, command: Command) {
        let find = |key: &str| self.items.iter().find(|item| item.item.key == key);
        let result = match &command {
            Command::Call { key, method, x, y } => find(key).map(|item| {
                self.connection.call_method(
                    Some(item.owner.as_str()),
                    item.path.as_str(),
                    Some(ITEM),
                    *method,
                    &(*x, *y),
                )
            }),
            Command::Scroll { key, steps } => find(key).map(|item| {
                self.connection.call_method(
                    Some(item.owner.as_str()),
                    item.path.as_str(),
                    Some(ITEM),
                    "Scroll",
                    &(*steps, "vertical"),
                )
            }),
            Command::Choose { key, id } => find(key)
                .and_then(|item| Some((item, item.menu.as_deref()?)))
                .map(|(item, menu)| {
                    self.connection.call_method(
                        Some(item.owner.as_str()),
                        menu,
                        Some(MENU),
                        "Event",
                        &(*id, "clicked", Value::I32(0), 0u32),
                    )
                }),
            Command::OpenMenu { key } => {
                let Some((item, menu)) =
                    find(key).and_then(|item| Some((item, item.menu.as_deref()?)))
                else {
                    return;
                };
                // Some apps only fill their menu in once told it's about to be shown.
                let _ = self.connection.call_method(
                    Some(item.owner.as_str()),
                    menu,
                    Some(MENU),
                    "AboutToShow",
                    &(0i32),
                );
                let layout = self.connection.call_method(
                    Some(item.owner.as_str()),
                    menu,
                    Some(MENU),
                    "GetLayout",
                    &(0i32, -1i32, Vec::<&str>::new()),
                );
                let entries = layout.and_then(|reply| {
                    let body = reply.body();
                    let layout: Structure = body.deserialize()?;
                    Ok(layout.fields().get(1).and_then(menu_entry).unwrap_or_default().children)
                });
                match entries {
                    Ok(entries) => {
                        let _ = self.updates.send(Update::Menu { key: key.clone(), entries });
                    }
                    Err(err) => eprintln!("tray: {key}: {err}"),
                }
                return;
            }
        };
        if let Some(Err(err)) = result {
            eprintln!("tray: {err}");
        }
    }
}

/// The strings in an array.
fn strings(value: &Value) -> Vec<String> {
    let value = if let Value::Value(inner) = value { inner } else { value };
    let Value::Array(array) = value else { return Vec::new() };
    let string = |value: &Value| match value {
        Value::Str(text) => Some(text.as_str().to_owned()),
        _ => None,
    };
    array.iter().filter_map(string).collect()
}

/// A menu entry from dbusmenu's layout: its id, its properties and its children, each
/// of those the same again.
fn menu_entry(value: &Value) -> Option<MenuEntry> {
    let value = if let Value::Value(inner) = value { inner } else { value };
    let Value::Structure(layout) = value else { return None };
    let [Value::I32(id), Value::Dict(properties), Value::Array(children)] = layout.fields() else {
        return None;
    };
    let mut entry = MenuEntry { id: *id, enabled: true, ..MenuEntry::default() };
    let mut visible = true;
    let mut toggles = false;
    for (key, value) in properties.iter() {
        let Value::Str(key) = key else { continue };
        let value = if let Value::Value(inner) = value { &**inner } else { value };
        match (key.as_str(), value) {
            ("label", Value::Str(label)) => entry.label = plain(label),
            ("enabled", Value::Bool(enabled)) => entry.enabled = *enabled,
            ("visible", Value::Bool(shown)) => visible = *shown,
            ("type", Value::Str(kind)) => entry.separator = kind.as_str() == "separator",
            ("toggle-type", Value::Str(kind)) => toggles = !kind.is_empty(),
            ("toggle-state", Value::I32(state)) => entry.checked = Some(*state == 1),
            _ => {}
        }
    }
    if !visible {
        return None;
    }
    if !toggles {
        entry.checked = None;
    } else if entry.checked.is_none() {
        entry.checked = Some(false);
    }
    entry.children = children.iter().filter_map(menu_entry).collect();
    Some(entry)
}

/// A menu label without the underscores that mark its access key (`_Quit`; `__` is an
/// underscore).
fn plain(label: &str) -> String {
    let mut plain = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c != '_' {
            plain.push(c);
        } else if let Some(next) = chars.next() {
            plain.push(next);
        }
    }
    plain
}

/// The file of the icon called `name`: a path as it is, else from the item's own theme
/// directory, else from the icon themes installed, hicolor first.
fn named_icon(name: &str, theme_path: &str) -> Option<PathBuf> {
    if Path::new(name).is_absolute() {
        return Path::new(name).is_file().then(|| name.into());
    }
    if !theme_path.is_empty()
        && let Some(file) = icon_under(Path::new(theme_path), name, 4)
    {
        return Some(file);
    }
    let home = env::var_os("HOME").map(|home| PathBuf::from(home).join(".icons"));
    let dirs: Vec<PathBuf> =
        home.into_iter().chain(apps::data_dirs().iter().map(|dir| dir.join("icons"))).collect();
    let hicolor = dirs.iter().map(|dir| dir.join("hicolor"));
    let others = dirs
        .iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flat_map(|themes| themes.flatten().map(|theme| theme.path()))
        .filter(|theme| theme.file_name().is_some_and(|name| name != "hicolor"));
    let found = hicolor.chain(others).find_map(|theme| icon_under(&theme, name, 3));
    found.or_else(|| {
        apps::data_dirs().iter().find_map(|dir| icon_under(&dir.join("pixmaps"), name, 0))
    })
}

/// The best file for icon `name` in `dir` or up to `depth` directories under it: an SVG,
/// or failing that the PNG in the directory named for the largest size.
fn icon_under(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    fn search(dir: &Path, name: &str, depth: usize, best: &mut Option<(u32, PathBuf)>) {
        let svg = dir.join(format!("{name}.svg"));
        if svg.is_file() {
            *best = Some((u32::MAX, svg));
            return;
        }
        let png = dir.join(format!("{name}.png"));
        if png.is_file() {
            // "48x48", "48", "48x48@2x": the size leads the directory's name, or that
            // of the one above (themes file by size and kind in either order).
            let size = dir
                .ancestors()
                .take(2)
                .filter_map(|dir| dir.file_name()?.to_str())
                .find_map(|name| name.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok())
                .unwrap_or(1);
            if best.as_ref().is_none_or(|(most, _)| size > *most) {
                *best = Some((size, png));
            }
        }
        if depth == 0 {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            if matches!(best, Some((u32::MAX, _))) {
                return;
            }
            // Followed through links, as themes are made of.
            let path = entry.path();
            if path.is_dir() {
                search(&path, name, depth - 1, best);
            }
        }
    }
    let mut best = None;
    search(dir, name, depth, &mut best);
    best.map(|(_, file)| file)
}

/// An icon sent as pixels (an array of width, height and ARGB bytes, at several sizes),
/// written as a PNG named for its contents. Returns that file.
fn pixmap_icon(pixmaps: &Value) -> Option<PathBuf> {
    let Value::Array(pixmaps) = pixmaps else { return None };
    fn pixmap<'a>(value: &'a Value<'a>) -> Option<(i32, i32, &'a zbus::zvariant::Array<'a>)> {
        let Value::Structure(pixmap) = value else { return None };
        let [Value::I32(width), Value::I32(height), Value::Array(bytes)] = pixmap.fields() else {
            return None;
        };
        (*width > 0 && *height > 0 && bytes.len() as i64 == *width as i64 * *height as i64 * 4)
            .then_some((*width, *height, bytes))
    }
    // The smallest that's big enough, or the biggest there is.
    let (width, height, bytes) = pixmaps.iter().filter_map(pixmap).min_by_key(|(width, ..)| {
        if *width >= PIXELS { (0, *width) } else { (1, PIXELS - *width) }
    })?;
    let argb: Vec<u8> = bytes
        .iter()
        .filter_map(|byte| if let Value::U8(byte) = byte { Some(*byte) } else { None })
        .collect();
    let rgba: Vec<u8> =
        argb.as_chunks::<4>().0.iter().flat_map(|&[a, r, g, b]| [r, g, b, a]).collect();
    if rgba.iter().skip(3).step_by(4).all(|&alpha| alpha == 0) {
        // Nothing to see: some apps send an empty picture along with a named icon.
        return None;
    }

    let mut hasher = DefaultHasher::new();
    (width, height, &rgba).hash(&mut hasher);
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    let dir = cache.join("shell/tray");
    let file = dir.join(format!("{:016x}.png", hasher.finish()));
    if file.is_file() {
        return Some(file);
    }
    let write = || -> Result<(), Box<dyn Error>> {
        fs::create_dir_all(&dir)?;
        let out = BufWriter::new(fs::File::create(&file)?);
        let mut encoder = png::Encoder::new(out, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&rgba)?;
        Ok(())
    };
    match write() {
        Ok(()) => Some(file),
        Err(err) => {
            eprintln!("tray: can't write {}: {err}", file.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use zbus::zvariant::{Array, Dict, Signature, StructureBuilder};

    use super::*;

    #[test]
    fn menu_labels_lose_their_access_keys() {
        assert_eq!(plain("_Quit"), "Quit");
        assert_eq!(plain("Save _As..."), "Save As...");
        assert_eq!(plain("snake__case"), "snake_case");
    }

    fn entry(
        id: i32,
        properties: &[(&'static str, Value<'static>)],
        children: Vec<Value<'static>>,
    ) -> Value<'static> {
        let mut dict = Dict::new(&Signature::Str, &Signature::Variant);
        for (key, value) in properties {
            dict.append(Value::from(*key), Value::Value(Box::new(value.try_clone().unwrap())))
                .unwrap();
        }
        let mut array = Array::new(&Signature::Variant);
        for child in children {
            array.append(Value::Value(Box::new(child))).unwrap();
        }
        let layout = StructureBuilder::new()
            .add_field(id)
            .append_field(Value::Dict(dict))
            .append_field(Value::Array(array))
            .build()
            .unwrap();
        Value::Structure(layout)
    }

    #[test]
    fn reads_a_menu_with_what_its_entries_are() {
        let layout = entry(
            0,
            &[],
            vec![
                entry(1, &[("label", "_Open".into())], vec![]),
                entry(2, &[("type", "separator".into())], vec![]),
                entry(
                    3,
                    &[
                        ("label", "Mute".into()),
                        ("toggle-type", "checkmark".into()),
                        ("toggle-state", 1i32.into()),
                    ],
                    vec![],
                ),
                entry(4, &[("label", "Hidden".into()), ("visible", false.into())], vec![]),
                entry(
                    5,
                    &[("label", "More".into()), ("enabled", false.into())],
                    vec![entry(6, &[("label", "Deep".into())], vec![])],
                ),
            ],
        );
        let menu = menu_entry(&layout).unwrap().children;
        assert_eq!(menu.len(), 4);
        assert_eq!((menu[0].id, menu[0].label.as_str(), menu[0].enabled), (1, "Open", true));
        assert!(menu[1].separator);
        assert_eq!(menu[2].checked, Some(true));
        assert!(!menu[3].enabled);
        assert_eq!(menu[3].children[0].label, "Deep");
    }
}
