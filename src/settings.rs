//! Persistent application settings.
//!
//! The file intentionally uses a small, line-oriented format. These settings
//! are owned by nvim-gpui rather than Neovim, so they live in the platform
//! application-support directory and do not change XDG or Neovim's runtime
//! directories.

use std::{env, fs, path::PathBuf};

const SETTINGS_FILE_ENV: &str = "NVIM_GPUI_SETTINGS_FILE";
pub const DEFAULT_IMAGE_CACHE_SIZE_MB: u32 = 128;
pub const IMAGE_CACHE_SIZE_OPTIONS_MB: &[u32] = &[64, 128, 256, 512, 1024];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PasteShortcut {
    #[default]
    CmdV,
    CtrlV,
    Custom(String),
    Disabled,
}

impl PasteShortcut {
    pub fn key(&self) -> &str {
        match self {
            Self::CmdV => "cmd-v",
            Self::CtrlV => "ctrl-v",
            Self::Custom(key) => key,
            Self::Disabled => "disabled",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::CmdV => "Cmd-V".to_owned(),
            Self::CtrlV => "Ctrl-V".to_owned(),
            Self::Custom(key) => gpui::Keystroke::parse(key)
                .map(|keystroke| keystroke.to_string())
                .unwrap_or_else(|_| key.clone()),
            Self::Disabled => "Disabled".to_owned(),
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "cmd-v" => Some(Self::CmdV),
            "ctrl-v" => Some(Self::CtrlV),
            "disabled" => Some(Self::Disabled),
            _ => {
                let keystroke = gpui::Keystroke::parse(value).ok()?;
                keystroke
                    .modifiers
                    .modified()
                    .then(|| Self::Custom(value.to_owned()))
            }
        }
    }

    pub fn from_keystroke(keystroke: &gpui::Keystroke) -> Option<Self> {
        if !keystroke.modifiers.modified() || keystroke.key.is_empty() {
            return None;
        }

        let key = keystroke.unparse();
        Some(match key.as_str() {
            "cmd-v" => Self::CmdV,
            "ctrl-v" => Self::CtrlV,
            _ => Self::Custom(key),
        })
    }

    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }

    fn parsed_keystroke(&self) -> Option<gpui::Keystroke> {
        match self {
            Self::Disabled => None,
            _ => gpui::Keystroke::parse(self.key()).ok(),
        }
    }

    pub fn matches(&self, keystroke: &gpui::Keystroke) -> bool {
        let Some(expected) = self.parsed_keystroke() else {
            return false;
        };
        keystroke.should_match(&gpui::KeybindingKeystroke::from_keystroke(expected))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NerdFontChoice {
    #[default]
    Symbols,
    SymbolsMono,
}

impl NerdFontChoice {
    pub fn key(self) -> &'static str {
        match self {
            Self::Symbols => "symbols",
            Self::SymbolsMono => "symbols-mono",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Symbols => "Symbols Nerd Font",
            Self::SymbolsMono => "Symbols Nerd Font Mono",
        }
    }

    pub fn family(self) -> &'static str {
        match self {
            Self::Symbols => crate::platform::SYMBOLS_NERD_FONT_FAMILY,
            Self::SymbolsMono => crate::platform::SYMBOLS_NERD_FONT_MONO_FAMILY,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "symbols" => Some(Self::Symbols),
            "symbols-mono" => Some(Self::SymbolsMono),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FallbackMode {
    None,
    #[default]
    Auto,
    Force,
}

impl FallbackMode {
    pub fn key(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Auto => "auto",
            Self::Force => "force",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Auto => "Auto",
            Self::Force => "Force",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "auto" => Some(Self::Auto),
            "force" => Some(Self::Force),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub nerd_font: NerdFontChoice,
    pub fallback_mode: FallbackMode,
    pub startup_maximized: bool,
    pub image_cache_size_mb: u32,
    pub paste_shortcut: PasteShortcut,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            nerd_font: NerdFontChoice::default(),
            fallback_mode: FallbackMode::default(),
            startup_maximized: false,
            image_cache_size_mb: DEFAULT_IMAGE_CACHE_SIZE_MB,
            paste_shortcut: PasteShortcut::default(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        let Ok(contents) = fs::read_to_string(path) else {
            return Self::default();
        };
        parse_settings(&contents)
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = settings_path() else {
            return Err("could not determine the settings directory".to_owned());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create settings directory: {error}"))?;
        }
        fs::write(&path, self.to_file_contents())
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }

    fn to_file_contents(&self) -> String {
        format!(
            "nerd_font={}\nfallback_mode={}\nstartup_maximized={}\nimage_cache_size_mb={}\npaste_shortcut={}\n",
            self.nerd_font.key(),
            self.fallback_mode.key(),
            self.startup_maximized,
            self.image_cache_size_mb,
            self.paste_shortcut.key()
        )
    }
}

fn parse_settings(contents: &str) -> Settings {
    let mut settings = Settings::default();
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "nerd_font" => {
                if let Some(value) = NerdFontChoice::parse(value.trim()) {
                    settings.nerd_font = value;
                }
            }
            "fallback_mode" => {
                if let Some(value) = FallbackMode::parse(value.trim()) {
                    settings.fallback_mode = value;
                }
            }
            "startup_maximized" => {
                if let Ok(value) = value.trim().parse() {
                    settings.startup_maximized = value;
                }
            }
            "image_cache_size_mb" => {
                if let Ok(value) = value.trim().parse::<u32>() {
                    if (16..=4096).contains(&value) {
                        settings.image_cache_size_mb = value;
                    }
                }
            }
            "paste_shortcut" => {
                if let Some(value) = PasteShortcut::parse(value.trim()) {
                    settings.paste_shortcut = value;
                }
            }
            _ => {}
        }
    }
    settings
}

fn settings_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os(SETTINGS_FILE_ENV) {
        return Some(PathBuf::from(path));
    }

    application_support_directory().map(|directory| directory.join("settings.conf"))
}

