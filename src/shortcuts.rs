use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use slint::platform::Key;
use slint::SharedString;

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize, PartialOrd, Ord)]
pub enum InputEvent {
    AlwaysOnTop,
    Fullscreen,
    OpenFile,
    NextImage,
    PreviousImage,
    ResetView,
    ZoomActualSize,
    Copy,
    Paste,
    SaveAs,
    CropSelection,
    Resize,
    ColorCorrections,
    Preferences,
    BrowseToFileLocation,
    ZenMode,
    PerfectFullscreen,
    ToggleThumbnails,
    SaveLayout1,
    SaveLayout2,
    SaveLayout3,
    LoadLayout1,
    LoadLayout2,
    LoadLayout3,
    Exit,
}

impl InputEvent {
    pub fn description(&self) -> &str {
        match self {
            InputEvent::OpenFile => "Open File",
            InputEvent::Fullscreen => "Toggle Fullscreen",
            InputEvent::AlwaysOnTop => "Toggle Always on Top",
            InputEvent::NextImage => "Next Image",
            InputEvent::PreviousImage => "Previous Image",
            InputEvent::ResetView => "Reset View",
            InputEvent::ZoomActualSize => "Zoom to Actual Size (1:1)",
            InputEvent::Copy => "Copy to Clipboard",
            InputEvent::Paste => "Paste from Clipboard",
            InputEvent::SaveAs => "Save as...",
            InputEvent::CropSelection => "Crop Image",
            InputEvent::Resize => "Resize Image",
            InputEvent::ColorCorrections => "Color Corrections",
            InputEvent::Preferences => "Preferences",
            InputEvent::BrowseToFileLocation => "Browse to File Location",
            InputEvent::ZenMode => "Toggle Zen Mode",
            InputEvent::PerfectFullscreen => "Fullscreen (Zen + Reset)",
            InputEvent::ToggleThumbnails => "Toggle Thumbnails",
            InputEvent::SaveLayout1 => "Save Layout 1",
            InputEvent::SaveLayout2 => "Save Layout 2",
            InputEvent::SaveLayout3 => "Save Layout 3",
            InputEvent::LoadLayout1 => "Load Layout 1",
            InputEvent::LoadLayout2 => "Load Layout 2",
            InputEvent::LoadLayout3 => "Load Layout 3",
            InputEvent::Exit => "Quit Application / Close Window",
        }
    }
}

pub type Shortcuts = BTreeMap<InputEvent, Vec<SimultaneousKeypresses>>;

#[derive(Debug, Default, PartialEq, Eq, Clone, Serialize, Deserialize, PartialOrd, Ord)]
pub struct SimultaneousKeypresses {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl SimultaneousKeypresses {
    pub fn format(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl { parts.push("Ctrl".to_string()); }
        if self.alt { parts.push("Alt".to_string()); }
        if self.shift { parts.push("Shift".to_string()); }
        parts.push(self.key.to_uppercase());
        parts.join(" + ")
    }

    pub fn new(key: &str) -> Self {
        Self {
            key: key.to_uppercase(),
            ..Default::default()
        }
    }

    pub fn from_key(key: Key) -> Self {
        Self {
            key: slint_to_human_readable(SharedString::from(key).as_str()).to_uppercase(),
            ..Default::default()
        }
    }

