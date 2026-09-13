use crate::nvim::{NvimProcess, NvimVersion, SessionId};
use crate::settings;
use crate::update_check;
use gpui::Task;
use std::sync::Arc;

/// State owned by the application coordinator rather than by an auxiliary
/// window. Editor protocol and presentation state live in `EditorRuntime`.
#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) session: Session,
    pub(crate) settings: settings::Settings,
    pub(crate) last_resize: Option<(u32, u32)>,
    pub(crate) settings_save_error: Option<String>,
    pub(crate) cli_install_error: Option<String>,
    pub(crate) update_status: update_check::Status,
    pub(crate) update_http_client: Option<Arc<dyn gpui::http_client::HttpClient>>,
    pub(crate) update_check_task: Option<Task<()>>,
    pub(crate) logger: Option<flexi_logger::LoggerHandle>,
}

impl AppState {
    pub(crate) fn settings_snapshot(&self) -> (settings::Settings, Option<String>, Option<String>) {
        (
            self.settings.clone(),
            self.settings_save_error.clone(),
            self.cli_install_error.clone(),
        )
    }

    pub(crate) fn settings_value(&self) -> settings::Settings {
        self.settings.clone()
    }

    pub(crate) fn set_cli_install_error(&mut self, error: Option<String>) {
        self.cli_install_error = error;
    }
}

/// State for the current Neovim connection.
pub(crate) struct Session {
    pub(crate) nvim: Option<NvimProcess>,
    pub(crate) session_id: Option<SessionId>,
    pub(crate) rpc_status: String,
    pub(crate) api_level: Option<u64>,
    pub(crate) nvim_version: Option<NvimVersion>,
    pub(crate) event_task: Option<Task<()>>,
    pub(crate) reconnect_task: Option<Task<()>>,
    pub(crate) reconnect_attempt: u32,
    pub(crate) clipboard_task: Option<Task<()>>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            nvim: None,
            session_id: None,
            rpc_status: "rpc: starting".to_owned(),
            api_level: None,
            nvim_version: None,
            event_task: None,
            reconnect_task: None,
            reconnect_attempt: 0,
            clipboard_task: None,
        }
    }
}

impl Session {
    pub(crate) fn request_startup_redraw(&self) {
        let Some(nvim) = self.nvim.as_ref() else {
            return;
        };
        match nvim.request(
            "nvim_command",
            rmpv::Value::Array(vec![rmpv::Value::from("redraw!")]),
        ) {
            Ok(_) => log::debug!(
                target: "nvim_gpui::app",
                "requested a complete redraw after Neovim startup/reconnect"
            ),
            Err(error) => log::warn!(
                target: "nvim_gpui::app",
                "could not request a complete redraw after Neovim startup/reconnect: {error}"
            ),
        }
    }
}
