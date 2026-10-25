//! What the terminal sends to its program for keys, pasted text and mouse
//! reports: xterm's encodings, as `alacritty_terminal`'s modes ask for them.
//! Plain functions, so they are tested without a terminal.

use alacritty_terminal::term::TermMode;
use egui::{Key, Modifiers};

/// xterm's modifier parameter: 1, plus 1 for Shift, 2 for Alt, 4 for Ctrl.
fn modifier_param(mods: Modifiers) -> u8 {
    1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl)
}

fn has_modifiers(mods: Modifiers) -> bool {
    mods.shift || mods.alt || mods.ctrl
}

/// `ESC [ 1 ; m X` with modifiers, else `ESC [ X` (or `ESC O X` in
/// application cursor mode, for the cursor keys).
fn cursor_key(x: u8, mods: Modifiers, app_cursor: bool) -> Vec<u8> {
    if has_modifiers(mods) {
        format!("\x1b[1;{}{}", modifier_param(mods), x as char).into_bytes()
    } else if app_cursor {
        vec![0x1b, b'O', x]
    } else {
        vec![0x1b, b'[', x]
    }
}

/// `ESC [ n ~`, or `ESC [ n ; m ~` with modifiers.
fn tilde_key(n: u8, mods: Modifiers) -> Vec<u8> {
    if has_modifiers(mods) {
        format!("\x1b[{n};{}~", modifier_param(mods)).into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

/// F1–F4: `ESC O P`…`ESC O S`, `ESC [ 1 ; m P` with modifiers.
fn pf_key(x: u8, mods: Modifiers) -> Vec<u8> {
    if has_modifiers(mods) {
        format!("\x1b[1;{}{}", modifier_param(mods), x as char).into_bytes()
    } else {
        vec![0x1b, b'O', x]
    }
}

/// The control character Ctrl gives `key`, if any.
fn control_byte(key: Key) -> Option<u8> {
    let name = key.name();
    if name.len() == 1 {
        let c = name.as_bytes()[0];
        if c.is_ascii_uppercase() {
            return Some(c & 0x1f);
        }
    }
    Some(match key {
        Key::Space | Key::Num2 => 0x00,
        Key::OpenBracket => 0x1b,
        Key::Backslash => 0x1c,
        Key::CloseBracket => 0x1d,
        Key::Num6 => 0x1e,
        Key::Minus | Key::Slash => 0x1f,
        _ => return None,
    })
}

/// The bytes for a key press, or `None` when the key types text (that
/// arrives as a text event) or means nothing to a terminal.
pub fn key_bytes(key: Key, mods: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let esc = |mut bytes: Vec<u8>| {
        if mods.alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    let bytes = match key {
        // Shift+Enter is Alt+Enter, as Claude Code's /terminal-setup sets
        // it up elsewhere: a new line without sending.
        Key::Enter if mods.shift || mods.alt => b"\x1b\r".to_vec(),
        Key::Enter => b"\r".to_vec(),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => esc(b"\t".to_vec()),
        // Ctrl+Backspace deletes a word, as in Windows Terminal.
        Key::Backspace if mods.ctrl => esc(vec![0x08]),
        Key::Backspace => esc(vec![0x7f]),
        Key::Escape => esc(vec![0x1b]),
        Key::ArrowUp => cursor_key(b'A', mods, app_cursor),
        Key::ArrowDown => cursor_key(b'B', mods, app_cursor),
        Key::ArrowRight => cursor_key(b'C', mods, app_cursor),
        Key::ArrowLeft => cursor_key(b'D', mods, app_cursor),
        Key::Home => cursor_key(b'H', mods, app_cursor),
        Key::End => cursor_key(b'F', mods, app_cursor),
        Key::Insert => tilde_key(2, mods),
        Key::Delete => tilde_key(3, mods),
        Key::PageUp => tilde_key(5, mods),
        Key::PageDown => tilde_key(6, mods),
        Key::F1 => pf_key(b'P', mods),
        Key::F2 => pf_key(b'Q', mods),
        Key::F3 => pf_key(b'R', mods),
        Key::F4 => pf_key(b'S', mods),
        Key::F5 => tilde_key(15, mods),
        Key::F6 => tilde_key(17, mods),
        Key::F7 => tilde_key(18, mods),
        Key::F8 => tilde_key(19, mods),
        Key::F9 => tilde_key(20, mods),
        Key::F10 => tilde_key(21, mods),
        Key::F11 => tilde_key(23, mods),
        Key::F12 => tilde_key(24, mods),
        Key::F13 => tilde_key(25, mods),
        Key::F14 => tilde_key(26, mods),
        Key::F15 => tilde_key(28, mods),
        Key::F16 => tilde_key(29, mods),
        Key::F17 => tilde_key(31, mods),
        Key::F18 => tilde_key(32, mods),
        Key::F19 => tilde_key(33, mods),
        Key::F20 => tilde_key(34, mods),
        _ if mods.ctrl => esc(vec![control_byte(key)?]),
        // Everything else types text, which comes as a text event.
        _ => return None,
    };
    Some(bytes)
}

/// Typed text: UTF-8, with ESC before it while Alt is held (meta).
pub fn text_bytes(text: &str, alt: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 1);
    if alt {
        bytes.push(0x1b);
    }
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// Pasted text. Line breaks become CR, as a terminal's Enter; with
/// bracketed paste the text is wrapped so the program knows it was pasted,
/// and ESC is taken out so it cannot end the bracket early.
pub fn paste_bytes(text: &str, mode: TermMode) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend(text.bytes().filter(|&b| b != 0x1b));
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.into_bytes()
    }
}

/// A mouse button or wheel step, as a report numbers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

impl MouseButton {
    fn code(self) -> u8 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
        }
    }
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    /// Moved with the button down (`None`: no button, for motion tracking).
    Drag,
}

