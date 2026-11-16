//! The weather outside, from [wttr.in](https://wttr.in): its temperature and conditions
//! now, and whether the sun is up.
//!
//! wttr.in needs no account or key, and by default reports for where the request comes
//! from (located by IP address). `SHELL_WEATHER_LOCATION` names a place instead, as
//! wttr.in takes it: a city (`London`), coordinates (`51.5,-0.12`) or an airport code.
//!
//! Requests go through `curl` in the background, every 15 minutes, and again after 2
//! minutes if one fails (offline, say).

use std::{
    cell::{Cell, RefCell},
    env,
    error::Error,
    rc::Rc,
    time::{Duration, Instant},
};

use calloop::{
    LoopHandle,
    timer::{TimeoutAction, Timer},
};
use serde_json::Value;

use crate::{clock::Time, process};

/// How often the weather is fetched.
const REFRESH: Duration = Duration::from_secs(15 * 60);
/// How soon to try again after a failed fetch.
const RETRY: Duration = Duration::from_secs(2 * 60);

/// The weather now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Weather {
    /// Degrees Celsius.
    pub temperature: i16,
    pub condition: Condition,
    /// Whether the sun is up.
    pub day: bool,
}

/// What the sky is doing, in as much detail as an icon shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    HeavyRain,
    Sleet,
    Snow,
    Thunderstorm,
}

impl Condition {
    /// From a World Weather Online condition code, which wttr.in reports.
    fn from_code(code: u16) -> Self {
        match code {
            113 => Self::Clear,
            116 => Self::PartlyCloudy,
            119 | 122 => Self::Cloudy,
            143 | 248 | 260 => Self::Fog,
            176 | 263 | 266 | 293 | 296 | 353 => Self::Drizzle,
            299 | 302 | 356 => Self::Rain,
            305 | 308 | 359 => Self::HeavyRain,
            182 | 185 | 281 | 284 | 311 | 314 | 317 | 320 | 350 | 362 | 365 | 374 | 377 => {
                Self::Sleet
            }
            179 | 227 | 230 | 323 | 326 | 329 | 332 | 335 | 338 | 368 | 371 => Self::Snow,
            200 | 386 | 389 | 392 | 395 => Self::Thunderstorm,
            _ => Self::Cloudy,
        }
    }
}

impl Weather {
    /// Reads wttr.in's JSON (`?format=j1` or `j2`), judging day or night at `now`.
    pub fn parse(json: &[u8], now: Time) -> Result<Self, Box<dyn Error>> {
        let root: Value = serde_json::from_slice(json)?;
        let current = &root["current_condition"][0];
        let field = |name: &str| current[name].as_str().ok_or_else(|| format!("no {name}"));
        let temperature = field("temp_C")?.trim().parse()?;
        let condition = Condition::from_code(field("weatherCode")?.trim().parse()?);

        let astronomy = &root["weather"][0]["astronomy"][0];
        let sun = |name: &str| astronomy[name].as_str().and_then(clock_minutes);
        let (sunrise, sunset) =
            (sun("sunrise").unwrap_or(6 * 60), sun("sunset").unwrap_or(18 * 60));
        let day = (sunrise..sunset).contains(&now.minutes());
        Ok(Self { temperature, condition, day })
    }
}

/// Minutes since midnight of a 12-hour time such as `"06:13 AM"`.
fn clock_minutes(text: &str) -> Option<u16> {
    let (time, half) = text.trim().split_once(' ')?;
    let (hour, minute) = time.split_once(':')?;
    let (hour, minute): (u16, u16) = (hour.parse().ok()?, minute.parse().ok()?);
    let hour = match half {
        "AM" => hour % 12,
        "PM" => hour % 12 + 12,
        _ => return None,
    };
    Some(hour * 60 + minute)
}

/// Fetches the weather now and every so often, calling `on_change` with it whenever it
/// differs from the last.
pub fn watch_weather<D: 'static>(
    handle: &LoopHandle<'static, D>,
    on_change: impl FnMut(&mut D, Weather) + 'static,
) -> Result<(), Box<dyn Error>> {
    let location = env::var("SHELL_WEATHER_LOCATION").unwrap_or_default().replace(' ', "+");
    let url = format!("https://wttr.in/{location}?format=j2");
    let on_change = Rc::new(RefCell::new(on_change));
    let last = Rc::new(Cell::new(None::<Weather>));
    // When the last fetch succeeded, and whether one is under way.
    let fetched = Rc::new(Cell::new(None::<Instant>));
    let busy = Rc::new(Cell::new(false));

    let handle_ = handle.clone();
    handle
        .insert_source(Timer::immediate(), move |_, _, _| {
            let due = fetched.get().is_none_or(|at| at.elapsed() >= REFRESH);
            if due && !busy.get() {
                busy.set(true);
                let (on_change, last, fetched, done) =
                    (on_change.clone(), last.clone(), fetched.clone(), busy.clone());
                let result = process::run(
                    &handle_,
                    "curl",
                    &["--silent", "--fail", "--location", "--max-time", "20", &url],
                    move |data, output| {
                        done.set(false);
                        match output.and_then(|json| Weather::parse(&json, Time::now())) {
                            Ok(weather) => {
                                fetched.set(Some(Instant::now()));
                                if last.replace(Some(weather)) != Some(weather) {
                                    (on_change.borrow_mut())(data, weather);
                                }
                            }
                            Err(err) => eprintln!("weather: {err}"),
                        }
                    },
                );
                if let Err(err) = result {
                    busy.set(false);
                    eprintln!("weather: can't run curl: {err}");
                }
            }
            // Checks often enough to retry soon after a failure; fetches only when due.
            TimeoutAction::ToDuration(RETRY)
        })
        .map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "current_condition": [{ "temp_C": "25", "weatherCode": "176" }],
        "weather": [{ "astronomy": [{ "sunrise": "06:13 AM", "sunset": "05:59 PM" }] }]
    }"#;

    fn at(hour: u8, minute: u8) -> Time {
        Time { hour, minute, ..Time::default() }
    }

    #[test]
    fn reads_wttr_json() {
        let weather = Weather::parse(SAMPLE.as_bytes(), at(13, 0)).unwrap();
        assert_eq!(weather, Weather { temperature: 25, condition: Condition::Drizzle, day: true });
        assert!(!Weather::parse(SAMPLE.as_bytes(), at(18, 0)).unwrap().day);
        assert!(!Weather::parse(SAMPLE.as_bytes(), at(6, 12)).unwrap().day);
        assert!(Weather::parse(b"{}", at(12, 0)).is_err());
    }

    #[test]
    fn reads_twelve_hour_times() {
        assert_eq!(clock_minutes("06:13 AM"), Some(6 * 60 + 13));
        assert_eq!(clock_minutes("12:05 AM"), Some(5));
        assert_eq!(clock_minutes("12:30 PM"), Some(12 * 60 + 30));
        assert_eq!(clock_minutes("05:59 PM"), Some(17 * 60 + 59));
        assert_eq!(clock_minutes("noon"), None);
    }
}
