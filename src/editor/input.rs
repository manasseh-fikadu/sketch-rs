//! Platform-agnostic input events, so the editor can be driven by winit or by tests.

use crate::geom::V2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Cmd on macOS, the Windows/Super key elsewhere.
    pub logo: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
        logo: false,
    };
    pub const SHIFT: Modifiers = Modifiers {
        shift: true,
        ctrl: false,
        alt: false,
        logo: false,
    };
    /// The platform command modifier: Cmd on macOS, Ctrl elsewhere.
    pub fn command(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.logo
        } else {
            self.ctrl
        }
    }
    pub fn cmd() -> Modifiers {
        if cfg!(target_os = "macos") {
            Modifiers {
                logo: true,
                ..Modifiers::NONE
            }
        } else {
            Modifiers {
                ctrl: true,
                ..Modifiers::NONE
            }
        }
    }
    pub fn cmd_shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Modifiers::cmd()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Primary,
    Middle,
    Secondary,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Escape,
    Enter,
    Backspace,
    Delete,
    Tab,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// Screen position in logical pixels relative to the canvas viewport.
    PointerDown {
        pos: V2,
        button: Button,
        pressure: Option<f64>,
        time_ms: f64,
    },
    PointerMove {
        pos: V2,
        pressure: Option<f64>,
        time_ms: f64,
    },
    PointerUp {
        pos: V2,
        button: Button,
        time_ms: f64,
    },
    /// Scroll in screen pixels (positive y = content moves up).
    Scroll {
        delta: V2,
        pos: V2,
    },
    /// Multiplicative zoom about a point (trackpad pinch).
    Zoom {
        factor: f64,
        pos: V2,
    },
    KeyDown {
        key: Key,
        repeat: bool,
    },
    KeyUp {
        key: Key,
    },
    /// Committed text input (IME/char), separate from key presses.
    Text(String),
    Modifiers(Modifiers),
}
