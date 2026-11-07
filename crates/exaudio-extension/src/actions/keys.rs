//! Keystroke chords in gpui keystroke syntax (`"secondary-z"`, `"alt-shift-up"`, `"space"`).
//!
//! `secondary-` is the platform modifier alias (cmd on macOS, ctrl elsewhere); it stays
//! symbolic until [`KeyChord::resolved`].

use std::fmt;

use serde_json::Value;

/// A normalized keystroke (`secondary` kept symbolic until [`KeyChord::resolved`]).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyChord {
    pub secondary: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub platform: bool,
    /// Lowercase gpui key name (`"a"`, `"space"`, `"left"`, `"f3"`, `"["`).
    pub key: String,
}

impl KeyChord {
    /// Parses a gpui keystroke string (`ctrl-`, `alt-`, `shift-`, `cmd-`/`super-`/`win-`,
    /// `secondary-` modifiers in any order). A trailing uppercase ASCII letter implies shift.
    pub fn parse(source: &str) -> Option<Self> {
        let source = source.trim();
        if source.is_empty() {
            return None;
        }
        let (modifier_part, key) = if source == "-" {
            ("", "-")
        } else if let Some(prefix) = source.strip_suffix("--") {
            (prefix, "-")
        } else {
            match source.rsplit_once('-') {
                Some((prefix, key)) => (prefix, key),
                None => ("", source),
            }
        };
        if key.is_empty() {
            return None;
        }
        let mut chord = KeyChord {
            secondary: false,
            ctrl: false,
            alt: false,
            shift: false,
            platform: false,
            key: String::new(),
        };
        for modifier in modifier_part.split('-').filter(|m| !m.is_empty()) {
            match modifier.to_ascii_lowercase().as_str() {
                "secondary" => chord.secondary = true,
                "ctrl" | "control" => chord.ctrl = true,
                "alt" => chord.alt = true,
                "shift" => chord.shift = true,
                "cmd" | "super" | "win" | "platform" => chord.platform = true,
                _ => return None,
            }
        }
        if key.len() == 1 && key.as_bytes()[0].is_ascii_uppercase() {
            chord.shift = true;
        }
        chord.key = key.to_ascii_lowercase();
        Some(chord)
    }

    /// Replaces `secondary` with the platform's concrete modifier.
    pub fn resolved(&self, mac: bool) -> KeyChord {
        let mut chord = self.clone();
        if chord.secondary {
            chord.secondary = false;
            if mac {
                chord.platform = true;
            } else {
                chord.ctrl = true;
            }
        }
        chord
    }

    /// Human label for the hotkeys dialog (`Ctrl+Shift+Z`, `Alt+←`).
    pub fn label(&self, mac: bool) -> String {
        let chord = self.resolved(mac);
        let mut parts: Vec<String> = Vec::new();
        if chord.ctrl {
            parts.push("Ctrl".into());
        }
        if chord.platform {
            parts.push(if mac { "Cmd" } else { "Win" }.into());
        }
        if chord.alt {
            parts.push("Alt".into());
        }
        if chord.shift {
            parts.push("Shift".into());
        }
        parts.push(key_label(&chord.key));
        parts.join("+")
    }
}

impl fmt::Display for KeyChord {
    /// Canonical gpui keystroke string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (enabled, name) in [
            (self.secondary, "secondary-"),
            (self.ctrl, "ctrl-"),
            (self.alt, "alt-"),
            (self.shift, "shift-"),
            (self.platform, "cmd-"),
        ] {
            if enabled {
                f.write_str(name)?;
            }
        }
        f.write_str(&self.key)
    }
}

fn key_label(key: &str) -> String {
    match key {
        "left" => "←".into(),
        "right" => "→".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "space" => "Space".into(),
        "tab" => "Tab".into(),
        "enter" => "Enter".into(),
        "escape" => "Escape".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        other if other.len() == 1 => other.to_ascii_uppercase(),
        other if other.starts_with('f') && other[1..].bytes().all(|b| b.is_ascii_digit()) => {
            other.to_ascii_uppercase()
        }
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// Chord from the legacy iced-era binding object `{"code":"KeyK","modifiers":["alt"]}`.
pub(crate) fn legacy_chord(value: &Value) -> Option<KeyChord> {
    let code = value.get("code")?.as_str()?;
    let key = if let Some(letter) = code.strip_prefix("Key") {
        letter.to_ascii_lowercase()
    } else if let Some(digit) = code.strip_prefix("Digit") {
        digit.to_string()
    } else if let Some(arrow) = code.strip_prefix("Arrow") {
        arrow.to_ascii_lowercase()
    } else if code.len() <= 3
        && code.starts_with('F')
        && code[1..].bytes().all(|b| b.is_ascii_digit())
    {
        code.to_ascii_lowercase()
    } else {
        match code {
            "Space" => "space",
            "Tab" => "tab",
            "Enter" => "enter",
            "Escape" => "escape",
            "Backspace" => "backspace",
            "Delete" => "delete",
            "Insert" => "insert",
            "Home" => "home",
            "End" => "end",
            "PageUp" => "pageup",
            "PageDown" => "pagedown",
            "Backquote" => "`",
            "Minus" => "-",
            "Equal" => "=",
            "BracketLeft" => "[",
            "BracketRight" => "]",
            "Backslash" => "\\",
            "Semicolon" => ";",
            "Quote" => "'",
            "Comma" => ",",
            "Period" => ".",
            "Slash" => "/",
            _ => return None,
        }
        .to_string()
    };
    let mut chord = KeyChord {
        secondary: false,
        ctrl: false,
        alt: false,
        shift: false,
        platform: false,
        key,
    };
    for name in value.get("modifiers")?.as_array()? {
        match name.as_str()? {
            "shift" => chord.shift = true,
            "control" => chord.ctrl = true,
            "alt" => chord.alt = true,
            "logo" => chord.platform = true,
            _ => return None,
        }
    }
    Some(chord)
}
