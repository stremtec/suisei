//! `:screensaver` — **xeifetch**: neofetch-style splash with theme colors,
//! analog clock, and optional location weather emoji.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct WeatherInfo {
    pub city: String,
    pub emoji: String,
    pub label: String,
    pub temp_c: Option<f32>,
}

#[derive(Debug)]
pub struct Screensaver {
    pub open: bool,
    pub opened_at: Instant,
    pub weather: Option<WeatherInfo>,
    weather_rx: Option<Receiver<Result<WeatherInfo, String>>>,
    weather_started: bool,
    /// `/` opens cryptex password entry (easter egg).
    pub cryptex_input: bool,
    /// Typed password (not shown as plain in status — only on cryptex).
    pub cryptex_buf: String,
    /// Easter egg: all splash text becomes "god" until screensaver reopens.
    pub god_mode: bool,
}

impl Default for Screensaver {
    fn default() -> Self {
        Self {
            open: false,
            opened_at: Instant::now(),
            weather: None,
            weather_rx: None,
            weather_started: false,
            cryptex_input: false,
            cryptex_buf: String::new(),
            god_mode: false,
        }
    }
}

impl Screensaver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self) {
        self.open = true;
        self.opened_at = Instant::now();
        // Fresh session: easter egg and input always reset.
        self.cryptex_input = false;
        self.cryptex_buf.clear();
        self.god_mode = false;
        if !self.weather_started {
            self.start_weather_fetch();
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.cryptex_input = false;
        self.cryptex_buf.clear();
        // god_mode dies with the session (also cleared on next open).
        self.god_mode = false;
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// Start `/` cryptex input mode.
    pub fn begin_cryptex_input(&mut self) {
        self.cryptex_input = true;
        self.cryptex_buf.clear();
    }

    pub fn cancel_cryptex_input(&mut self) {
        self.cryptex_input = false;
        self.cryptex_buf.clear();
    }

    /// Push a character into the cryptex (max 16).
    pub fn cryptex_push(&mut self, c: char) {
        if self.cryptex_buf.chars().count() < 16 && !c.is_control() {
            self.cryptex_buf.push(c);
        }
    }

    pub fn cryptex_backspace(&mut self) {
        self.cryptex_buf.pop();
    }

    /// Submit password. Returns true if the easter egg unlocked.
    pub fn submit_cryptex(&mut self) -> bool {
        let ok = self.cryptex_buf.eq_ignore_ascii_case("fakers");
        self.cryptex_input = false;
        self.cryptex_buf.clear();
        if ok {
            self.god_mode = true;
        }
        ok
    }

    fn start_weather_fetch(&mut self) {
        self.weather_started = true;
        let (tx, rx) = mpsc::channel();
        self.weather_rx = Some(rx);
        thread::spawn(move || {
            let _ = tx.send(fetch_weather());
        });
    }

    /// Poll background weather (non-blocking).
    pub fn poll(&mut self) {
        let Some(rx) = self.weather_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(w)) => {
                self.weather = Some(w);
                self.weather_rx = None;
            }
            Ok(Err(_)) => {
                self.weather_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.weather_rx = None;
            }
        }
    }
}

/// Local wall-clock (H, M, S). Uses `date` so we avoid extra time crates.
pub fn local_hms() -> (u32, u32, u32) {
    let s = std::process::Command::new("date")
        .arg("+%H %M %S")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let mut parts = s.split_whitespace();
    let h = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let m = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let sec = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    (h.min(23), m.min(59), sec.min(59))
}

/// System facts for xeifetch side panel.
pub fn system_info() -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".into());
    let host = hostname();
    rows.push(("user".into(), format!("{user}@{host}")));
    rows.push(("os".into(), os_pretty()));
    rows.push((
        "host".into(),
        host,
    ));
    rows.push((
        "shell".into(),
        std::env::var("SHELL")
            .ok()
            .and_then(|s| s.rsplit('/').next().map(|x| x.to_string()))
            .unwrap_or_else(|| "?".into()),
    ));
    rows.push((
        "term".into(),
        std::env::var("TERM").unwrap_or_else(|_| "?".into()),
    ));
    rows.push((
        "cwd".into(),
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into()),
    ));
    rows.push((
        "xei".into(),
        env!("CARGO_PKG_VERSION").into(),
    ));
    rows
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| {
            // BSD/macOS
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "localhost".into())
        })
}

