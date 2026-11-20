//! The shell: a bar, and what opens from it and over it.
//!
//! Run with no arguments it is the shell itself. Run with a command it passes that to
//! the shell already running, which is how keys are bound to it:
//!
//! - `shell launcher_open`, `launcher_close`, `launcher_toggle`: the app launcher.

mod bar;
mod clock;
mod launcher;
mod media;
mod power;
mod quick;
mod status;
mod tray;
mod workspaces;

use std::{
    cell::RefCell, env, error::Error, path::PathBuf, process::ExitCode, rc::Rc, time::Duration,
};

use shell_m3e::{use_typeface, watch_theme};
use shell_services::{
    audio::{self, Volume},
    clock::{Time, watch_minutes},
    hyprland::{self, Workspaces},
    ipc, mpris,
    network::{self, Network},
    quick as quick_settings, tray as tray_service,
};
use shell_ui::{
    Frame, Ui, WindowId,
    calloop::{
        EventLoop,
        timer::{TimeoutAction, Timer},
    },
};

use bar::Bar;
use clock::Clock;
use launcher::Launcher;
use media::{Action, Media};
use power::Power;
use quick::QuickSettings;
use status::Status;
use tray::Tray;
use workspaces::WorkspaceIndicator;

/// The bar's state, shared by its views and the event sources that update it.
type SharedBar = Rc<RefCell<Bar>>;

/// The way to the tray's items, once the tray is kept, and where the menu last asked for
/// is to go: how far its middle is from the screen's right edge, and the screen's width.
#[derive(Default)]
struct TrayLink {
    service: Option<tray_service::Tray>,
    anchor: (f32, f32),
}

type SharedTray = Rc<RefCell<TrayLink>>;

/// What can be asked of the running shell.
// Named as they are typed; the launcher's are only the first.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    LauncherOpen,
    LauncherClose,
    LauncherToggle,
}

impl Command {
    const USAGE: &str = "usage: shell [COMMAND]

With no command, runs the shell. With one, asks the running shell to do it:

  launcher_open     open the app launcher
  launcher_close    close it
  launcher_toggle   open it if it is closed, close it if it is open";

