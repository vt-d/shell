//! Installed apps, found the freedesktop way: the desktop entries in `applications`
//! across `$XDG_DATA_HOME` and `$XDG_DATA_DIRS`.
//!
//! - [`installed`] lists the ones a launcher shows, and [`launch`] starts one.
//! - [`AppIcons`] finds icons: a window's class (Wayland app id) names a desktop entry,
//!   whose `Icon` is looked up in the hicolor icon theme and then in `pixmaps`.
//! - [`Launches`] remembers how often each app was started, across runs.

use std::{
    collections::{HashMap, HashSet},
    env,
    error::Error,
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::hyprland;

/// Icon lookups, cached by window class: each one reads directories.
#[derive(Default)]
pub struct AppIcons {
    cache: HashMap<String, Option<PathBuf>>,
    named: HashMap<String, Option<PathBuf>>,
}

impl AppIcons {
    /// The icon file (PNG or SVG) for the app whose windows have class `class`.
    pub fn get(&mut self, class: &str) -> Option<&Path> {
        self.cache.entry(class.to_owned()).or_insert_with(|| find(class)).as_deref()
    }

    /// The file for the icon called `name`, as a desktop entry's `Icon` names it.
    pub fn named(&mut self, name: &str) -> Option<&Path> {
        let find = || icon_file(&data_dirs(), name);
        self.named.entry(name.to_owned()).or_insert_with(find).as_deref()
    }
}

pub(crate) fn find(class: &str) -> Option<PathBuf> {
    let dirs = data_dirs();
    let name = desktop_entry_icon(&dirs, class);
    // Without a desktop entry, apps commonly name their icon after their app id.
    let names = name.into_iter().chain([class.to_owned(), class.to_lowercase()]);
    names.into_iter().find_map(|name| icon_file(&dirs, &name))
}

/// `$XDG_DATA_HOME`, then `$XDG_DATA_DIRS`, most important first.
pub(crate) fn data_dirs() -> Vec<PathBuf> {
    let home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let system = env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    home.into_iter().chain(system.split(':').map(PathBuf::from)).collect()
}

/// The `Icon` of the desktop entry for `class`: the entry named after it, else one whose
/// `StartupWMClass` is it, else one whose reverse-DNS name ends in it
/// (`org.gnome.Nautilus` for `nautilus`).
fn desktop_entry_icon(dirs: &[PathBuf], class: &str) -> Option<String> {
    let mut by_wm_class = None;
    let mut by_suffix = None;
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir.join("applications")) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "desktop") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let named = stem.eq_ignore_ascii_case(class);
            let suffix =
                stem.rsplit('.').next().is_some_and(|last| last.eq_ignore_ascii_case(class));
            if !named && by_suffix.is_some() && by_wm_class.is_some() {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else { continue };
            let entry = DesktopEntry::parse(&contents);
            let Some(icon) = entry.icon else { continue };
            if named {
                return Some(icon.to_owned());
            }
            if by_wm_class.is_none()
                && entry.wm_class.is_some_and(|wm_class| wm_class.eq_ignore_ascii_case(class))
            {
                by_wm_class = Some(icon.to_owned());
            } else if by_suffix.is_none() && suffix {
                by_suffix = Some(icon.to_owned());
            }
        }
    }
    by_wm_class.or(by_suffix)
}

#[derive(Default)]
struct DesktopEntry<'a> {
    kind: Option<&'a str>,
    name: Option<&'a str>,
    generic_name: Option<&'a str>,
    comment: Option<&'a str>,
    keywords: Option<&'a str>,
    exec: Option<&'a str>,
    icon: Option<&'a str>,
    wm_class: Option<&'a str>,
    terminal: bool,
    /// `NoDisplay` or `Hidden`: not for menus.
    hidden: bool,
    only_show_in: Option<&'a str>,
    not_show_in: Option<&'a str>,
}

impl<'a> DesktopEntry<'a> {
    /// The keys of the `[Desktop Entry]` group (not of actions).
    fn parse(contents: &'a str) -> Self {
        let mut entry = Self::default();
        let mut in_main_group = false;
        for line in contents.lines().map(str::trim) {
            if line.starts_with('[') {
                in_main_group = line == "[Desktop Entry]";
            } else if in_main_group && let Some((key, value)) = line.split_once('=') {
                let value = value.trim();
                match key.trim() {
                    "Type" => entry.kind = Some(value),
                    "Name" => entry.name = Some(value),
                    "GenericName" => entry.generic_name = Some(value),
                    "Comment" => entry.comment = Some(value),
                    "Keywords" => entry.keywords = Some(value),
                    "Exec" => entry.exec = Some(value),
                    "Icon" => entry.icon = Some(value),
                    "StartupWMClass" => entry.wm_class = Some(value),
                    "Terminal" => entry.terminal = value == "true",
                    "NoDisplay" | "Hidden" => entry.hidden |= value == "true",
                    "OnlyShowIn" => entry.only_show_in = Some(value),
                    "NotShowIn" => entry.not_show_in = Some(value),
                    _ => {}
                }
            }
        }
        entry
    }
}

