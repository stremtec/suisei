//! Media helpers for explorer/preview: images, CSV/NPY tables, audio playback.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::preview::{PreviewLine, PreviewStyle};

// ── Classification ──────────────────────────────────────────────────────

pub fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico"
    )
}

pub fn is_csv_ext(ext: &str) -> bool {
    matches!(ext.to_ascii_lowercase().as_str(), "csv" | "tsv")
}

pub fn is_npy_ext(ext: &str) -> bool {
    ext.eq_ignore_ascii_case("npy")
}

pub fn is_audio_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "aiff" | "wma" | "opus"
    )
}

pub fn is_media_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| is_image_ext(e) || is_csv_ext(e) || is_npy_ext(e) || is_audio_ext(e))
}

// ── Image ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ImageAsset {
    pub path: PathBuf,
    pub src_w: u32,
    pub src_h: u32,
    pub rgba: Vec<u8>,
    /// Display width in terminal cells (arrow keys adjust).
    pub width_cells: u16,
    pub cached_w: u32,
    pub cached_h: u32,
    pub cached_rgba: Vec<u8>,
    pub cached_b64: String,
    pub kitty_id: u32,
}

impl ImageAsset {
    pub fn load(path: &Path, cell_px: u32) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let img = image::load_from_memory(&data).map_err(|e| e.to_string())?;
        let rgba = img.to_rgba8();
        let (src_w, src_h) = rgba.dimensions();
        let mut asset = Self {
            path: path.to_path_buf(),
            src_w,
            src_h,
            rgba: rgba.into_raw(),
            width_cells: 48,
            cached_w: 0,
            cached_h: 0,
            cached_rgba: Vec::new(),
            cached_b64: String::new(),
            kitty_id: 88,
        };
        asset.rebuild_cache(cell_px);
        Ok(asset)
    }

    pub fn adjust_width(&mut self, delta: i16, cell_px: u32) {
        let w = self.width_cells as i16 + delta;
        self.width_cells = w.clamp(8, 120) as u16;
        self.rebuild_cache(cell_px);
    }

    pub fn rebuild_cache(&mut self, cell_px: u32) {
        let cell_px = cell_px.max(8);
        let tw = (self.width_cells as u32).saturating_mul(cell_px).max(8);
        let th = if self.src_w == 0 {
            tw
        } else {
            (tw as u64 * self.src_h as u64 / self.src_w as u64).max(1) as u32
        };
        let frame = RgbaFrame {
            width: self.src_w,
            height: self.src_h,
            rgba: self.rgba.clone(),
        };
        let out = resize_rgba(&frame, tw, th);
        self.cached_b64 = encode_b64(&out);
        self.cached_rgba = out;
        self.cached_w = tw;
        self.cached_h = th;
    }
}