    pub fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub fn matches(&self, text: &str, ctrl: bool, alt: bool, shift: bool) -> bool {
        let stored_key = human_readable_to_slint(&self.key);
        
        let mut key_match = if text == stored_key {
            true
        } else if text.len() == 1 && stored_key.len() == 1 {
            text.to_lowercase() == stored_key.to_lowercase()
        } else {
            false
        };

        // Normalize Return key (\r vs \n)
        if !key_match && text.len() == 1 && (text == "\r" || text == "\n") {
            if stored_key == "\r" || stored_key == "\n" {
                key_match = true;
            }
        }

        key_match && self.ctrl == ctrl && self.alt == alt && self.shift == shift
    }
}

pub fn slint_to_human_readable(key: &str) -> String {
    let k_lower = key.to_lowercase();
    if k_lower == "\r" || k_lower == "\n" || key == SharedString::from(Key::Return).as_str() { return "Return".into(); }
    if key == "\u{001b}" || key == SharedString::from(Key::Escape).as_str() { return "Esc".into(); }
    if key == "\u{007f}" || key == SharedString::from(Key::Delete).as_str() { return "Delete".into(); }
    if key == "\u{0008}" || key == SharedString::from(Key::Backspace).as_str() { return "Backspace".into(); }
    if key == "\t" || key == SharedString::from(Key::Tab).as_str() { return "Tab".into(); }
    if key == " " || key == SharedString::from(Key::Space).as_str() { return "Space".into(); }
    if key == "\u{f702}" || key == "\u{f060}" || key == SharedString::from(Key::LeftArrow).as_str() { return "Left".into(); }
    if key == "\u{f703}" || key == "\u{f061}" || key == SharedString::from(Key::RightArrow).as_str() { return "Right".into(); }
    if key == "\u{f700}" || key == "\u{f062}" || key == SharedString::from(Key::UpArrow).as_str() { return "Up".into(); }
    if key == "\u{f701}" || key == "\u{f063}" || key == SharedString::from(Key::DownArrow).as_str() { return "Down".into(); }
    if key == SharedString::from(Key::F1).as_str() { return "F1".into(); }
    if key == SharedString::from(Key::F2).as_str() { return "F2".into(); }
    if key == SharedString::from(Key::F3).as_str() { return "F3".into(); }
    if key == SharedString::from(Key::F4).as_str() { return "F4".into(); }
    if key == SharedString::from(Key::F5).as_str() { return "F5".into(); }
    if key == SharedString::from(Key::F6).as_str() { return "F6".into(); }
    if key == SharedString::from(Key::F7).as_str() { return "F7".into(); }
    if key == SharedString::from(Key::F8).as_str() { return "F8".into(); }
    if key == SharedString::from(Key::F9).as_str() { return "F9".into(); }
    if key == SharedString::from(Key::F10).as_str() { return "F10".into(); }
    if key == SharedString::from(Key::F11).as_str() { return "F11".into(); }
    if key == SharedString::from(Key::F12).as_str() { return "F12".into(); }
    key.to_string()
}

pub fn human_readable_to_slint(name: &str) -> String {
    match name.to_uppercase().as_str() {
        "RETURN" | "ENTER" => "\r".to_string(),
        "ESC" | "ESCAPE" => "\u{001b}".to_string(),
        "DELETE" | "DEL" => "\u{007f}".to_string(),
        "BACKSPACE" => "\u{0008}".to_string(),
        "TAB" => "\t".to_string(),
        "SPACE" => " ".to_string(),
        "LEFT" => "\u{f702}".to_string(),
        "RIGHT" => "\u{f703}".to_string(),
        "UP" => "\u{f700}".to_string(),
        "DOWN" => "\u{f701}".to_string(),
        "F1" => SharedString::from(Key::F1).to_string(),
        "F2" => SharedString::from(Key::F2).to_string(),
        "F3" => SharedString::from(Key::F3).to_string(),
        "F4" => SharedString::from(Key::F4).to_string(),
        "F5" => SharedString::from(Key::F5).to_string(),
        "F6" => SharedString::from(Key::F6).to_string(),
        "F7" => SharedString::from(Key::F7).to_string(),
        "F8" => SharedString::from(Key::F8).to_string(),
        "F9" => SharedString::from(Key::F9).to_string(),
        "F10" => SharedString::from(Key::F10).to_string(),
        "F11" => SharedString::from(Key::F11).to_string(),
        "F12" => SharedString::from(Key::F12).to_string(),
        _ => name.to_string(),
    }
}

pub fn is_modifier(text: &str) -> bool {
    text == SharedString::from(Key::Control).as_str() ||
    text == SharedString::from(Key::Shift).as_str() ||
    text == SharedString::from(Key::Alt).as_str() ||
    text == SharedString::from(Key::Meta).as_str()
}

pub trait ShortcutExt {
    fn default_keys() -> Self;
}

impl ShortcutExt for Shortcuts {
    fn default_keys() -> Self {
        let mut s = Shortcuts::default();
        s.insert(InputEvent::OpenFile, vec![SimultaneousKeypresses::new("O").ctrl()]);
        s.insert(InputEvent::Fullscreen, vec![SimultaneousKeypresses::new("F")]);
        s.insert(InputEvent::AlwaysOnTop, vec![SimultaneousKeypresses::new("T").ctrl()]);
        s.insert(InputEvent::ToggleThumbnails, vec![SimultaneousKeypresses::new("T")]);
        s.insert(InputEvent::CropSelection, vec![SimultaneousKeypresses::new("Y").ctrl()]);
        s.insert(InputEvent::Exit, vec![
            SimultaneousKeypresses::new("Esc"),
            SimultaneousKeypresses::new("Q")
        ]);
        s.insert(InputEvent::PreviousImage, vec![
            SimultaneousKeypresses::new("A"),
            SimultaneousKeypresses::new("Left")
        ]);
        s.insert(InputEvent::NextImage, vec![
            SimultaneousKeypresses::new("D"),
            SimultaneousKeypresses::new("Right")
        ]);
        s.insert(InputEvent::ZoomActualSize, vec![SimultaneousKeypresses::new("1")]);
        s.insert(InputEvent::ResetView, vec![SimultaneousKeypresses::new("V")]);
        s.insert(InputEvent::Copy, vec![SimultaneousKeypresses::new("C").ctrl()]);
        s.insert(InputEvent::Paste, vec![SimultaneousKeypresses::new("V").ctrl()]);
        s.insert(InputEvent::SaveAs, vec![SimultaneousKeypresses::new("S").ctrl().shift()]);
        s.insert(InputEvent::Resize, vec![SimultaneousKeypresses::new("R")]);
        s.insert(InputEvent::ColorCorrections, vec![SimultaneousKeypresses::new("C")]);
        s.insert(InputEvent::Preferences, vec![SimultaneousKeypresses::new("P")]);
        s.insert(InputEvent::BrowseToFileLocation, vec![SimultaneousKeypresses::new("B").ctrl()]);
        s.insert(InputEvent::ZenMode, vec![SimultaneousKeypresses::new("Z")]);
        s.insert(InputEvent::PerfectFullscreen, vec![SimultaneousKeypresses::new("Return")]);
        s.insert(InputEvent::SaveLayout1, vec![SimultaneousKeypresses::new("F1").shift()]);
        s.insert(InputEvent::SaveLayout2, vec![SimultaneousKeypresses::new("F2").shift()]);
        s.insert(InputEvent::SaveLayout3, vec![SimultaneousKeypresses::new("F3").shift()]);
        s.insert(InputEvent::LoadLayout1, vec![SimultaneousKeypresses::new("F1")]);
        s.insert(InputEvent::LoadLayout2, vec![SimultaneousKeypresses::new("F2")]);
        s.insert(InputEvent::LoadLayout3, vec![SimultaneousKeypresses::new("F3")]);
        s
    }
}

pub fn lookup(
    shortcuts: &Shortcuts,
    text: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> Option<InputEvent> {
    for (input_event, key_list) in shortcuts {
        for keys in key_list {
            if keys.matches(text, ctrl, alt, shift) {
                return Some(input_event.clone());
            }
        }
    }
    None
}