/// The best file for icon `name`: an absolute path as is; else from hicolor, a scalable
/// SVG or failing that the smallest PNG of at least 64 pixels (the largest if none is),
/// else one from `pixmaps`.
fn icon_file(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    if Path::new(name).is_absolute() {
        return Path::new(name).is_file().then(|| name.into());
    }
    let home_icons = env::var_os("HOME").map(|home| PathBuf::from(home).join(".icons"));
    let icon_dirs = home_icons.into_iter().chain(dirs.iter().map(|dir| dir.join("icons")));

    let mut best_png: Option<(u32, PathBuf)> = None;
    for theme in icon_dirs.map(|dir| dir.join("hicolor")) {
        let Ok(sizes) = fs::read_dir(&theme) else { continue };
        for size_dir in sizes.flatten() {
            let size_name = size_dir.file_name();
            let Some(size_name) = size_name.to_str() else { continue };
            let apps = size_dir.path().join("apps");
            if size_name == "scalable" {
                let svg = apps.join(format!("{name}.svg"));
                if svg.is_file() {
                    return Some(svg);
                }
                continue;
            }
            // "48x48", or "48x48@2" for 96 pixels.
            let (dimensions, factor) = size_name.split_once('@').unwrap_or((size_name, "1"));
            let Some(size) = dimensions.split_once('x').and_then(|(w, _)| w.parse::<u32>().ok())
            else {
                continue;
            };
            let size = size * factor.parse::<u32>().unwrap_or(1);
            let png = apps.join(format!("{name}.png"));
            if !png.is_file() {
                continue;
            }
            let better =
                best_png.as_ref().is_none_or(|(best, _)| match (*best >= 64, size >= 64) {
                    (true, true) => size < *best,
                    (false, false) => size > *best,
                    (best_big_enough, _) => !best_big_enough,
                });
            if better {
                best_png = Some((size, png));
            }
        }
    }
    if let Some((_, png)) = best_png {
        return Some(png);
    }
    dirs.iter().map(|dir| dir.join("pixmaps")).find_map(|pixmaps| {
        ["svg", "png"].iter().map(|ext| pixmaps.join(format!("{name}.{ext}"))).find(|f| f.is_file())
    })
}

/// An installed app, as its desktop entry describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// Its desktop entry's file name, without `.desktop`.
    pub id: String,
    pub name: String,
    /// What kind of thing it is ("Web Browser"), or failing that what it's for.
    pub description: Option<String>,
    pub keywords: Vec<String>,
    /// Its icon's name, for [`AppIcons::named`].
    pub icon: Option<String>,
    /// The command line that starts it, for a shell.
    pub command: String,
    /// Whether it runs in a terminal.
    pub terminal: bool,
}

impl App {
    /// The app `entry` describes, if it's one to list on a desktop among `desktops`.
    fn from_entry(id: &str, entry: &DesktopEntry, desktops: &[String]) -> Option<Self> {
        let listed =
            |list: &str| list.split(';').any(|name| desktops.iter().any(|desktop| desktop == name));
        if entry.kind != Some("Application")
            || entry.hidden
            || entry.only_show_in.is_some_and(|list| !listed(list))
            || entry.not_show_in.is_some_and(listed)
        {
            return None;
        }
        let name = entry.name.filter(|name| !name.is_empty())?;
        let command = command(entry.exec?);
        if command.is_empty() {
            return None;
        }
        fn described<'a>(text: Option<&'a str>, name: &str) -> Option<&'a str> {
            text.filter(|text| !text.is_empty() && !text.eq_ignore_ascii_case(name))
        }
        Some(Self {
            id: id.to_owned(),
            name: name.to_owned(),
            description: described(entry.generic_name, name)
                .or(described(entry.comment, name))
                .map(str::to_owned),
            keywords: entry
                .keywords
                .into_iter()
                .flat_map(|keywords| keywords.split(';'))
                .filter(|keyword| !keyword.is_empty())
                .map(str::to_lowercase)
                .collect(),
            icon: entry.icon.filter(|icon| !icon.is_empty()).map(str::to_owned),
            command,
            terminal: entry.terminal,
        })
    }
}

/// An `Exec` line without its field codes (`%U` and the like, which stand for files to
/// open): the command that starts the app on its own.
fn command(exec: &str) -> String {
    let mut command = String::with_capacity(exec.len());
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            command.push(c);
        } else if chars.next() == Some('%') {
            command.push('%');
        }
    }
    command.trim().to_owned()
}

/// The apps to list in a launcher, by name. Where directories have an entry of the same
/// name, the first (the user's own, then the system's in order) stands for all, even if
/// only to hide the app.
pub fn installed() -> Vec<App> {
    let desktops: Vec<String> = env::var("XDG_CURRENT_DESKTOP")
        .map(|desktops| desktops.split(':').map(str::to_owned).collect())
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in data_dirs() {
        let Ok(entries) = fs::read_dir(dir.join("applications")) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "desktop") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else { continue };
            if !seen.insert(id.to_owned()) {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else { continue };
            apps.extend(App::from_entry(id, &DesktopEntry::parse(&contents), &desktops));
        }
    }
    apps.sort_by_cached_key(|app| (app.name.to_lowercase(), app.id.clone()));
    apps
}