fn os_pretty() -> String {
    if cfg!(target_os = "macos") {
        "macOS".into()
    } else if cfg!(target_os = "linux") {
        // Try /etc/os-release PRETTY_NAME
        if let Ok(s) = std::fs::read_to_string("/etc/os-release") {
            for line in s.lines() {
                if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
                    return v.trim_matches('"').to_string();
                }
            }
        }
        "Linux".into()
    } else if cfg!(target_os = "windows") {
        "Windows".into()
    } else {
        std::env::consts::OS.into()
    }
}

/// Display width of every cryptex line (fixed monospaced grid).
pub const CRYPTEX_WIDTH: usize = 13; // `┌─┬─┬─┬─┬─┬─┐`

/// Easter-egg display rewrite: every visible string becomes `god`.
pub fn godify(s: &str, god: bool) -> String {
    if god {
        "god".into()
    } else {
        s.to_string()
    }
}

/// Da Vinci Code–style **cryptex**: six rings (HH MM SS) in a fixed-width grid.
///
/// - `input`: password entry shows in the aperture
/// - `god`: easter egg — everything chants GOD
pub fn cryptex_lines(
    hour: u32,
    minute: u32,
    second: u32,
    spin: u64,
    input: Option<&str>,
    god: bool,
) -> Vec<String> {
    let d = [
        (hour / 10) % 10,
        hour % 10,
        (minute / 10) % 10,
        minute % 10,
        (second / 10) % 10,
        second % 10,
    ];

    let ch = |n: u32| char::from(b'0' + (n % 10) as u8);
    let far_a: [char; 6] = std::array::from_fn(|i| ch(d[i] + 8));
    let near_a: [char; 6] = std::array::from_fn(|i| ch(d[i] + 9));
    let mut mid: [char; 6] = std::array::from_fn(|i| ch(d[i]));
    let near_b: [char; 6] = std::array::from_fn(|i| ch(d[i] + 1));
    let far_b: [char; 6] = std::array::from_fn(|i| ch(d[i] + 2));

    if let Some(buf) = input {
        // Underscore fill (avoid `'_'` which Rust parses as a lifetime).
        mid = ['\x5f', '\x5f', '\x5f', '\x5f', '\x5f', '\x5f'];
        for (i, c) in buf.chars().take(6).enumerate() {
            mid[i] = if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '·'
            };
        }
    }

    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let phase = ((spin / 140) % 26) as usize;
    let letter_band = |offset: usize| -> [char; 6] {
        std::array::from_fn(|i| ALPHA[(phase + offset + i * 4) % 26] as char)
    };
    let mut top_alpha = letter_band(0);
    let mut bot_alpha = letter_band(11);

    if god {
        top_alpha = ['G', 'O', 'D', 'G', 'O', 'D'];
        bot_alpha = ['G', 'O', 'D', 'G', 'O', 'D'];
        mid = ['G', 'O', 'D', 'G', 'O', 'D'];
        let g = |i: usize| ['g', 'o', 'd', 'g', 'o', 'd'][i];
        return assemble_cryptex(
            top_alpha,
            bot_alpha,
            std::array::from_fn(g),
            std::array::from_fn(g),
            mid,
            std::array::from_fn(g),
            std::array::from_fn(g),
            true,
        );
    }

    assemble_cryptex(
        top_alpha, bot_alpha, far_a, near_a, mid, near_b, far_b, false,
    )
}

fn assemble_cryptex(
    top_alpha: [char; 6],
    bot_alpha: [char; 6],
    far_a: [char; 6],
    near_a: [char; 6],
    mid: [char; 6],
    near_b: [char; 6],
    far_b: [char; 6],
    god: bool,
) -> Vec<String> {
    fn row(cells: &[char; 6]) -> String {
        format!(
            "│{}│{}│{}│{}│{}│{}│",
            cells[0], cells[1], cells[2], cells[3], cells[4], cells[5]
        )
    }

    let title = {
        let t = if god { "god" } else { "cryptex" };
        let pad = CRYPTEX_WIDTH.saturating_sub(t.len());
        let left = pad / 2;
        let right = pad - left;
        format!("{}{}{}", " ".repeat(left), t, " ".repeat(right))
    };

    let labels = if god {
        " g o d g o d ".to_string()
    } else {
        " H H M M S S ".to_string()
    };

    let lines = vec![
        title,
        "┌─┬─┬─┬─┬─┬─┐".into(),
        row(&top_alpha),
        "├─┼─┼─┼─┼─┼─┤".into(),
        row(&far_a),
        row(&near_a),
        row(&mid),
        row(&near_b),
        row(&far_b),
        "├─┼─┼─┼─┼─┼─┤".into(),
        row(&bot_alpha),
        "└─┴─┴─┴─┴─┴─┘".into(),
        labels,
    ];

    debug_assert!(lines.iter().all(|l| l.chars().count() == CRYPTEX_WIDTH));
    lines
}