/// One decoded RGBA frame (input for the preview resize path).
#[derive(Clone)]
struct RgbaFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Bilinear resize of RGBA to target size (nearest looked blocky once the
/// terminal rescaled to real cell pixels).
fn resize_rgba(src: &RgbaFrame, tw: u32, th: u32) -> Vec<u8> {
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

/// Base64 encode for the Kitty graphics payload cache.
fn encode_b64(data: &[u8]) -> String {
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

// ── CSV ─────────────────────────────────────────────────────────────────

pub fn render_csv(text: &str, tsv: bool) -> Vec<PreviewLine> {
    let sep = if tsv { '\t' } else { ',' };
    let mut out = Vec::new();
    out.push(pl(vec![(
        format!("  CSV/TSV table  ·  sep={sep:?}"),
        PreviewStyle::Dim,
    )]));
    out.push(pl(vec![("".into(), PreviewStyle::Normal)]));

    // Parse first, then size columns so the table actually lines up.
    const MAX_COLS: usize = 12;
    const MAX_CELL_W: usize = 24;
    let mut header: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for (i, line) in text.lines().take(200).enumerate() {
        let mut cols = split_csv_line(line, sep);
        cols.truncate(MAX_COLS);
        if i == 0 {
            header = cols;
        } else {
            rows.push(cols);
        }
    }
    let ncols = header
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    let cell_w = |s: &str| -> usize {
        s.chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
            .sum()
    };
    let mut widths = vec![0usize; ncols];
    for (c, w) in widths.iter_mut().enumerate() {
        *w = std::iter::once(&header)
            .chain(rows.iter())
            .filter_map(|r| r.get(c))
            .map(|s| cell_w(s).min(MAX_CELL_W))
            .max()
            .unwrap_or(1)
            .max(1);
    }
    let fmt_row = |row: &[String]| -> String {
        let mut s = String::from("  ");
        for (c, w) in widths.iter().enumerate() {
            let cell = row.get(c).map(|s| s.as_str()).unwrap_or("");
            // Clip to the column budget, then pad to it (width-aware).
            let mut taken = String::new();
            let mut used = 0usize;
            for ch in cell.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                if used + cw > *w {
                    if used < *w {
                        taken.push('…');
                        used += 1;
                    }
                    break;
                }
                taken.push(ch);
                used += cw;
            }
            s.push_str(&taken);
            s.push_str(&" ".repeat(w.saturating_sub(used)));
            if c + 1 < widths.len() {
                s.push_str(" │ ");
            }
        }
        s
    };
    if !header.is_empty() {
        out.push(pl(vec![(fmt_row(&header), PreviewStyle::H3)]));
        let rule: usize = widths.iter().sum::<usize>() + widths.len().saturating_sub(1) * 3;
        out.push(pl(vec![(
            format!("  {}", "─".repeat(rule.clamp(8, 200))),
            PreviewStyle::Hr,
        )]));
    }
    for (ri, row) in rows.iter().enumerate() {
        let style = if ri % 2 == 0 {
            PreviewStyle::Normal
        } else {
            PreviewStyle::Dim
        };
        out.push(pl(vec![(fmt_row(row), style)]));
    }
    if text.lines().count() > 200 {
        out.push(pl(vec![(
            "  … truncated (200 rows)".into(),
            PreviewStyle::Dim,
        )]));
    }
    if out.len() <= 2 {
        out.push(pl(vec![("(empty)".into(), PreviewStyle::Dim)]));
    }
    out
}

fn split_csv_line(line: &str, sep: char) -> Vec<String> {
    // Minimal CSV: honor quotes for commas
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if in_q && chars.peek() == Some(&'"') {
                cur.push('"');
                chars.next();
            } else {
                in_q = !in_q;
            }
        } else if c == sep && !in_q {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

// ── NPY (NumPy) ─────────────────────────────────────────────────────────

pub fn render_npy(path: &Path) -> Result<Vec<PreviewLine>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if data.len() < 10 || &data[0..6] != b"\x93NUMPY" {
        return Err("not a .npy file".into());
    }
    let major = data[6];
    let _minor = data[7];
    let (hdr_len, hdr_start) = if major == 1 {
        if data.len() < 10 {
            return Err("truncated npy".into());
        }
        let len = u16::from_le_bytes([data[8], data[9]]) as usize;
        (len, 10usize)
    } else {
        if data.len() < 12 {
            return Err("truncated npy".into());
        }
        let len = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        (len, 12usize)
    };
    let hdr_end = hdr_start + hdr_len;
    if data.len() < hdr_end {
        return Err("truncated npy header".into());
    }
    let header = String::from_utf8_lossy(&data[hdr_start..hdr_end]).to_string();
    let descr = npy_field(&header, "descr").unwrap_or_else(|| "?".into());
    let fortran = npy_field(&header, "fortran_order").unwrap_or_else(|| "False".into());
    let shape_s = npy_field(&header, "shape").unwrap_or_else(|| "()".into());
    let payload = &data[hdr_end..];

    let mut out = Vec::new();
    out.push(pl(vec![("  NumPy .npy".into(), PreviewStyle::H2)]));
    out.push(pl(vec![(format!("  dtype   {descr}"), PreviewStyle::Code)]));
    out.push(pl(vec![(
        format!("  shape   {shape_s}"),
        PreviewStyle::Code,
    )]));
    out.push(pl(vec![(
        format!("  fortran {fortran}  ·  payload {} bytes", payload.len()),
        PreviewStyle::Dim,
    )]));
    out.push(pl(vec![("".into(), PreviewStyle::Normal)]));

    // Pretty sample of values
    let sample = sample_npy_values(payload, &descr, 48);
    if sample.is_empty() {
        out.push(pl(vec![(
            "  (binary payload — no numeric sample)".into(),
            PreviewStyle::Dim,
        )]));
    } else {
        out.push(pl(vec![("  values (sample)".into(), PreviewStyle::H4)]));
        for chunk in sample.chunks(6) {
            let line = chunk.join("  ");
            out.push(pl(vec![(format!("  {line}"), PreviewStyle::JsonNumber)]));
        }
    }
    Ok(out)
}

fn npy_field(header: &str, key: &str) -> Option<String> {
    // header is a python dict-like string: {'descr': '<f8', 'fortran_order': False, 'shape': (2, 3), }
    let pat = format!("'{key}':");
    let i = header.find(&pat)?;
    let rest = header[i + pat.len()..].trim_start();
    if rest.starts_with('\'') {
        let rest = &rest[1..];
        let end = rest.find('\'')?;
        return Some(rest[..end].to_string());
    }
    if rest.starts_with('"') {
        let rest = &rest[1..];
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    // tuple — take through the closing paren (a bare `,` split would cut
    // `(2, 3)` down to `(2`)
    if rest.starts_with('(') {
        let end = rest.find(')')?;
        return Some(rest[..=end].to_string());
    }
    // bare True/False
    let end = rest
        .find(',')
        .or_else(|| rest.find('}'))
        .unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

fn sample_npy_values(payload: &[u8], descr: &str, n: usize) -> Vec<String> {
    let d = descr.trim();
    // e.g. <f8, >f4, <i4, |u1
    let is_le = d.starts_with('<') || d.starts_with('|') || !d.starts_with('>');
    let type_ch = d.chars().find(|c| c.is_ascii_alphabetic()).unwrap_or('f');
    let size: usize = d
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(4);

    let mut out = Vec::new();
    let mut off = 0;
    while out.len() < n && off + size <= payload.len() {
        let chunk = &payload[off..off + size];
        let s = match (type_ch, size) {
            ('f', 4) => {
                let mut b = [0u8; 4];
                b.copy_from_slice(chunk);
                let v = if is_le {
                    f32::from_le_bytes(b)
                } else {
                    f32::from_be_bytes(b)
                };
                format!("{v:.4}")
            }
            ('f', 8) => {
                let mut b = [0u8; 8];
                b.copy_from_slice(chunk);
                let v = if is_le {
                    f64::from_le_bytes(b)
                } else {
                    f64::from_be_bytes(b)
                };
                format!("{v:.4}")
            }
            ('i', 1) => format!("{}", chunk[0] as i8),
            ('i', 2) => {
                let mut b = [0u8; 2];
                b.copy_from_slice(chunk);
                let v = if is_le {
                    i16::from_le_bytes(b)
                } else {
                    i16::from_be_bytes(b)
                };
                format!("{v}")
            }
            ('i', 4) => {
                let mut b = [0u8; 4];
                b.copy_from_slice(chunk);
                let v = if is_le {
                    i32::from_le_bytes(b)
                } else {
                    i32::from_be_bytes(b)
                };
                format!("{v}")
            }
            ('i', 8) => {
                let mut b = [0u8; 8];
                b.copy_from_slice(chunk);
                let v = if is_le {
                    i64::from_le_bytes(b)
                } else {
                    i64::from_be_bytes(b)
                };
                format!("{v}")
            }
            ('u', 1) => format!("{}", chunk[0]),
            _ => format!("{:02x?}", &chunk[..size.min(4)]),
        };
        out.push(s);
        off += size;
    }
    out
}

// ── Audio ───────────────────────────────────────────────────────────────

pub struct AudioPlayer {
    pub path: PathBuf,
    child: Option<Child>,
}

impl AudioPlayer {
    pub fn new(path: PathBuf) -> Self {
        Self { path, child: None }
    }

    pub fn playing(&mut self) -> bool {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.child = None;
                    false
                }
                Ok(None) => true,
                Err(_) => {
                    self.child = None;
                    false
                }
            }
        } else {
            false
        }
    }

    pub fn toggle(&mut self) -> Result<String, String> {
        if self.playing() {
            self.stop();
            return Ok("Audio stopped".into());
        }
        self.play()
    }

    pub fn play(&mut self) -> Result<String, String> {
        self.stop();
        let path = self.path.display().to_string();
        // Prefer platform players; no extra crates.
        let child = if cfg!(target_os = "macos") {
            Command::new("afplay")
                .arg(&path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        } else if cfg!(target_os = "windows") {
            // powershell SoundPlayer is async-awkward; try ffplay/mpv
            Command::new("ffplay")
                .args(["-nodisp", "-autoexit", "-loglevel", "quiet", &path])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .or_else(|_| {
                    Command::new("mpv")
                        .args(["--no-video", "--really-quiet", &path])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                })
        } else {
            Command::new("ffplay")
                .args(["-nodisp", "-autoexit", "-loglevel", "quiet", &path])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .or_else(|_| {
                    Command::new("mpv")
                        .args(["--no-video", "--really-quiet", &path])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                })
                .or_else(|_| {
                    Command::new("aplay")
                        .arg(&path)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                })
        }
        .map_err(|e| format!("cannot play audio ({e}) — install afplay/ffplay/mpv"))?;
        self.child = Some(child);
        Ok(format!("Playing {}", self.path.display()))
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn audio_info_lines(path: &Path, playing: bool) -> Vec<PreviewLine> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("audio");
    let status = if playing {
        "▶ playing"
    } else {
        "■ stopped"
    };
    vec![
        pl(vec![("  Audio".into(), PreviewStyle::H2)]),
        pl(vec![(format!("  {name}"), PreviewStyle::Normal)]),
        pl(vec![(format!("  {status}"), PreviewStyle::Code)]),
        pl(vec![("".into(), PreviewStyle::Normal)]),
        pl(vec![("  Space  play / stop".into(), PreviewStyle::Dim)]),
        pl(vec![("  Esc    close preview".into(), PreviewStyle::Dim)]),
        pl(vec![(
            "  (uses afplay / ffplay / mpv)".into(),
            PreviewStyle::Dim,
        )]),
    ]
}

fn pl(spans: Vec<(String, PreviewStyle)>) -> PreviewLine {
    PreviewLine { spans, image: None }
}
