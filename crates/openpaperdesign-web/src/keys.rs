//! DOM keyboard and modifier state to upstream editor input.
//!
//! Upstream decides the command modifier at compile time (`Modifiers::command()`: Cmd when
//! built for macOS, Ctrl otherwise). A wasm32 build is "otherwise" even in a Mac browser, so
//! on a Mac (detected from `navigator` by the page) Meta (⌘) is reported to the editor as
//! `ctrl` and Control as `logo`. Shortcuts then use ⌘ on a Mac and Ctrl elsewhere, like the
//! native app. Upstream code is not changed.

use crate::editor::{Key, Modifiers};

pub fn modifiers(mac: bool, shift: bool, ctrl: bool, alt: bool, meta: bool) -> Modifiers {
    let (command, secondary) = if mac { (meta, ctrl) } else { (ctrl, meta) };
    // The browser adapter is also exercised by native-host tests. Normalize for the
    // Editor's compiled host here, rather than assuming every caller is already wasm.
    // On wasm (target_os != macos) this is byte-for-byte the previous mapping.
    let (ctrl, logo) = if cfg!(target_os = "macos") {
        (secondary, command)
    } else {
        (command, secondary)
    };
    Modifiers {
        shift,
        ctrl,
        alt,
        logo,
    }
}

/// `KeyboardEvent.key` (the layout-dependent character or a named key).
pub fn logical(key: &str) -> Option<Key> {
    Some(match key {
        "Escape" => Key::Escape,
        "Enter" => Key::Enter,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Tab" => Key::Tab,
        " " => Key::Space,
        "ArrowLeft" => Key::Left,
        "ArrowRight" => Key::Right,
        "ArrowUp" => Key::Up,
        "ArrowDown" => Key::Down,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        s => {
            let mut chars = s.chars();
            let c = chars.next()?;
            // Named keys ("Shift", "F1", ...) are longer than one character.
            if chars.next().is_some() {
                return None;
            }
            Key::Char(c)
        }
    })
}

/// `KeyboardEvent.code` (the physical key), for chords: the same choice the native app makes
/// with winit's physical key codes.
pub fn physical(code: &str) -> Option<Key> {
    if let Some(c) = code.strip_prefix("Key")
        && c.len() == 1
    {
        return c.chars().next().map(|c| Key::Char(c.to_ascii_lowercase()));
    }
    if let Some(d) = code.strip_prefix("Digit")
        && d.len() == 1
    {
        return d.chars().next().map(Key::Char);
    }
    Some(Key::Char(match code {
        "BracketLeft" => '[',
        "BracketRight" => ']',
        "Equal" => '=',
        "Minus" => '-',
        "Quote" => '\'',
        "Slash" => '/',
        "Backslash" => '\\',
        "Comma" => ',',
        "Period" => '.',
        "Semicolon" => ';',
        "Backquote" => '`',
        _ => return logical(code_named(code)?),
    }))
}

fn code_named(code: &str) -> Option<&str> {
    Some(match code {
        "Escape" | "Enter" | "Backspace" | "Delete" | "Tab" | "ArrowLeft" | "ArrowRight"
        | "ArrowUp" | "ArrowDown" | "Home" | "End" | "PageUp" | "PageDown" => code,
        "Space" => " ",
        _ => return None,
    })
}

/// Pick the key like the native app: chords (Alt or the command modifier) resolve by
/// physical key so ⌥A or a non-Latin layout still hits the shortcut; plain keys use the
/// logical character.
pub fn key(mods: Modifiers, text_editing: bool, key: &str, code: &str) -> Option<Key> {
    let chord = mods.alt || mods.command();
    if chord && !text_editing {
        physical(code).or_else(|| logical(key))
    } else {
        logical(key).or_else(|| physical(code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_meta_is_the_command_modifier() {
        let m = modifiers(true, false, false, false, true);
        assert!(m.command());
        let m = modifiers(true, false, true, false, false);
        assert!(!m.command());
    }

    #[test]
    fn other_platforms_use_ctrl() {
        assert!(modifiers(false, false, true, false, false).command());
        assert!(!modifiers(false, false, false, false, true).command());
    }

    #[test]
    fn chords_use_the_physical_key() {
        let cmd = modifiers(false, false, true, false, false);
        assert_eq!(key(cmd, false, "å", "KeyA"), Some(Key::Char('a')));
        assert_eq!(
            key(Modifiers::NONE, false, "R", "KeyR"),
            Some(Key::Char('R'))
        );
        assert_eq!(
            key(Modifiers::NONE, false, "Delete", "Delete"),
            Some(Key::Delete)
        );
        assert_eq!(key(cmd, false, "z", "KeyZ"), Some(Key::Char('z')));
        assert_eq!(key(Modifiers::NONE, false, "Shift", "ShiftLeft"), None);
        assert_eq!(key(Modifiers::NONE, false, " ", "Space"), Some(Key::Space));
    }
}
