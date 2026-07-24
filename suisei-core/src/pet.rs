//! Desktop **pet** — looping GIF/PNG overlay (Kitty graphics) placed at cell coords.
//!
//! Requires `gpu_acc` + Kitty/Ghostty graphics. Playback speed is a percent of
//! the GIF's native frame delay (`100` = 1×, `200` = 2× faster, `50` = half).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::AnimationDecoder;

/// One decoded RGBA frame.
#[derive(Clone)]
pub struct PetFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub delay: Duration,
}

/// Pre-scaled display frame for the Kitty hot path.
#[derive(Clone)]
pub struct PetDisplayFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Precomputed base64 — avoids re-encoding every tick (major stutter source).
    pub b64: String,
}

pub struct PetState {
    pub enabled: bool,
    pub path: String,
    /// Cell column / row (top-left of image).
    pub x: u16,
    pub y: u16,
    /// Display width in terminal cells (height scales with aspect).
    pub width_cells: u16,
    /// Playback speed percent (25..=400). 100 = native GIF timing.
    pub speed: u16,
    frames: Vec<PetFrame>,
    /// Cached resize+base64 at (`cache_width_cells`, `cache_cell_px`).
    display: Vec<PetDisplayFrame>,
    cache_width_cells: u16,
    cache_cell_px: u32,
    frame_idx: usize,
    last_tick: Instant,
    pub load_error: Option<String>,
    /// Kitty image id (stable so we replace rather than leak).
    pub image_id: u32,
}

impl Default for PetState {
    fn default() -> Self {
        Self {
            enabled: false,
            path: String::new(),
            x: 2,
            y: 2,
            width_cells: 12,
            speed: 100,
            frames: Vec::new(),
            display: Vec::new(),
            cache_width_cells: 0,
            cache_cell_px: 0,
            frame_idx: 0,
            last_tick: Instant::now(),
            load_error: None,
            image_id: 77,
        }
    }
}

impl PetState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_frames(&self) -> bool {
        !self.frames.is_empty()
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn frame_idx(&self) -> usize {
        self.frame_idx
    }

    /// Clamp speed to a sensible range.
    pub fn clamp_speed(s: u16) -> u16 {
        s.clamp(25, 400)
    }

    /// Load GIF (or static PNG) from disk.
    pub fn load_path(&mut self, path: &str) {
        self.path = path.to_string();
        self.frames.clear();
        self.display.clear();
        self.cache_width_cells = 0;
        self.cache_cell_px = 0;
        self.frame_idx = 0;
        self.load_error = None;
        if path.is_empty() {
            return;
        }
        match load_frames(Path::new(path)) {
            Ok(frames) if !frames.is_empty() => {
                self.frames = frames;
                self.last_tick = Instant::now();
            }
            Ok(_) => self.load_error = Some("no frames".into()),
            Err(e) => self.load_error = Some(e),
        }
    }

    /// Invalidate display cache (call when width_cells changes).
    pub fn invalidate_display_cache(&mut self) {
        self.display.clear();
        self.cache_width_cells = 0;
        self.cache_cell_px = 0;
    }

    /// Build / refresh pre-scaled + base64 frames for the Kitty paint path.
    pub fn ensure_display_cache(&mut self, cell_px: u32) {
        let cell_px = cell_px.max(8);
        if self.cache_width_cells == self.width_cells
            && self.cache_cell_px == cell_px
            && self.display.len() == self.frames.len()
            && !self.display.is_empty()
        {
            return;
        }
        self.display.clear();
        let target_w = (self.width_cells as u32).saturating_mul(cell_px).max(8);
        for f in &self.frames {
            let (tw, th) = if f.width == 0 {
                (target_w, target_w)
            } else {
                let h = (target_w as u64 * f.height as u64 / f.width as u64).max(1) as u32;
                (target_w, h)
            };
            let rgba = resize_rgba(f, tw, th);
            let b64 = encode_base64_local(&rgba);
            self.display.push(PetDisplayFrame {
                width: tw,
                height: th,
                rgba,
                b64,
            });
        }
        self.cache_width_cells = self.width_cells;
        self.cache_cell_px = cell_px;
    }

    /// Effective delay for the current frame after speed scaling.
    pub fn effective_delay(&self) -> Duration {
        let base = self
            .frames
            .get(self.frame_idx)
            .map(|f| f.delay)
            .unwrap_or(Duration::from_millis(100));
        let speed = Self::clamp_speed(self.speed).max(1) as u128;
        // higher speed → shorter delay; floor 12ms for snappy loops
        let ms = (base.as_millis() * 100 / speed).max(12) as u64;
        Duration::from_millis(ms)
    }

    /// Advance animation clock; returns true if the visible frame changed.
    /// Uses catch-up so heavy frames don't permanently lag.
    pub fn tick(&mut self) -> bool {
        if self.frames.len() <= 1 {
            return false;
        }
        let delay = self.effective_delay();
        if delay.is_zero() {
            return false;
        }
        let now = Instant::now();
        if now.duration_since(self.last_tick) < delay {
            return false;
        }
        // Walk the clock forward in delay steps (stable phase, no Instant::now reset jank).
        let mut steps = 0usize;
        let max_steps = self.frames.len().saturating_mul(2).max(1);
        while self.last_tick + delay <= now && steps < max_steps {
            self.last_tick += delay;
            steps += 1;
        }
        if steps == 0 {
            return false;
        }
        self.frame_idx = (self.frame_idx + steps) % self.frames.len();
        true
    }

