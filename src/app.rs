pub(crate) const DEFAULT_GRID_WIDTH: u32 = 80;
pub(crate) const DEFAULT_GRID_HEIGHT: u32 = 24;

pub(crate) mod clipboard;
mod events;
mod file_open;
mod multicursor;
mod session;
mod startup;
mod state;
mod titlebar;
mod workspace;

use crate::{editor::EditorRuntime, settings as app_settings, update_check};
use gpui::{AnyWindowHandle, FocusHandle, Subscription, Task};
use std::sync::Arc;

pub(crate) use crate::editor::initial_window_size_for_grid;
pub(crate) use startup::run;
pub(crate) use state::{AppState, Session};
#[cfg(target_os = "linux")]
pub(crate) use titlebar::themed_resize_handles;
pub(crate) use titlebar::{
    themed_titlebar, themed_titlebar_enabled, themed_titlebar_options, themed_window_decorations,
    RimeTitlebarState,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum QuitDialogState {
    #[default]
    Hidden,
    Checking,
    Confirm {
        modified_buffers: Vec<String>,
        error: Option<String>,
    },
    Saving,
    Quitting,
}

pub(crate) const DEFAULT_GRID_FONT_SIZE: f32 = 14.0;
pub(crate) const DEFAULT_GRID_CELL_WIDTH: f32 = DEFAULT_GRID_FONT_SIZE * 0.6;
pub(crate) const DEFAULT_GRID_LINE_HEIGHT: f32 = 20.0;

#[cfg(target_os = "macos")]
pub(crate) const DEFAULT_GRID_FONT_FAMILY: &str = "Menlo";
#[cfg(target_os = "windows")]
pub(crate) const DEFAULT_GRID_FONT_FAMILY: &str = "Consolas";
#[cfg(target_os = "linux")]
pub(crate) const DEFAULT_GRID_FONT_FAMILY: &str = "DejaVu Sans Mono";
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(crate) const DEFAULT_GRID_FONT_FAMILY: &str = "monospace";

#[cfg(target_os = "macos")]
pub(crate) const PREFERRED_SYSTEM_MONOSPACE_FONTS: &[&str] = &["Menlo", "SF Mono", "Monaco"];
#[cfg(target_os = "windows")]
pub(crate) const PREFERRED_SYSTEM_MONOSPACE_FONTS: &[&str] =
    &["Cascadia Mono", "Consolas", "Courier New"];
#[cfg(target_os = "linux")]
pub(crate) const PREFERRED_SYSTEM_MONOSPACE_FONTS: &[&str] = &[
    "Ubuntu Mono",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Source Code Pro",
];
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(crate) const PREFERRED_SYSTEM_MONOSPACE_FONTS: &[&str] = &["monospace"];

#[cfg(target_os = "macos")]
pub(crate) const PREFERRED_SYSTEM_WIDE_FONTS: &[&str] = &["PingFang SC", "Hiragino Sans GB"];
#[cfg(target_os = "windows")]
pub(crate) const PREFERRED_SYSTEM_WIDE_FONTS: &[&str] =
    &["Microsoft YaHei UI", "Microsoft YaHei", "SimSun"];
#[cfg(target_os = "linux")]
pub(crate) const PREFERRED_SYSTEM_WIDE_FONTS: &[&str] = &[
    "Noto Sans Mono CJK SC",
    "Noto Sans CJK SC",
    "WenQuanYi Zen Hei",
];
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(crate) const PREFERRED_SYSTEM_WIDE_FONTS: &[&str] = &[];

pub(crate) const MIN_WINDOW_WIDTH: f32 = 80.0;
pub(crate) const MIN_WINDOW_HEIGHT: f32 = 44.0;
pub(crate) const THEMED_TITLEBAR_HEIGHT: f32 = 32.0;
pub(crate) const DEFAULT_WINDOW_TITLE: &str = "gpvim";
pub(crate) const LOGO_ASSET: &str = "neovim-gpui.png";
pub(crate) const WINDOW_CONTROL_MINIMIZE_ASSET: &str = "window-controls/minimize.svg";
pub(crate) const WINDOW_CONTROL_MAXIMIZE_ASSET: &str = "window-controls/maximize.svg";
pub(crate) const WINDOW_CONTROL_RESTORE_ASSET: &str = "window-controls/restore.svg";
pub(crate) const WINDOW_CONTROL_CLOSE_ASSET: &str = "window-controls/close.svg";
pub(crate) const DEBUG_WINDOW_HEIGHT: f32 = 240.0;
pub(crate) const MAX_EVENTS_PER_UI_UPDATE: usize = 2048;

/// Window-facing state owned by the application root.
pub(crate) struct WindowRuntime {
    pub(crate) focus_handle: Option<FocusHandle>,
    pub(crate) window_title: String,
    pub(crate) window_icon: String,
    pub(crate) quit_dialog: QuitDialogState,
    pub(crate) open_urls_task: Option<Task<()>>,
    pub(crate) file_drop_notice: Option<String>,
    pub(crate) file_drop_notice_generation: u64,
    pub(crate) file_drop_notice_task: Option<Task<()>>,
    pub(crate) window_bounds_subscription: Option<Subscription>,
    pub(crate) settings_window: Option<AnyWindowHandle>,
    pub(crate) about_window: Option<AnyWindowHandle>,
}

impl Default for WindowRuntime {
    fn default() -> Self {
        Self {
            focus_handle: None,
            window_title: DEFAULT_WINDOW_TITLE.to_owned(),
            window_icon: "nvim-gpui".to_owned(),
            quit_dialog: QuitDialogState::default(),
            open_urls_task: None,
            file_drop_notice: None,
            file_drop_notice_generation: 0,
            file_drop_notice_task: None,
            window_bounds_subscription: None,
            settings_window: None,
            about_window: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct NvimGpui {
    pub(crate) app: AppState,
    pub(crate) editor: EditorRuntime,
    pub(crate) window: WindowRuntime,
    pub(crate) update_http_client: Option<Arc<dyn gpui::http_client::HttpClient>>,
    pub(crate) update_status: update_check::Status,
    pub(crate) update_check_task: Option<Task<()>>,
    pub(crate) logger: Option<flexi_logger::LoggerHandle>,
}

impl NvimGpui {
    pub(crate) fn settings_snapshot(
        &self,
    ) -> (app_settings::Settings, Option<String>, Option<String>) {
        (
            self.app.settings.clone(),
            self.app.settings_save_error.clone(),
            self.app.cli_install_error.clone(),
        )
    }

    pub(crate) fn update_status(&self) -> update_check::Status {
        self.update_status.clone()
    }

    pub(crate) fn settings_value(&self) -> app_settings::Settings {
        self.app.settings.clone()
    }

    pub(crate) fn set_cli_install_error(&mut self, error: Option<String>) {
        self.app.cli_install_error = error;
    }

    pub(crate) fn settings_window_handle(&self) -> Option<AnyWindowHandle> {
        self.window.settings_window
    }

    pub(crate) fn set_settings_window_handle(&mut self, handle: AnyWindowHandle) {
        self.window.settings_window = Some(handle);
    }

    pub(crate) fn about_window_handle(&self) -> Option<AnyWindowHandle> {
        self.window.about_window
    }

    pub(crate) fn set_about_window_handle(&mut self, handle: AnyWindowHandle) {
        self.window.about_window = Some(handle);
    }
}