/// A mouse report at cell (`col`, `line`), counted from 0 on the screen, in
/// the encoding the mode asks for (SGR, else the original X10 encoding,
/// with UTF-8 coordinates when asked). `None` when the cell is beyond what
/// the original encoding can say.
pub fn mouse_report(
    button: Option<MouseButton>,
    action: MouseAction,
    mods: Modifiers,
    col: usize,
    line: usize,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let mut code = match button {
        Some(b) => b.code(),
        // Motion with no button down.
        None => 3,
    };
    if action == MouseAction::Drag {
        code += 32;
    }
    code += 4 * u8::from(mods.shift) + 8 * u8::from(mods.alt) + 16 * u8::from(mods.ctrl);
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if action == MouseAction::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{};{}{end}", col + 1, line + 1).into_bytes());
    }
    // The original encoding cannot say which button was released.
    if action == MouseAction::Release {
        code = (code & !3) | 3;
    }
    let utf8 = mode.contains(TermMode::UTF8_MOUSE);
    let mut bytes = vec![0x1b, b'[', b'M', 32 + code];
    for v in [col, line] {
        let v = v + 1 + 32;
        if utf8 {
            let c = char::from_u32(u32::try_from(v).ok()?).filter(|_| v < 2048)?;
            let mut buf = [0; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        } else {
            bytes.push(u8::try_from(v).ok()?);
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: TermMode = TermMode::empty();

    fn mods(shift: bool, alt: bool, ctrl: bool) -> Modifiers {
        Modifiers {
            shift,
            alt,
            ctrl,
            command: ctrl,
            ..Default::default()
        }
    }

    fn key(k: Key, m: Modifiers) -> Option<String> {
        key_bytes(k, m, NONE).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn cursor_keys_follow_the_application_mode() {
        let plain = Modifiers::NONE;
        assert_eq!(key(Key::ArrowUp, plain).unwrap(), "\x1b[A");
        let app = key_bytes(Key::ArrowUp, plain, TermMode::APP_CURSOR).unwrap();
        assert_eq!(app, b"\x1bOA");
        assert_eq!(key(Key::Home, plain).unwrap(), "\x1b[H");
        assert_eq!(key(Key::End, plain).unwrap(), "\x1b[F");
    }

    #[test]
    fn modifiers_are_numbered_as_xterm_does() {
        assert_eq!(
            key(Key::ArrowLeft, mods(false, false, true)).unwrap(),
            "\x1b[1;5D"
        );
        assert_eq!(
            key(Key::ArrowRight, mods(true, false, false)).unwrap(),
            "\x1b[1;2C"
        );
        assert_eq!(
            key(Key::ArrowUp, mods(true, true, true)).unwrap(),
            "\x1b[1;8A"
        );
        assert_eq!(
            key(Key::Delete, mods(false, false, true)).unwrap(),
            "\x1b[3;5~"
        );
        assert_eq!(
            key(Key::F5, mods(true, false, false)).unwrap(),
            "\x1b[15;2~"
        );
        assert_eq!(key(Key::F1, Modifiers::NONE).unwrap(), "\x1bOP");
        assert_eq!(key(Key::F1, mods(false, true, false)).unwrap(), "\x1b[1;3P");
    }

    #[test]
    fn ctrl_letters_are_control_characters() {
        let ctrl = mods(false, false, true);
        assert_eq!(key_bytes(Key::A, ctrl, NONE).unwrap(), [0x01]);
        assert_eq!(key_bytes(Key::C, ctrl, NONE).unwrap(), [0x03]);
        assert_eq!(key_bytes(Key::U, ctrl, NONE).unwrap(), [0x15]);
        assert_eq!(key_bytes(Key::W, ctrl, NONE).unwrap(), [0x17]);
        assert_eq!(key_bytes(Key::Z, ctrl, NONE).unwrap(), [0x1a]);
        assert_eq!(key_bytes(Key::Space, ctrl, NONE).unwrap(), [0x00]);
        assert_eq!(key_bytes(Key::OpenBracket, ctrl, NONE).unwrap(), [0x1b]);
        // Ctrl+Alt: ESC first.
        let ctrl_alt = mods(false, true, true);
        assert_eq!(key_bytes(Key::B, ctrl_alt, NONE).unwrap(), [0x1b, 0x02]);
    }

    #[test]
    fn text_keys_are_left_to_text_events() {
        assert_eq!(key(Key::A, Modifiers::NONE), None);
        assert_eq!(key(Key::A, mods(true, false, false)), None);
        assert_eq!(key(Key::A, mods(false, true, false)), None);
        assert_eq!(key(Key::Space, Modifiers::NONE), None);
        assert_eq!(text_bytes("é", false), "é".as_bytes());
        assert_eq!(text_bytes("x", true), b"\x1bx");
    }

    #[test]
    fn enter_backspace_tab_and_escape() {
        assert_eq!(key(Key::Enter, Modifiers::NONE).unwrap(), "\r");
        assert_eq!(key(Key::Enter, mods(true, false, false)).unwrap(), "\x1b\r");
        assert_eq!(key(Key::Backspace, Modifiers::NONE).unwrap(), "\x7f");
        assert_eq!(
            key(Key::Backspace, mods(false, false, true)).unwrap(),
            "\x08"
        );
        assert_eq!(
            key(Key::Backspace, mods(false, true, false)).unwrap(),
            "\x1b\x7f"
        );
        assert_eq!(key(Key::Tab, Modifiers::NONE).unwrap(), "\t");
        assert_eq!(key(Key::Tab, mods(true, false, false)).unwrap(), "\x1b[Z");
        assert_eq!(key(Key::Escape, Modifiers::NONE).unwrap(), "\x1b");
    }

    #[test]
    fn pastes_use_carriage_returns_and_brackets() {
        assert_eq!(paste_bytes("a\r\nb\nc", NONE), b"a\rb\rc");
        assert_eq!(
            paste_bytes("x\x1b[201~y\n", TermMode::BRACKETED_PASTE),
            b"\x1b[200~x[201~y\r\x1b[201~"
        );
    }

    #[test]
    fn mouse_reports_in_both_encodings() {
        let none = Modifiers::NONE;
        let sgr = TermMode::SGR_MOUSE;
        let r = |b, a, m, mode| mouse_report(b, a, m, 4, 9, mode).unwrap();
        assert_eq!(
            r(Some(MouseButton::Left), MouseAction::Press, none, sgr),
            b"\x1b[<0;5;10M"
        );
        assert_eq!(
            r(Some(MouseButton::Left), MouseAction::Release, none, sgr),
            b"\x1b[<0;5;10m"
        );
        assert_eq!(
            r(Some(MouseButton::Left), MouseAction::Drag, none, sgr),
            b"\x1b[<32;5;10M"
        );
        assert_eq!(
            r(
                Some(MouseButton::WheelUp),
                MouseAction::Press,
                mods(false, false, true),
                sgr
            ),
            b"\x1b[<80;5;10M"
        );
        assert_eq!(
            r(Some(MouseButton::Right), MouseAction::Press, none, NONE),
            [0x1b, b'[', b'M', 32 + 2, 32 + 5, 32 + 10]
        );
        assert_eq!(
            r(Some(MouseButton::Right), MouseAction::Release, none, NONE),
            [0x1b, b'[', b'M', 32 + 3, 32 + 5, 32 + 10]
        );
        // Beyond column 222 the original encoding has no byte.
        assert!(
            mouse_report(
                Some(MouseButton::Left),
                MouseAction::Press,
                none,
                300,
                0,
                NONE
            )
            .is_none()
        );
        let utf8 = mouse_report(
            Some(MouseButton::Left),
            MouseAction::Press,
            none,
            300,
            0,
            TermMode::UTF8_MOUSE,
        )
        .unwrap();
        assert_eq!(&utf8[4..], "\u{14d}!".as_bytes());
    }
}