/// The terminal apps that need one run in, unless `$TERMINAL` names another.
const TERMINAL: &str = "kitty";

/// Starts `app`, as the compositor's child where that can be asked for, so that it
/// outlives the shell and has the session's environment.
pub fn launch(app: &App) -> Result<(), Box<dyn Error>> {
    let command = if app.terminal {
        let terminal = env::var("TERMINAL").ok().filter(|terminal| !terminal.is_empty());
        format!("{} -e {}", terminal.as_deref().unwrap_or(TERMINAL), app.command)
    } else {
        app.command.clone()
    };
    if hyprland::exec(&command).is_ok() {
        return Ok(());
    }
    // Left running by a shell that exits at once, in a process group of its own.
    let status = Command::new("sh")
        .args(["-c", &format!("{command} &")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .status()?;
    if status.success() { Ok(()) } else { Err(format!("sh failed: {status}").into()) }
}

/// How many times each app has been [started](Launches::record), kept in
/// `$XDG_STATE_HOME/shell/launches` (or `~/.local/state/...`): a line for each, its
/// count and then its id.
#[derive(Default)]
pub struct Launches {
    counts: HashMap<String, u32>,
    path: Option<PathBuf>,
}

impl Launches {
    pub fn load() -> Self {
        let state = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")));
        let path = state.map(|state| state.join("shell/launches"));
        let contents = path.as_ref().and_then(|path| fs::read_to_string(path).ok());
        Self { counts: Self::parse(contents.as_deref().unwrap_or_default()), path }
    }

    fn parse(contents: &str) -> HashMap<String, u32> {
        let line = |line: &str| {
            let (count, id) = line.split_once(' ')?;
            Some((id.to_owned(), count.parse().ok()?))
        };
        contents.lines().filter_map(line).collect()
    }

    /// How many times the app with this id has been started.
    pub fn count(&self, id: &str) -> u32 {
        self.counts.get(id).copied().unwrap_or(0)
    }

    /// Notes that the app with this id was started.
    pub fn record(&mut self, id: &str) {
        *self.counts.entry(id.to_owned()).or_default() += 1;
        let Some(path) = &self.path else { return };
        let mut lines: Vec<String> =
            self.counts.iter().map(|(id, count)| format!("{count} {id}\n")).collect();
        lines.sort();
        let written = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(path, lines.concat()));
        if let Err(err) = written {
            eprintln!("apps: can't write {}: {err}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_the_main_group() {
        let entry = DesktopEntry::parse(
            "[Desktop Entry]\nName=Files\nIcon=org.gnome.Nautilus\nStartupWMClass=nautilus\n\
             [Desktop Action new-window]\nIcon=window-new\n",
        );
        assert_eq!(entry.icon, Some("org.gnome.Nautilus"));
        assert_eq!(entry.wm_class, Some("nautilus"));
    }

    fn app(contents: &str, desktops: &[&str]) -> Option<App> {
        let desktops: Vec<String> = desktops.iter().map(|&d| d.to_owned()).collect();
        App::from_entry("id", &DesktopEntry::parse(contents), &desktops)
    }

    #[test]
    fn lists_an_app_with_what_starts_it() {
        let app = app(
            "[Desktop Entry]\nType=Application\nName=Firefox\nGenericName=Web Browser\n\
             Comment=Browse the web\nKeywords=Internet;WWW;\nExec=firefox --name \"100%%\" %U\n\
             Icon=firefox\nTerminal=false\n",
            &["Hyprland"],
        )
        .unwrap();
        assert_eq!(app.name, "Firefox");
        assert_eq!(app.description.as_deref(), Some("Web Browser"));
        assert_eq!(app.keywords, ["internet", "www"]);
        assert_eq!(app.command, "firefox --name \"100%\"");
        assert_eq!(app.icon.as_deref(), Some("firefox"));
        assert!(!app.terminal);
    }

    #[test]
    fn leaves_out_what_is_not_for_this_menu() {
        let entry = |extra: &str| {
            app(
                &format!("[Desktop Entry]\nType=Application\nName=A\nExec=a\n{extra}"),
                &["Hyprland"],
            )
        };
        assert!(entry("").is_some());
        assert!(entry("NoDisplay=true\n").is_none());
        assert!(entry("Hidden=true\n").is_none());
        assert!(entry("OnlyShowIn=GNOME;KDE;\n").is_none());
        assert!(entry("OnlyShowIn=GNOME;Hyprland;\n").is_some());
        assert!(entry("NotShowIn=Hyprland;\n").is_none());
        assert!(app("[Desktop Entry]\nType=Link\nName=A\nExec=a\n", &[]).is_none());
        assert!(app("[Desktop Entry]\nType=Application\nName=A\n", &[]).is_none());
    }

    #[test]
    fn reads_launch_counts() {
        let counts = Launches::parse("3 firefox\n12 org.gnome.Nautilus\nnonsense\n");
        assert_eq!(counts.len(), 2);
        assert_eq!(counts["firefox"], 3);
        assert_eq!(counts["org.gnome.Nautilus"], 12);
    }
}
