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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogLevel {
    #[default]
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Error => "Error",
            Self::Warn => "Warn",
            Self::Info => "Info",
            Self::Debug => "Debug",
            Self::Trace => "Trace",
        }
    }

    pub fn filter(self) -> log::LevelFilter {
        match self {
            Self::Off => log::LevelFilter::Off,
            Self::Error => log::LevelFilter::Error,
            Self::Warn => log::LevelFilter::Warn,
            Self::Info => log::LevelFilter::Info,
            Self::Debug => log::LevelFilter::Debug,
            Self::Trace => log::LevelFilter::Trace,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "error" => Some(Self::Error),
            "warn" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }
}

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
pub enum ImeBackend {
    #[default]
    System,
    Rime,
}

impl ImeBackend {
    pub fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Rime => "rime",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System IME",
            Self::Rime => "Rime",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "rime" => Some(Self::Rime),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RimeCandidateLayout {
    #[default]
    Vertical,
    Horizontal,
}

impl RimeCandidateLayout {
    pub fn key(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "vertical" => Some(Self::Vertical),
            "horizontal" => Some(Self::Horizontal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RimeToggleShortcut {
    #[default]
    PlatformDefault,
    CmdBackslash,
    CtrlBackslash,
    CtrlSpace,
    Custom(String),
    Disabled,
}

impl RimeToggleShortcut {
    pub fn key(&self) -> &str {
        match self {
            Self::PlatformDefault => {
                #[cfg(target_os = "macos")]
                {
                    "cmd-\\"
                }
                #[cfg(target_os = "windows")]
                {
                    "ctrl-\\"
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    "ctrl-space"
                }
            }
            Self::CmdBackslash => "cmd-\\",
            Self::CtrlBackslash => "ctrl-\\",
            Self::CtrlSpace => "ctrl-space",
            Self::Custom(key) => key,
            Self::Disabled => "disabled",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::PlatformDefault => gpui::Keystroke::parse(self.key())
                .map(|keystroke| keystroke.to_string())
                .unwrap_or_else(|_| self.key().to_owned()),
            Self::CmdBackslash => "Cmd-\\".to_owned(),
            Self::CtrlBackslash => "Ctrl-\\".to_owned(),
            Self::CtrlSpace => "Ctrl-Space".to_owned(),
            Self::Custom(key) => gpui::Keystroke::parse(key)
                .map(|keystroke| keystroke.to_string())
                .unwrap_or_else(|_| key.clone()),
            Self::Disabled => "Disabled".to_owned(),
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "cmd-\\" => Some(Self::CmdBackslash),
            "ctrl-\\" => Some(Self::CtrlBackslash),
            "ctrl-space" => Some(Self::CtrlSpace),
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
            "cmd-\\" => Self::CmdBackslash,
            "ctrl-\\" => Self::CtrlBackslash,
            "ctrl-space" => Self::CtrlSpace,
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
    pub quit_on_window_close: bool,
    pub allow_multiple_instances: bool,
    pub log_level: LogLevel,
    pub image_cache_size_mb: u32,
    pub paste_shortcut: PasteShortcut,
    pub ime_backend: ImeBackend,
    pub rime_candidate_layout: RimeCandidateLayout,
    pub rime_toggle_shortcut: RimeToggleShortcut,
    pub rime_library_dir: String,
    pub rime_library_auto_detect: bool,
    pub rime_data_dir: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            nerd_font: NerdFontChoice::default(),
            fallback_mode: FallbackMode::default(),
            startup_maximized: false,
            quit_on_window_close: true,
            allow_multiple_instances: true,
            log_level: LogLevel::default(),
            image_cache_size_mb: DEFAULT_IMAGE_CACHE_SIZE_MB,
            paste_shortcut: PasteShortcut::default(),
            ime_backend: ImeBackend::default(),
            rime_candidate_layout: RimeCandidateLayout::default(),
            rime_toggle_shortcut: RimeToggleShortcut::default(),
            rime_library_dir: String::new(),
            rime_library_auto_detect: false,
            rime_data_dir: String::new(),
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
            "nerd_font={}\nfallback_mode={}\nstartup_maximized={}\nquit_on_window_close={}\nallow_multiple_instances={}\nlog_level={}\nimage_cache_size_mb={}\npaste_shortcut={}\nime_backend={}\nrime_candidate_layout={}\nrime_toggle_shortcut={}\nrime_library_dir={}\nrime_library_auto_detect={}\nrime_data_dir={}\n",
            self.nerd_font.key(),
            self.fallback_mode.key(),
            self.startup_maximized,
            self.quit_on_window_close,
            self.allow_multiple_instances,
            self.log_level.key(),
            self.image_cache_size_mb,
            self.paste_shortcut.key(),
            self.ime_backend.key(),
            self.rime_candidate_layout.key(),
            self.rime_toggle_shortcut.key(),
            self.rime_library_dir,
            self.rime_library_auto_detect,
            self.rime_data_dir
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
            "quit_on_window_close" => {
                if let Ok(value) = value.trim().parse() {
                    settings.quit_on_window_close = value;
                }
            }
            "allow_multiple_instances" => {
                if let Ok(value) = value.trim().parse() {
                    settings.allow_multiple_instances = value;
                }
            }
            "log_level" => {
                if let Some(value) = LogLevel::parse(value.trim()) {
                    settings.log_level = value;
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
            "ime_backend" => {
                if let Some(value) = ImeBackend::parse(value.trim()) {
                    settings.ime_backend = value;
                }
            }
            "rime_candidate_layout" => {
                if let Some(value) = RimeCandidateLayout::parse(value.trim()) {
                    settings.rime_candidate_layout = value;
                }
            }
            "rime_toggle_shortcut" => {
                if let Some(value) = RimeToggleShortcut::parse(value.trim()) {
                    settings.rime_toggle_shortcut = value;
                }
            }
            "rime_library_dir" => settings.rime_library_dir = value.trim().to_owned(),
            "rime_library_auto_detect" => {
                if let Ok(value) = value.trim().parse() {
                    settings.rime_library_auto_detect = value;
                }
            }
            "rime_data_dir" => settings.rime_data_dir = value.trim().to_owned(),
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

pub(crate) fn rime_user_data_directory() -> Option<PathBuf> {
    application_support_directory().map(|directory| directory.join("rime"))
}

#[cfg(test)]
mod tests {
    use super::{
        parse_settings, FallbackMode, ImeBackend, LogLevel, NerdFontChoice, PasteShortcut,
        RimeCandidateLayout, RimeToggleShortcut, Settings,
    };

    #[test]
    fn defaults_are_conservative_and_use_a_128_mb_image_cache() {
        assert_eq!(Settings::default().image_cache_size_mb, 128);
        assert_eq!(Settings::default().nerd_font, NerdFontChoice::Symbols);
        assert_eq!(Settings::default().fallback_mode, FallbackMode::Auto);
        assert!(!Settings::default().startup_maximized);
        assert!(Settings::default().quit_on_window_close);
        assert!(Settings::default().allow_multiple_instances);
        assert_eq!(Settings::default().log_level, LogLevel::Off);
        assert_eq!(Settings::default().paste_shortcut, PasteShortcut::CmdV);
        assert_eq!(Settings::default().ime_backend, ImeBackend::System);
        assert!(!Settings::default().rime_library_auto_detect);
        assert_eq!(
            Settings::default().rime_candidate_layout,
            RimeCandidateLayout::Vertical
        );
        assert_eq!(
            Settings::default().rime_toggle_shortcut,
            RimeToggleShortcut::PlatformDefault
        );
        #[cfg(target_os = "macos")]
        assert_eq!(Settings::default().rime_toggle_shortcut.key(), "cmd-\\");
        #[cfg(target_os = "windows")]
        assert_eq!(Settings::default().rime_toggle_shortcut.key(), "ctrl-\\");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(Settings::default().rime_toggle_shortcut.key(), "ctrl-space");
        assert_eq!(NerdFontChoice::Symbols.family(), "Symbols Nerd Font");
        assert_eq!(
            NerdFontChoice::SymbolsMono.family(),
            "Symbols Nerd Font Mono"
        );
    }

    #[test]
    fn settings_parser_ignores_unknown_and_invalid_values() {
        let settings = parse_settings(
            "nerd_font=symbols-mono\nfallback_mode=force\nstartup_maximized=true\nquit_on_window_close=false\nallow_multiple_instances=false\nlog_level=debug\nimage_cache_size_mb=512\npaste_shortcut=ctrl-v\nime_backend=system\nrime_candidate_layout=horizontal\nrime_toggle_shortcut=ctrl-shift-space\nrime_library_dir=/tmp/librime\nrime_library_auto_detect=true\nrime_data_dir=/tmp/rime-data\nrime_user_data_dir=/tmp/rime-user\nrime_staging_data_dir=/tmp/rime-staging\nunknown=x\nimage_cache_size_mb=1\n",
        );

        assert_eq!(settings.nerd_font, NerdFontChoice::SymbolsMono);
        assert_eq!(settings.fallback_mode, FallbackMode::Force);
        assert!(settings.startup_maximized);
        assert!(!settings.quit_on_window_close);
        assert!(!settings.allow_multiple_instances);
        assert_eq!(settings.log_level, LogLevel::Debug);
        assert_eq!(settings.image_cache_size_mb, 512);
        assert_eq!(settings.paste_shortcut, PasteShortcut::CtrlV);
        assert_eq!(settings.ime_backend, ImeBackend::System);
        assert!(settings.rime_library_auto_detect);
        assert_eq!(
            settings.rime_candidate_layout,
            RimeCandidateLayout::Horizontal
        );
        assert_eq!(
            settings.rime_toggle_shortcut,
            RimeToggleShortcut::Custom("ctrl-shift-space".to_owned())
        );
    }

    #[test]
    fn log_levels_map_to_filters_and_ignore_invalid_values() {
        assert_eq!(LogLevel::Off.filter(), log::LevelFilter::Off);
        assert_eq!(LogLevel::Error.filter(), log::LevelFilter::Error);
        assert_eq!(LogLevel::Warn.filter(), log::LevelFilter::Warn);
        assert_eq!(LogLevel::Info.filter(), log::LevelFilter::Info);
        assert_eq!(LogLevel::Debug.filter(), log::LevelFilter::Debug);
        assert_eq!(LogLevel::Trace.filter(), log::LevelFilter::Trace);
        assert_eq!(
            parse_settings("log_level=invalid\n").log_level,
            LogLevel::Off
        );
    }

    #[test]
    fn new_runtime_settings_are_written_to_the_persistent_file() {
        let settings = Settings {
            quit_on_window_close: false,
            allow_multiple_instances: false,
            log_level: LogLevel::Trace,
            ..Settings::default()
        };

        let contents = settings.to_file_contents();
        assert!(contents.contains("quit_on_window_close=false\n"));
        assert!(contents.contains("allow_multiple_instances=false\n"));
        assert!(contents.contains("log_level=trace\n"));
        assert!(contents.contains(&format!(
            "rime_toggle_shortcut={}\n",
            RimeToggleShortcut::default().key()
        )));
        assert!(contents.contains("ime_backend=system\n"));
        assert!(contents.contains("rime_candidate_layout=vertical\n"));
        assert!(contents.contains("rime_library_auto_detect=false\n"));
        assert!(!contents.contains("rime_user_data_dir="));
        assert!(!contents.contains("rime_staging_data_dir="));
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

    #[test]
    fn rime_toggle_shortcut_matches_and_rejects_unmodified_keys() {
        let platform_default = RimeToggleShortcut::default();
        let platform_default_key =
            gpui::Keystroke::parse(platform_default.key()).expect("default shortcut should parse");
        let ctrl_space = gpui::Keystroke::parse("ctrl-space").expect("ctrl-space should parse");
        let ctrl_shift_space =
            gpui::Keystroke::parse("ctrl-shift-space").expect("shortcut should parse");
        let space = gpui::Keystroke::parse("space").expect("space should parse");

        assert!(platform_default.matches(&platform_default_key));
        assert!(RimeToggleShortcut::CtrlSpace.matches(&ctrl_space));
        assert!(!RimeToggleShortcut::CtrlSpace.matches(&ctrl_shift_space));
        assert!(!RimeToggleShortcut::CtrlSpace.matches(&space));
        assert_eq!(
            RimeToggleShortcut::from_keystroke(&ctrl_shift_space),
            Some(RimeToggleShortcut::Custom("ctrl-shift-space".to_owned()))
        );
        assert!(RimeToggleShortcut::from_keystroke(&space).is_none());
        assert!(!RimeToggleShortcut::Disabled.matches(&ctrl_space));
    }

    #[test]
    fn rime_toggle_shortcut_parses_backslash_variants() {
        assert_eq!(
            RimeToggleShortcut::parse("cmd-\\"),
            Some(RimeToggleShortcut::CmdBackslash)
        );
        assert_eq!(
            RimeToggleShortcut::parse("ctrl-\\"),
            Some(RimeToggleShortcut::CtrlBackslash)
        );
        assert_eq!(
            RimeToggleShortcut::from_keystroke(
                &gpui::Keystroke::parse("cmd-\\").expect("cmd-backslash should parse")
            ),
            Some(RimeToggleShortcut::CmdBackslash)
        );
        assert_eq!(
            RimeToggleShortcut::from_keystroke(
                &gpui::Keystroke::parse("ctrl-\\").expect("ctrl-backslash should parse")
            ),
            Some(RimeToggleShortcut::CtrlBackslash)
        );
    }
}
