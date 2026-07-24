//! Map C/Swift key codes → `suisei_core::KeyEvent`.

use suisei_core::key::{KeyCode, KeyEvent, KeyModifiers};

/// Stable C ABI codes (must match `include/suisei_engine.h` and Swift).
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum FfiKeyCode {
    Char = 1,
    Enter = 2,
    Esc = 3,
    Backspace = 4,
    Tab = 5,
    BackTab = 6,
    Delete = 7,
    Left = 8,
    Right = 9,
    Up = 10,
    Down = 11,
    Home = 12,
    End = 13,
    PageUp = 14,
    PageDown = 15,
    F = 16,
}

pub fn key_from_ffi(
    code: u32,
    ch: u32,
    f_num: u8,
    mods: u8,
) -> Option<KeyEvent> {
    let modifiers = map_mods(mods);
    let code = match code {
        x if x == FfiKeyCode::Char as u32 => {
            let c = char::from_u32(ch)?;
            KeyCode::Char(c)
        }
        x if x == FfiKeyCode::Enter as u32 => KeyCode::Enter,
        x if x == FfiKeyCode::Esc as u32 => KeyCode::Esc,
        x if x == FfiKeyCode::Backspace as u32 => KeyCode::Backspace,
        x if x == FfiKeyCode::Tab as u32 => KeyCode::Tab,
        x if x == FfiKeyCode::BackTab as u32 => KeyCode::BackTab,
        x if x == FfiKeyCode::Delete as u32 => KeyCode::Delete,
        x if x == FfiKeyCode::Left as u32 => KeyCode::Left,
        x if x == FfiKeyCode::Right as u32 => KeyCode::Right,
        x if x == FfiKeyCode::Up as u32 => KeyCode::Up,
        x if x == FfiKeyCode::Down as u32 => KeyCode::Down,
        x if x == FfiKeyCode::Home as u32 => KeyCode::Home,
        x if x == FfiKeyCode::End as u32 => KeyCode::End,
        x if x == FfiKeyCode::PageUp as u32 => KeyCode::PageUp,
        x if x == FfiKeyCode::PageDown as u32 => KeyCode::PageDown,
        x if x == FfiKeyCode::F as u32 => KeyCode::F(f_num),
        _ => return None,
    };
    Some(KeyEvent::new(code, modifiers))
}

fn map_mods(bits: u8) -> KeyModifiers {
    // Match header: SHIFT=1 CONTROL=2 ALT=4 SUPER=8
    let mut m = KeyModifiers::NONE;
    if bits & 1 != 0 {
        m |= KeyModifiers::SHIFT;
    }
    if bits & 2 != 0 {
        m |= KeyModifiers::CONTROL;
    }
    if bits & 4 != 0 {
        m |= KeyModifiers::ALT;
    }
    if bits & 8 != 0 {
        m |= KeyModifiers::SUPER;
    }
    m
}