    fn parse(name: &str) -> Option<Self> {
        match name {
            "launcher_open" => Some(Self::LauncherOpen),
            "launcher_close" => Some(Self::LauncherClose),
            "launcher_toggle" => Some(Self::LauncherToggle),
            _ => None,
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.as_slice() {
        [] => run(),
        [help] if matches!(help.as_str(), "help" | "-h" | "--help") => {
            println!("{}", Command::USAGE);
            Ok(())
        }
        [name] if Command::parse(name).is_some() => match ipc::send(name) {
            Ok(answer) if answer == "ok" => Ok(()),
            Ok(answer) => Err(answer.into()),
            Err(err) => Err(err),
        },
        _ => Err(Command::USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the shell until its last window closes.
fn run() -> Result<(), Box<dyn Error>> {
    // Everything the shell reacts to is an event source on this loop. It sleeps in the
    // kernel until one is ready; there is no polling and no timeout.
    let mut event_loop = EventLoop::try_new()?;
    let mut ui = Ui::connect(&event_loop)?;
    use_typeface(&mut ui);
    if let Some(path) = theme_path() {
        // Without it the built-in theme is used, which isn't worth failing over.
        if let Err(err) = watch_theme(&mut ui, &path) {
            eprintln!("can't watch {}: {err}", path.display());
        }
    }

    let workspaces = Workspaces::query().unwrap_or_else(|err| {
        eprintln!("hyprland: {err}");
        Workspaces::default()
    });
    let network = Network::query().unwrap_or_else(|err| {
        eprintln!("network: {err}");
        Network::default()
    });
    let volume = Volume::query().unwrap_or_else(|err| {
        eprintln!("audio: {err}");
        Volume::default()
    });
    let players = mpris::query().unwrap_or_else(|err| {
        eprintln!("mpris: {err}");
        Vec::new()
    });
    let bar: SharedBar = Rc::new(RefCell::new(Bar {
        workspaces: WorkspaceIndicator::new(&workspaces),
        media: Media::new(players),
        tray: Tray::new(),
        status: Status::new(network, volume),
        quick: QuickSettings::new(volume, Time::now()),
        clock: Clock::new(Time::now()),
        power: Power::new(),
        launcher: Launcher::new(),
    }));

    let tray: SharedTray = Rc::default();
    let view = Rc::clone(&bar);
    let bar_id = ui.open(move |frame: &mut Frame| view.borrow_mut().window(frame));
    let b = Rc::clone(&bar);
    let t = Rc::clone(&tray);
    ui.on_pointer(bar_id, move |ui, event| {
        let (workspace, volume) = {
            let mut bar = b.borrow_mut();
            (bar.workspaces.pointer_event(event), bar.status.pointer_event(event))
        };
        if let Some(id) = workspace
            && let Err(err) = hyprland::focus_workspace(id)
        {
            eprintln!("hyprland: {err}");
        }
        if let Some(change) = volume {
            audio::change_volume(ui.loop_handle(), change);
        }
        let panel = if b.borrow_mut().power.pointer_event(event) {
            Some(quick::Panel::Power)
        } else if status::clicked(&event) {
            Some(quick::Panel::Settings)
        } else {
            None
        };
        if let Some(panel) = panel {
            let actions = {
                let mut bar = b.borrow_mut();
                let actions = bar.quick.toggle(panel);
                if bar.quick.is_open() {
                    bar.media.dismiss();
                    bar.tray.close_menu();
                }
                actions
            };
            perform_quick(ui, &b, bar_id, actions);
        }
        let actions = {
            let mut bar = b.borrow_mut();
            let was_open = bar.media.card_is_open();
            let actions = bar.media.pill_event(event);
            if !was_open && bar.media.card_is_open() {
                bar.quick.close();
                bar.tray.close_menu();
            }
            actions
        };
        perform(ui, &b, bar_id, actions);
        let actions = b.borrow_mut().tray.pointer_event(event);
        if !actions.is_empty() {
            perform_tray(ui, &b, bar_id, &t, actions);
        }
    });
    if bar.borrow().media.quiet() {
        expire_later(&mut ui, &bar, bar_id);
    }

    let handle = event_loop.handle();
    let b = Rc::clone(&bar);
    let result = ipc::listen(&handle, move |ui, message| match Command::parse(message) {
        Some(command) => {
            command_launcher(ui, &b, bar_id, command);
            "ok".to_owned()
        }
        None => format!("unknown command: {message}"),
    });
    if let Err(err) = result {
        eprintln!("ipc: can't listen: {err}");
    }
    let b = Rc::clone(&bar);
    let result = hyprland::watch_workspaces(&handle, move |ui, state| {
        b.borrow_mut().workspaces.update(&state);
        ui.refresh(bar_id);
    });
    if let Err(err) = result {
        eprintln!("hyprland: can't watch workspaces: {err}");
    }
    let b = Rc::clone(&bar);
    let result = network::watch_network(&handle, move |ui, state| {
        b.borrow_mut().status.set_network(state);
        ui.refresh(bar_id);
        settle_later(ui, &b, bar_id);
    });
    if let Err(err) = result {
        eprintln!("network: can't watch: {err}");
    }
    let b = Rc::clone(&bar);
    let result = audio::watch_volume(&handle, move |ui, volume| {
        {
            let mut bar = b.borrow_mut();
            bar.status.set_volume(volume);
            bar.quick.set_volume(volume);
        }
        refresh(ui, &b, bar_id);
        settle_later(ui, &b, bar_id);
    });
    if let Err(err) = result {
        eprintln!("audio: can't watch: {err}");
    }
    let b = Rc::clone(&bar);
    let t = Rc::clone(&tray);
    let result = tray_service::watch(&handle, move |ui, update| match update {
        tray_service::Update::Items(items) => {
            b.borrow_mut().tray.set_items(items);
            refresh(ui, &b, bar_id);
        }
        tray_service::Update::Menu { key, entries } => {
            let (center, width) = t.borrow().anchor;
            let actions = {
                let mut bar = b.borrow_mut();
                let actions = bar.tray.show_menu(key, entries, center, width);
                if bar.tray.menu_is_open() {
                    bar.media.dismiss();
                    bar.quick.close();
                }
                actions
            };
            perform_tray(ui, &b, bar_id, &t, actions);
        }
    });
    match result {
        Ok(service) => tray.borrow_mut().service = Some(service),
        Err(err) => eprintln!("tray: can't keep it: {err}"),
    }
    let b = Rc::clone(&bar);
    let result = mpris::watch_players(&handle, move |ui, players| {
        b.borrow_mut().media.set_players(players);
        refresh(ui, &b, bar_id);
        if b.borrow().media.quiet() {
            expire_later(ui, &b, bar_id);
        }
    });
    if let Err(err) = result {
        eprintln!("mpris: can't watch: {err}");
    }
    let result = watch_minutes(&handle, move |ui, time| {
        {
            let mut bar = bar.borrow_mut();
            bar.clock.set_time(time);
            bar.quick.set_time(time);
            bar.power.tick();
        }
        // Refreshing may build the view at once, which borrows the bar itself.
        ui.refresh(bar_id);
    });
    if let Err(err) = result {
        eprintln!("clock: can't watch: {err}");
    }

    event_loop.run(None, &mut ui, |_| {})?;
    Ok(())
}

/// Rebuilds the bar and, if they're open, the windows of the media card and the quick
/// settings: they show the same state.
fn refresh(ui: &mut Ui, bar: &SharedBar, bar_id: WindowId) {
    ui.refresh(bar_id);
    let windows: Vec<WindowId> = {
        let bar = bar.borrow();
        bar.media
            .card_windows()
            .chain(bar.quick.windows())
            .chain(bar.tray.windows())
            .chain(bar.launcher.window())
            .collect()
    };
    for window in windows {
        ui.refresh(window);
    }
}

/// Does what the media widget asked for in answer to the pointer, and shows the result.
fn perform(ui: &mut Ui, bar: &SharedBar, bar_id: WindowId, actions: Vec<Action>) {
    // What the pointer does on either of the card's windows is the card's to answer.
    let listen = |ui: &mut Ui, window: WindowId| {
        let bar = Rc::clone(bar);
        ui.on_pointer(window, move |ui, event| {
            let actions = bar.borrow_mut().media.card_event(event);
            perform(ui, &bar, bar_id, actions);
        });
    };
    for action in actions {
        match action {
            Action::OpenScrim => {
                let view = Rc::clone(bar);
                let scrim = ui.open(move |frame: &mut Frame| view.borrow_mut().media.scrim(frame));
                bar.borrow_mut().media.set_scrim_window(scrim);
                listen(ui, scrim);
            }
            Action::OpenCard => {
                let view = Rc::clone(bar);
                let card = ui.open(move |frame: &mut Frame| view.borrow_mut().media.card(frame));
                bar.borrow_mut().media.set_card_window(card);
                listen(ui, card);
            }
            Action::Send(player, command) => mpris::send(ui.loop_handle(), &player, command),
        }
    }
    refresh(ui, bar, bar_id);
}

/// Does what the quick settings asked for in answer to the pointer, and shows the result.
fn perform_quick(ui: &mut Ui, bar: &SharedBar, bar_id: WindowId, actions: Vec<quick::Action>) {
    // What the pointer does on either of the panel's windows is the panel's to answer.
    let listen = |ui: &mut Ui, window: WindowId| {
        let bar = Rc::clone(bar);
        ui.on_pointer(window, move |ui, event| {
            let actions = bar.borrow_mut().quick.event(event);
            perform_quick(ui, &bar, bar_id, actions);
        });
    };
    for action in actions {
        match action {
            quick::Action::OpenScrim => {
                let view = Rc::clone(bar);
                let scrim = ui.open(move |frame: &mut Frame| view.borrow_mut().quick.scrim(frame));
                bar.borrow_mut().quick.set_scrim_window(scrim);
                listen(ui, scrim);
            }
            quick::Action::OpenPanel => {
                let view = Rc::clone(bar);
                let panel = ui.open(move |frame: &mut Frame| view.borrow_mut().quick.panel(frame));
                bar.borrow_mut().quick.set_panel_window(panel);
                listen(ui, panel);
            }
            quick::Action::Query => query_quick(ui, bar, bar_id),
            quick::Action::Switch(switch) => {
                let bar = Rc::clone(bar);
                quick_settings::switch(ui.loop_handle(), switch, move |ui| {
                    query_quick(ui, &bar, bar_id);
                });
            }
            quick::Action::SetVolume(level) => audio::set_volume(ui.loop_handle(), level),
        }
    }
    refresh(ui, bar, bar_id);
}

/// Does what the tray asked for in answer to the pointer, and shows the result.
fn perform_tray(
    ui: &mut Ui,
    bar: &SharedBar,
    bar_id: WindowId,
    tray: &SharedTray,
    actions: Vec<tray::Action>,
) {
    // What the pointer does on either of the menu's windows is the menu's to answer.
    let listen = |ui: &mut Ui, window: WindowId| {
        let (bar, tray) = (Rc::clone(bar), Rc::clone(tray));
        ui.on_pointer(window, move |ui, event| {
            let actions = bar.borrow_mut().tray.menu_event(event);
            perform_tray(ui, &bar, bar_id, &tray, actions);
        });
    };
    // The bar is at the screen's top-left corner and as wide as it: a place on it is a
    // place on the screen.
    let (x, y) = match ui.pointer_position() {
        Some((window, x, y)) if window == bar_id => (x, y),
        _ => (0.0, 0.0),
    };
    let Some(service) = tray.borrow().service.clone() else { return };
    for action in actions {
        match action {
            tray::Action::Activate(key) => service.activate(&key, x as i32, y as i32),
            tray::Action::SecondaryActivate(key) => {
                service.secondary_activate(&key, x as i32, y as i32);
            }
            tray::Action::ContextMenu(key) => service.context_menu(&key, x as i32, y as i32),
            tray::Action::Scroll(key, steps) => service.scroll(&key, steps),
            tray::Action::RequestMenu(key) => {
                // Asking again for the menu that is open closes it.
                if bar.borrow().tray.menu_is_open_for(&key) {
                    bar.borrow_mut().tray.close_menu();
                    continue;
                }
                let width = ui.window_size(bar_id).map_or(0.0, |(width, _)| width as f32);
                tray.borrow_mut().anchor = (width - x, width);
                service.open_menu(&key);
            }
            tray::Action::Choose(key, id) => service.choose(&key, id),
            tray::Action::OpenScrim => {
                let view = Rc::clone(bar);
                let scrim = ui.open(move |frame: &mut Frame| view.borrow_mut().tray.scrim(frame));
                bar.borrow_mut().tray.set_scrim_window(scrim);
                listen(ui, scrim);
            }
            tray::Action::OpenMenu => {
                let view = Rc::clone(bar);
                let menu = ui.open(move |frame: &mut Frame| view.borrow_mut().tray.menu(frame));
                bar.borrow_mut().tray.set_menu_window(menu);
                listen(ui, menu);
            }
        }
    }
    refresh(ui, bar, bar_id);
}

/// Finds out how what the quick settings switch stands, and shows it.
fn query_quick(ui: &mut Ui, bar: &SharedBar, bar_id: WindowId) {
    let bar = Rc::clone(bar);
    let (id, include_devices) = bar.borrow_mut().quick.begin_query();
    quick_settings::query(ui.loop_handle(), include_devices, move |ui, state| {
        bar.borrow_mut().quick.set_state(id, state);
        refresh(ui, &bar, bar_id);
    });
}

/// Opens or closes the launcher, as asked from outside. Opening it closes what the bar
/// has open.
fn command_launcher(ui: &mut Ui, bar: &SharedBar, bar_id: WindowId, command: Command) {
    let needs_window = {
        let mut bar = bar.borrow_mut();
        let needs_window = match command {
            Command::LauncherOpen => bar.launcher.open(),
            Command::LauncherClose => {
                bar.launcher.close();
                false
            }
            Command::LauncherToggle => bar.launcher.toggle(),
        };
        if bar.launcher.is_open() {
            bar.media.dismiss();
            bar.quick.close();
            bar.tray.close_menu();
        }
        needs_window
    };
    if needs_window {
        let view = Rc::clone(bar);
        let window = ui.open(move |frame: &mut Frame| view.borrow_mut().launcher.view(frame));
        bar.borrow_mut().launcher.set_window(window);
        let b = Rc::clone(bar);
        ui.on_pointer(window, move |ui, event| {
            b.borrow_mut().launcher.pointer_event(event);
            ui.refresh(window);
        });
        let b = Rc::clone(bar);
        ui.on_key(window, move |ui, event| {
            b.borrow_mut().launcher.key_event(event);
            ui.refresh(window);
        });
    }
    refresh(ui, bar, bar_id);
}

/// Has the media pill leave once nothing has played for [`media::LINGER`].
fn expire_later(ui: &mut Ui, bar: &SharedBar, bar_id: WindowId) {
    let bar = Rc::clone(bar);
    // A little late, so that its time is surely up.
    let timer = Timer::from_duration(media::LINGER + Duration::from_millis(20));
    let result = ui.loop_handle().insert_source(timer, move |_, _, ui| {
        bar.borrow_mut().media.expire();
        refresh(ui, &bar, bar_id);
        TimeoutAction::Drop
    });
    if let Err(err) = result {
        eprintln!("media: can't set a timer: {err}");
    }
}

/// Has the status segments that just spoke up close again, once they've been open for
/// [`status::HOLD`].
fn settle_later(ui: &mut Ui, bar: &SharedBar, bar_id: WindowId) {
    let bar = Rc::clone(bar);
    // A little late, so that their time is surely up.
    let timer = Timer::from_duration(status::HOLD + Duration::from_millis(20));
    let result = ui.loop_handle().insert_source(timer, move |_, _, ui| {
        bar.borrow_mut().status.settle();
        ui.refresh(bar_id);
        TimeoutAction::Drop
    });
    if let Err(err) = result {
        eprintln!("status: can't set a timer: {err}");
    }
}

/// `$XDG_CONFIG_HOME/shell/theme.json` (or `~/.config/...`): matugen output, generated
/// with e.g. `matugen image wallpaper.png --json hex > ~/.config/shell/theme.json` or by
/// the template in `matugen/`. Changes to it are applied live.
fn theme_path() -> Option<PathBuf> {
    let config_dir = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config_dir.join("shell/theme.json"))
}
