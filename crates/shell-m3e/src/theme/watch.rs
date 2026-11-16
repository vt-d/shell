//! Live theme reloading.

use std::{
    error::Error,
    fs, io,
    path::{Path, PathBuf},
};

use inotify::{Inotify, WatchMask};
use shell_ui::{
    Ui,
    calloop::{Interest, Mode, PostAction, generic::Generic},
};

use super::Theme;

/// Uses the matugen output at `path` as the UI's theme, now and whenever the file is
/// written or replaced (as matugen does on each wallpaper change), by
/// [providing](Ui::provide) it. Until the file exists, and whenever it can't be parsed,
/// the current theme stays and the problem is reported on stderr.
///
/// The file's directory is created if missing, since that is what's watched: tools
/// often replace a file rather than rewrite it, which would end a watch on the file.
pub fn watch_theme(ui: &mut Ui, path: impl Into<PathBuf>) -> Result<(), Box<dyn Error>> {
    let path = path.into();
    let name = path.file_name().ok_or("theme path has no file name")?.to_owned();
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_owned(),
        _ => PathBuf::from("."),
    };
    fs::create_dir_all(&dir)?;
    let inotify = Inotify::init()?;
    // A write ends with CLOSE_WRITE; a replacement arrives as MOVED_TO.
    inotify.watches().add(&dir, WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO)?;

    if path.exists() {
        reload(ui, &path);
    }
    let source = Generic::new(inotify, Interest::READ, Mode::Level);
    ui.loop_handle()
        .insert_source(source, move |_, inotify, ui| {
            // SAFETY: the inotify instance is only read from, never replaced or dropped.
            let inotify = unsafe { inotify.get_mut() };
            let mut buffer = [0; 4096];
            let mut changed = false;
            loop {
                match inotify.read_events(&mut buffer) {
                    Ok(events) => {
                        changed |= events.into_iter().any(|e| e.name == Some(name.as_os_str()))
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e),
                }
            }
            if changed {
                reload(ui, &path);
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| e.error)?;
    Ok(())
}

fn reload(ui: &mut Ui, path: &Path) {
    match Theme::load(path) {
        Ok(theme) => ui.provide(theme),
        Err(err) => eprintln!("{}: {err}; keeping the current theme", path.display()),
    }
}
