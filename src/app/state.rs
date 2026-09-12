use crate::nvim::{NvimProcess, NvimVersion, SessionId};
use crate::settings;
use gpui::Task;

/// State owned by the application coordinator rather than by an auxiliary
/// window. Editor protocol and presentation state live in `EditorRuntime`.
#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) session: Session,
    pub(crate) settings: settings::Settings,
    pub(crate) last_resize: Option<(u32, u32)>,
    pub(crate) settings_save_error: Option<String>,
    pub(crate) cli_install_error: Option<String>,
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
