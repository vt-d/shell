//! System services the shell reflects, as calloop event sources.
//!
//! Each service reports changes by calling a closure from an event source on the
//! caller's calloop event loop, with that loop's data (in the shell, `shell_ui::Ui`), so
//! services know nothing about the UI. Services are event driven: they wait on the
//! system to say something changed rather than polling, except the weather, which has
//! to be fetched now and then.

pub mod apps;
pub mod audio;
pub mod clock;
pub mod hyprland;
pub mod ipc;
pub mod mpris;
pub mod network;
mod process;
pub mod quick;
pub mod tray;
pub mod weather;