pub(crate) fn application_support_directory() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/nvim-gpui"))
    }

    #[cfg(target_os = "windows")]
    {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|app_data| app_data.join("nvim-gpui"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|config| config.join("nvim-gpui"))
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_settings, FallbackMode, NerdFontChoice, PasteShortcut, Settings};

    #[test]
    fn defaults_are_conservative_and_use_a_128_mb_image_cache() {
        assert_eq!(Settings::default().image_cache_size_mb, 128);
        assert_eq!(Settings::default().nerd_font, NerdFontChoice::Symbols);
        assert_eq!(Settings::default().fallback_mode, FallbackMode::Auto);
        assert!(!Settings::default().startup_maximized);
        assert_eq!(Settings::default().paste_shortcut, PasteShortcut::CmdV);
        assert_eq!(NerdFontChoice::Symbols.family(), "Symbols Nerd Font");
        assert_eq!(
            NerdFontChoice::SymbolsMono.family(),
            "Symbols Nerd Font Mono"
        );
    }

    #[test]
    fn settings_parser_ignores_unknown_and_invalid_values() {
        let settings = parse_settings(
            "nerd_font=symbols-mono\nfallback_mode=force\nstartup_maximized=true\nimage_cache_size_mb=512\npaste_shortcut=ctrl-v\nunknown=x\nimage_cache_size_mb=1\n",
        );

        assert_eq!(settings.nerd_font, NerdFontChoice::SymbolsMono);
        assert_eq!(settings.fallback_mode, FallbackMode::Force);
        assert!(settings.startup_maximized);
        assert_eq!(settings.image_cache_size_mb, 512);
        assert_eq!(settings.paste_shortcut, PasteShortcut::CtrlV);
    }

    #[test]
    fn paste_shortcut_matches_only_its_configured_keystroke() {
        let cmd_v = gpui::Keystroke::parse("cmd-v").expect("cmd-v should parse");
        let ctrl_v = gpui::Keystroke::parse("ctrl-v").expect("ctrl-v should parse");

        assert!(PasteShortcut::CmdV.matches(&cmd_v));
        assert!(!PasteShortcut::CmdV.matches(&ctrl_v));
        assert!(PasteShortcut::CtrlV.matches(&ctrl_v));
        assert!(!PasteShortcut::Disabled.matches(&cmd_v));
    }

    #[test]
    fn paste_shortcut_accepts_custom_modified_keystrokes() {
        let keystroke = gpui::Keystroke::parse("cmd-shift-v").expect("shortcut should parse");
        let shortcut = PasteShortcut::from_keystroke(&keystroke).expect("shortcut is modified");

        assert_eq!(shortcut, PasteShortcut::Custom("cmd-shift-v".to_owned()));
        assert!(shortcut.matches(&keystroke));
        assert_eq!(shortcut.key(), "cmd-shift-v");
    }

    #[test]
    fn paste_shortcut_rejects_unmodified_key_recordings() {
        let key = gpui::Keystroke::parse("v").expect("key should parse");

        assert!(PasteShortcut::from_keystroke(&key).is_none());
        assert_eq!(
            parse_settings("paste_shortcut=v\n").paste_shortcut,
            PasteShortcut::CmdV
        );
    }
}