    pub fn current_frame(&self) -> Option<&PetFrame> {
        self.frames.get(self.frame_idx)
    }

    pub fn current_display(&self) -> Option<&PetDisplayFrame> {
        self.display.get(self.frame_idx)
    }

    /// Pixel size when fitting into `width_cells` × ~cell_px.
    pub fn display_px(&self, cell_px: u32) -> (u32, u32) {
        if let Some(d) = self.current_display() {
            return (d.width, d.height);
        }
        let Some(f) = self.current_frame() else {
            return (0, 0);
        };
        let target_w = (self.width_cells as u32).saturating_mul(cell_px).max(8);
        if f.width == 0 {
            return (target_w, target_w);
        }
        let h = (target_w as u64 * f.height as u64 / f.width as u64).max(1) as u32;
        (target_w, h)
    }

    /// Clamp **for painting only** — never mutates saved `x`/`y`.
    ///
    /// Startup used to clamp into the default 80×24 before the first draw,
    /// permanently wiping a bottom-right config until the next Settings save.
    pub fn screen_xy(&self, screen_w: u16, screen_h: u16) -> (u16, u16) {
        if screen_w < 4 || screen_h < 3 {
            return (self.x, self.y);
        }
        let max_x = screen_w.saturating_sub(self.width_cells.max(1));
        let max_y = screen_h.saturating_sub(2);
        (self.x.min(max_x), self.y.min(max_y))
    }

    /// Human label for speed (e.g. `1.0×`).
    pub fn speed_label(speed: u16) -> String {
        let s = Self::clamp_speed(speed);
        format!("{:.2}×", s as f32 / 100.0)
    }

    /// ASCII slider for settings (10 segments).
    pub fn speed_slider(speed: u16, width: usize) -> String {
        let s = Self::clamp_speed(speed);
        // map 25..=400 → 0..width-1
        let t = ((s as u32 - 25) * (width.saturating_sub(1) as u32) / (400 - 25)).min(width.saturating_sub(1) as u32);
        let mut bar = String::with_capacity(width);
        for i in 0..width {
            if i as u32 == t {
                bar.push('●');
            } else if (i as u32) < t {
                bar.push('━');
            } else {
                bar.push('─');
            }
        }
        bar
    }
}

fn load_frames(path: &Path) -> Result<Vec<PetFrame>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "gif" {
        let dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(data))
            .map_err(|e| e.to_string())?;
        let mut frames = Vec::new();
        for fr in dec.into_frames() {
            let fr = fr.map_err(|e| e.to_string())?;
            let delay: Duration = fr.delay().into();
            let delay = if delay.as_millis() < 20 {
                Duration::from_millis(100)
            } else {
                delay
            };
            let rgba = fr.into_buffer();
            let (w, h) = rgba.dimensions();
            frames.push(PetFrame {
                width: w,
                height: h,
                rgba: rgba.into_raw(),
                delay,
            });
        }
        Ok(frames)
    } else {
        // Static PNG / other
        let img = image::load_from_memory(&data).map_err(|e| e.to_string())?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        Ok(vec![PetFrame {
            width: w,
            height: h,
            rgba: rgba.into_raw(),
            delay: Duration::from_secs(3600),
        }])
    }
}

/// Bilinear resize of RGBA to target size (nearest looked blocky once the
/// terminal rescaled to real cell pixels).
pub fn resize_rgba(src: &PetFrame, tw: u32, th: u32) -> Vec<u8> {
    let (sw, sh) = (src.width.max(1), src.height.max(1));
    let mut out = vec![0u8; (tw * th * 4) as usize];
    if tw == 0 || th == 0 {
        return out;
    }
    let fx = sw as f32 / tw as f32;
    let fy = sh as f32 / th as f32;
    for y in 0..th {
        let sy = (y as f32 + 0.5) * fy - 0.5;
        let y0 = sy.floor().max(0.0) as u32;
        let y1 = (y0 + 1).min(sh - 1);
        let wy = (sy - y0 as f32).clamp(0.0, 1.0);
        for x in 0..tw {
            let sx = (x as f32 + 0.5) * fx - 0.5;
            let x0 = sx.floor().max(0.0) as u32;
            let x1 = (x0 + 1).min(sw - 1);
            let wx = (sx - x0 as f32).clamp(0.0, 1.0);
            let di = ((y * tw + x) * 4) as usize;
            for ch in 0..4 {
                let p = |px: u32, py: u32| -> f32 {
                    let i = ((py * sw + px) * 4) as usize + ch;
                    src.rgba.get(i).copied().unwrap_or(0) as f32
                };
                let top = p(x0, y0) * (1.0 - wx) + p(x1, y0) * wx;
                let bot = p(x0, y1) * (1.0 - wx) + p(x1, y1) * wx;
                out[di + ch] = (top * (1.0 - wy) + bot * wy).round() as u8;
            }
        }
    }
    out
}

/// Public base64 for media/pet display caches.
pub fn encode_b64_public(data: &[u8]) -> String {
    encode_base64_local(data)
}

fn encode_base64_local(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < data.len() {
        let rem = data.len() - i;
        let n = if rem == 1 {
            (data[i] as u32) << 16
        } else {
            ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8)
        };
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if rem == 1 {
            out.push('=');
            out.push('=');
        } else {
            out.push(T[((n >> 6) & 63) as usize] as char);
            out.push('=');
        }
    }
    out
}

pub fn expand_path(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) =
            std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))
        {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}
