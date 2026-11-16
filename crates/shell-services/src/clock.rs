//! Local date and time, ticking on each minute.

use std::{
    error::Error,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use calloop::{
    LoopHandle,
    timer::{TimeoutAction, Timer},
};

/// A local wall-clock time, to the minute, and the date it falls on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
    /// Day of the week, 0 for Sunday.
    pub weekday: u8,
    /// Day of the month, from 1.
    pub day: u8,
    /// Month, from 1 for January.
    pub month: u8,
}

impl Time {
    /// The time now, in the system's time zone (`TZ` or `/etc/localtime`).
    pub fn now() -> Self {
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let seconds = seconds as libc::time_t;
        // SAFETY: localtime_r only writes the `tm` it's given.
        let tm = unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&seconds, &mut tm);
            tm
        };
        Self {
            hour: tm.tm_hour as u8,
            minute: tm.tm_min as u8,
            weekday: tm.tm_wday as u8,
            day: tm.tm_mday as u8,
            month: tm.tm_mon as u8 + 1,
        }
    }

    /// The day of the week, abbreviated: `"Mon"`.
    pub fn weekday_name(&self) -> &'static str {
        const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        NAMES[self.weekday as usize % 7]
    }

    /// The month, abbreviated: `"Nov"`.
    pub fn month_name(&self) -> &'static str {
        const NAMES: [&str; 12] =
            ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        NAMES[(self.month as usize + 11) % 12]
    }

    /// Minutes since midnight.
    pub fn minutes(&self) -> u16 {
        self.hour as u16 * 60 + self.minute as u16
    }
}

impl std::fmt::Display for Time {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

/// How long until the next minute starts, plus a little so the tick lands inside it.
fn until_next_minute() -> Duration {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let into_minute = Duration::from_millis((now.as_millis() % 60_000) as u64);
    Duration::from_secs(60) - into_minute + Duration::from_millis(5)
}

/// Calls `on_tick` with the loop's data and the time at the start of every minute.
pub fn watch_minutes<D: 'static>(
    handle: &LoopHandle<'static, D>,
    mut on_tick: impl FnMut(&mut D, Time) + 'static,
) -> Result<(), Box<dyn Error>> {
    handle
        .insert_source(Timer::from_duration(until_next_minute()), move |_, _, data| {
            on_tick(data, Time::now());
            // Re-aligned every time, so the clock can't drift and survives suspend.
            TimeoutAction::ToDuration(until_next_minute())
        })
        .map_err(|e| e.error)?;
    Ok(())
}