/// @deprecated alias — use [`cryptex_lines`].
pub fn analog_clock_grid(hour: u32, minute: u32, second: u32) -> Vec<String> {
    cryptex_lines(hour, minute, second, 0, None, false)
}

fn fetch_weather() -> Result<WeatherInfo, String> {
    // 1) IP geo (city + lat/lon)
    let geo = http_get("https://ipapi.co/json/")?;
    let city = json_str(&geo, "city").unwrap_or_else(|| "somewhere".into());
    let lat: f64 = json_num(&geo, "latitude").ok_or("no lat")?;
    let lon: f64 = json_num(&geo, "longitude").ok_or("no lon")?;

    // 2) Open-Meteo current weather (no API key)
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}&current=temperature_2m,weather_code"
    );
    let wx = http_get(&url)?;
    let code = json_num(&wx, "weather_code")
        .or_else(|| {
            // nested current.weather_code
            wx.find("\"weather_code\":")
                .and_then(|i| {
                    let rest = &wx[i + 15..];
                    let n: String = rest
                        .chars()
                        .skip_while(|c| !c.is_ascii_digit() && *c != '-')
                        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                        .collect();
                    n.parse().ok()
                })
        })
        .unwrap_or(0.0) as i32;
    let temp = json_num(&wx, "temperature_2m").or_else(|| {
        wx.find("\"temperature_2m\":").and_then(|i| {
            let rest = &wx[i + 17..];
            let n: String = rest
                .chars()
                .skip_while(|c| !c.is_ascii_digit() && *c != '-')
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            n.parse().ok()
        })
    });

    let (emoji, label) = wmo_emoji(code);
    Ok(WeatherInfo {
        city,
        emoji: emoji.into(),
        label: label.into(),
        temp_c: temp.map(|t| t as f32),
    })
}

fn wmo_emoji(code: i32) -> (&'static str, &'static str) {
    match code {
        0 => ("☀️", "clear"),
        1 | 2 => ("🌤", "mostly clear"),
        3 => ("☁️", "overcast"),
        45 | 48 => ("🌫", "fog"),
        51 | 53 | 55 | 56 | 57 => ("🌦", "drizzle"),
        61 | 63 | 65 | 66 | 67 => ("🌧", "rain"),
        71 | 73 | 75 | 77 => ("❄️", "snow"),
        80 | 81 | 82 => ("🌧", "showers"),
        85 | 86 => ("🌨", "snow showers"),
        95 | 96 | 99 => ("⛈", "thunder"),
        _ => ("🌡", "weather"),
    }
}

fn http_get(url: &str) -> Result<String, String> {
    // Prefer curl (always on macOS/dev machines); no extra HTTP crate.
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "8", url])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("curl failed".into());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

fn json_str(body: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let i = body.find(&pat)?;
    let rest = &body[i + pat.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn json_num(body: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{key}\":");
    let i = body.find(&pat)?;
    let rest = &body[i + pat.len()..];
    let n: String = rest
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    n.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cryptex_shows_time_digits() {
        let g = cryptex_lines(14, 32, 5, 0, None, false);
        assert!(g.len() >= 10);
        for (i, line) in g.iter().enumerate() {
            assert_eq!(
                line.chars().count(),
                CRYPTEX_WIDTH,
                "line {i} width mismatch: {line:?}"
            );
        }
        let aperture = &g[6];
        assert!(aperture.contains('1') && aperture.contains('4'));
        assert!(g[0].contains("cryptex"));
    }

    #[test]
    fn cryptex_fakers_god_mode() {
        let g = cryptex_lines(0, 0, 0, 0, None, true);
        assert!(g[0].contains("god"));
        assert!(g[6].contains('G') && g[6].contains('O') && g[6].contains('D'));
    }

    #[test]
    fn cryptex_input_aperture() {
        let g = cryptex_lines(0, 0, 0, 0, Some("ab"), false);
        assert!(g[6].contains('A') && g[6].contains('B'));
    }

    #[test]
    fn submit_fakers() {
        let mut ss = Screensaver::new();
        ss.open();
        ss.begin_cryptex_input();
        for c in "fakers".chars() {
            ss.cryptex_push(c);
        }
        assert!(ss.submit_cryptex());
        assert!(ss.god_mode);
        ss.close();
        assert!(!ss.god_mode);
    }

    #[test]
    fn wmo_clear() {
        assert_eq!(wmo_emoji(0).0, "☀️");
    }
}
