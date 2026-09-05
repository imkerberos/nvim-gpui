//! Reusable native librime backend.
//!
//! This module owns the C ABI boundary and session lifecycle. It deliberately
//! does not know about GPUI, Neovim, or how a candidate window is rendered.

use std::env;
use std::ffi::{c_char, c_int, CStr, CString};
use std::fs;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use libloading::Library;

type Bool = c_int;
type RimeSessionId = usize;

// These values are part of librime's public key_table.h ABI. Keep them here
// rather than depending on X11 headers: the same key event representation is
// used by librime on macOS, Linux, and Windows.
pub const RIME_SHIFT_MASK: c_int = 1 << 0;
pub const RIME_CONTROL_MASK: c_int = 1 << 2;
pub const RIME_ALT_MASK: c_int = 1 << 3;
pub const RIME_SUPER_MASK: c_int = 1 << 26;
pub const RIME_RELEASE_MASK: c_int = 1 << 30;

#[repr(C)]
struct RimeTraits {
    data_size: c_int,
    shared_data_dir: *const c_char,
    user_data_dir: *const c_char,
    distribution_name: *const c_char,
    distribution_code_name: *const c_char,
    distribution_version: *const c_char,
    app_name: *const c_char,
    modules: *const *const c_char,
    min_log_level: c_int,
    log_dir: *const c_char,
    prebuilt_data_dir: *const c_char,
    staging_dir: *const c_char,
}

#[repr(C)]
#[derive(Default)]
struct RimeComposition {
    length: c_int,
    cursor_pos: c_int,
    sel_start: c_int,
    sel_end: c_int,
    preedit: *mut c_char,
}

#[repr(C)]
#[derive(Default)]
struct RimeCandidateRaw {
    text: *mut c_char,
    comment: *mut c_char,
    reserved: *mut std::ffi::c_void,
}

#[repr(C)]
#[derive(Default)]
struct RimeMenu {
    page_size: c_int,
    page_no: c_int,
    is_last_page: Bool,
    highlighted_candidate_index: c_int,
    num_candidates: c_int,
    candidates: *mut RimeCandidateRaw,
    select_keys: *mut c_char,
}

#[repr(C)]
struct RimeContext {
    data_size: c_int,
    composition: RimeComposition,
    menu: RimeMenu,
    commit_text_preview: *mut c_char,
    select_labels: *mut *mut c_char,
}

#[repr(C)]
struct RimeCommit {
    data_size: c_int,
    text: *mut c_char,
}

#[repr(C)]
#[derive(Default)]
struct RimeStatus {
    data_size: c_int,
    schema_id: *mut c_char,
    schema_name: *mut c_char,
    is_disabled: Bool,
    is_composing: Bool,
    is_ascii_mode: Bool,
    is_full_shape: Bool,
    is_simplified: Bool,
    is_traditional: Bool,
    is_ascii_punct: Bool,
}

type Setup = unsafe extern "C" fn(*mut RimeTraits);
type Initialize = unsafe extern "C" fn(*mut RimeTraits);
type Finalize = unsafe extern "C" fn();
type DeployerInitialize = unsafe extern "C" fn(*mut RimeTraits);
type Prebuild = unsafe extern "C" fn() -> Bool;
type Deploy = unsafe extern "C" fn() -> Bool;
type CreateSession = unsafe extern "C" fn() -> RimeSessionId;
type DestroySession = unsafe extern "C" fn(RimeSessionId) -> Bool;
type ProcessKey = unsafe extern "C" fn(RimeSessionId, c_int, c_int) -> Bool;
type GetCommit = unsafe extern "C" fn(RimeSessionId, *mut RimeCommit) -> Bool;
type FreeCommit = unsafe extern "C" fn(*mut RimeCommit) -> Bool;
type GetContext = unsafe extern "C" fn(RimeSessionId, *mut RimeContext) -> Bool;
type FreeContext = unsafe extern "C" fn(*mut RimeContext) -> Bool;
type GetStatus = unsafe extern "C" fn(RimeSessionId, *mut RimeStatus) -> Bool;
type FreeStatus = unsafe extern "C" fn(*mut RimeStatus) -> Bool;

// Keep the prefix through free_status in lockstep with RimeApi in
// librime's rime_api.h. RimeApi is self-versioned; the data_size check below
// prevents calling a library that predates the fields this backend needs.
#[repr(C)]
struct RimeApi {
    data_size: c_int,
    setup: Option<Setup>,
    set_notification_handler: Option<
        unsafe extern "C" fn(
            Option<
                unsafe extern "C" fn(
                    *mut std::ffi::c_void,
                    RimeSessionId,
                    *const c_char,
                    *const c_char,
                ),
            >,
            *mut std::ffi::c_void,
        ),
    >,
    initialize: Option<Initialize>,
    finalize: Option<Finalize>,
    start_maintenance: Option<unsafe extern "C" fn(Bool) -> Bool>,
    is_maintenance_mode: Option<unsafe extern "C" fn() -> Bool>,
    join_maintenance_thread: Option<unsafe extern "C" fn()>,
    deployer_initialize: Option<DeployerInitialize>,
    prebuild: Option<Prebuild>,
    deploy: Option<Deploy>,
    deploy_schema: Option<unsafe extern "C" fn(*const c_char) -> Bool>,
    deploy_config_file: Option<unsafe extern "C" fn(*const c_char, *const c_char) -> Bool>,
    sync_user_data: Option<unsafe extern "C" fn() -> Bool>,
    create_session: Option<CreateSession>,
    find_session: Option<unsafe extern "C" fn(RimeSessionId) -> Bool>,
    destroy_session: Option<DestroySession>,
    cleanup_stale_sessions: Option<unsafe extern "C" fn()>,
    cleanup_all_sessions: Option<unsafe extern "C" fn()>,
    process_key: Option<ProcessKey>,
    commit_composition: Option<unsafe extern "C" fn(RimeSessionId) -> Bool>,
    clear_composition: Option<unsafe extern "C" fn(RimeSessionId)>,
    get_commit: Option<GetCommit>,
    free_commit: Option<FreeCommit>,
    get_context: Option<GetContext>,
    free_context: Option<FreeContext>,
    get_status: Option<GetStatus>,
    free_status: Option<FreeStatus>,
}

/// Paths and initialization policy for one librime instance.
pub struct RimeConfig {
    pub library: Option<PathBuf>,
    pub shared_data: PathBuf,
    pub user_data: PathBuf,
    pub prebuilt_data: Option<PathBuf>,
    pub staging_data: Option<PathBuf>,
    pub deploy: bool,
}

const RIME_LIBRARY_ENV: &str = "NVIM_GPUI_RIME_LIBRARY";
const RIME_SHARED_DATA_ENV: &str = "NVIM_GPUI_RIME_SHARED_DIR";

/// The origin of a runtime candidate, used in diagnostics and discovery
/// tests. Explicit paths are never silently replaced by another source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RimeRuntimeSource {
    Explicit,
    Environment,
    Bundled,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RimeRuntimeCandidate {
    path: PathBuf,
    source: RimeRuntimeSource,
}

/// Resolves librime and shared-data locations without knowing how the GUI
/// stores settings. The resolver deliberately describes candidates rather
/// than downloading or copying a runtime; the platform packaging tasks
/// provide the bundled candidates.
#[derive(Debug, Clone)]
pub struct RimeRuntimeResolver {
    executable: Option<PathBuf>,
}

impl Default for RimeRuntimeResolver {
    fn default() -> Self {
        Self {
            executable: env::current_exe().ok(),
        }
    }
}

impl RimeRuntimeResolver {
    /// Resolve an existing shared-data directory. An explicit path is the
    /// only candidate when supplied; automatic discovery may fall back from
    /// an invalid environment override to the bundled or system locations.
    pub fn resolve_shared_data(&self, explicit: Option<&Path>) -> Result<PathBuf, String> {
        let candidates = self.shared_data_candidates(explicit);
        for candidate in &candidates {
            if let Some(path) = normalized_shared_data_path(&candidate.path) {
                return Ok(path);
            }
        }

        let attempted = candidates
            .iter()
            .map(|candidate| format!("  {} ({:?})", candidate.path.display(), candidate.source))
            .collect::<Vec<_>>()
            .join("\n");
        Err(format!(
            "could not find Rime shared data; tried:\n{attempted}"
        ))
    }

    fn library_candidates(&self, explicit: Option<&Path>) -> Vec<RimeRuntimeCandidate> {
        if let Some(path) = explicit {
            return library_path_candidates(path, RimeRuntimeSource::Explicit);
        }

        let mut candidates = Vec::new();
        if let Some(path) = non_empty_env_path(RIME_LIBRARY_ENV) {
            candidates.extend(library_path_candidates(
                &path,
                RimeRuntimeSource::Environment,
            ));
        }

        for directory in self.bundled_library_directories() {
            candidates.extend(platform_library_names().into_iter().map(|name| {
                RimeRuntimeCandidate {
                    path: directory.join(name),
                    source: RimeRuntimeSource::Bundled,
                }
            }));
        }

        candidates.extend(
            platform_library_names()
                .into_iter()
                .map(|name| RimeRuntimeCandidate {
                    path: PathBuf::from(name),
                    source: RimeRuntimeSource::System,
                }),
        );
        deduplicate_candidates(candidates)
    }

    fn shared_data_candidates(&self, explicit: Option<&Path>) -> Vec<RimeRuntimeCandidate> {
        if let Some(path) = explicit {
            return vec![RimeRuntimeCandidate {
                path: path.to_owned(),
                source: RimeRuntimeSource::Explicit,
            }];
        }

        let mut candidates = Vec::new();
        if let Some(path) = non_empty_env_path(RIME_SHARED_DATA_ENV) {
            candidates.push(RimeRuntimeCandidate {
                path,
                source: RimeRuntimeSource::Environment,
            });
        }

        for directory in self.bundled_runtime_directories() {
            candidates.push(RimeRuntimeCandidate {
                path: directory.join("data"),
                source: RimeRuntimeSource::Bundled,
            });
            candidates.push(RimeRuntimeCandidate {
                path: directory.join("share/rime-data"),
                source: RimeRuntimeSource::Bundled,
            });
            candidates.push(RimeRuntimeCandidate {
                path: directory.join("rime-data"),
                source: RimeRuntimeSource::Bundled,
            });
        }

        if let Some(executable) = self.executable.as_deref() {
            if let Some(bin_dir) = executable.parent() {
                candidates.push(RimeRuntimeCandidate {
                    path: bin_dir.join("rime-data"),
                    source: RimeRuntimeSource::Bundled,
                });
                if cfg!(target_os = "macos") {
                    if let Some(contents_dir) = bin_dir.parent() {
                        candidates.push(RimeRuntimeCandidate {
                            path: contents_dir.join("Resources/rime-data"),
                            source: RimeRuntimeSource::Bundled,
                        });
                    }
                }
            }
        }

        candidates.extend(system_shared_data_directories().into_iter().map(|path| {
            RimeRuntimeCandidate {
                path,
                source: RimeRuntimeSource::System,
            }
        }));
        deduplicate_candidates(candidates)
    }

    fn bundled_runtime_directories(&self) -> Vec<PathBuf> {
        let Some(executable) = self.executable.as_deref() else {
            return Vec::new();
        };
        let Some(bin_dir) = executable.parent() else {
            return Vec::new();
        };

        let mut directories = Vec::new();
        if cfg!(target_os = "macos") {
            if let Some(contents_dir) = bin_dir.parent() {
                directories.push(contents_dir.join("Resources/rime"));
            }
        }
        directories.push(bin_dir.join("rime"));
        directories.push(bin_dir.join("rime-runtime"));
        deduplicate_paths(directories)
    }

    fn bundled_library_directories(&self) -> Vec<PathBuf> {
        let Some(executable) = self.executable.as_deref() else {
            return Vec::new();
        };
        let Some(bin_dir) = executable.parent() else {
            return Vec::new();
        };

        let mut directories = Vec::new();
        if cfg!(target_os = "macos") {
            if let Some(contents_dir) = bin_dir.parent() {
                directories.push(contents_dir.join("Frameworks"));
                directories.push(contents_dir.join("Resources/rime/lib"));
                directories.push(contents_dir.join("Resources/rime"));
            }
        }
        directories.push(bin_dir.join("rime/lib"));
        directories.push(bin_dir.join("rime"));
        directories.push(bin_dir.join("rime-runtime/lib"));
        directories.push(bin_dir.join("rime-runtime"));
        deduplicate_paths(directories)
    }

    #[cfg(test)]
    fn from_executable(executable: PathBuf) -> Self {
        Self {
            executable: Some(executable),
        }
    }
}

/// One candidate exposed by librime's current context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RimeCandidate {
    pub text: String,
    pub comment: Option<String>,
}

/// Owned, UI-independent snapshot of a session's composition and candidates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RimeContextSnapshot {
    pub preedit: String,
    /// UTF-8 byte offset of the caret inside `preedit`, as reported by
    /// librime's `RimeComposition.cursor_pos`.
    pub cursor_pos: usize,
    pub candidates: Vec<RimeCandidate>,
    pub highlighted_candidate_index: c_int,
    pub page_no: usize,
    pub is_last_page: bool,
}

struct RimeTraitsStorage {
    shared_data: CString,
    user_data: CString,
    app_name: CString,
    log_dir: CString,
    prebuilt_data: Option<CString>,
    staging_data: Option<CString>,
    traits: RimeTraits,
}

impl RimeTraitsStorage {
    fn new(config: &RimeConfig, shared_data: &Path) -> Result<Box<Self>, String> {
        let shared_data = c_string_path(shared_data, "shared data")?;
        let user_data = c_string_path(&config.user_data, "user data")?;
        let app_name = CString::new("rime.nvimgpui").map_err(|error| error.to_string())?;
        let log_dir = CString::new("").map_err(|error| error.to_string())?;
        let prebuilt_data = config
            .prebuilt_data
            .as_deref()
            .map(|path| c_string_path(path, "prebuilt data"))
            .transpose()?;
        let staging_data = config
            .staging_data
            .as_deref()
            .map(|path| c_string_path(path, "staging data"))
            .transpose()?;

        // The Box is allocated before these pointers are assigned, so moving
        // the Box itself does not move the CString storage they reference.
        let mut storage = Box::new(Self {
            shared_data,
            user_data,
            app_name,
            log_dir,
            prebuilt_data,
            staging_data,
            traits: RimeTraits {
                data_size: 0,
                shared_data_dir: std::ptr::null(),
                user_data_dir: std::ptr::null(),
                distribution_name: std::ptr::null(),
                distribution_code_name: std::ptr::null(),
                distribution_version: std::ptr::null(),
                app_name: std::ptr::null(),
                modules: std::ptr::null(),
                min_log_level: 0,
                log_dir: std::ptr::null(),
                prebuilt_data_dir: std::ptr::null(),
                staging_dir: std::ptr::null(),
            },
        });
        storage.traits = RimeTraits {
            data_size: (size_of::<RimeTraits>() - size_of::<c_int>()) as c_int,
            shared_data_dir: storage.shared_data.as_ptr(),
            user_data_dir: storage.user_data.as_ptr(),
            distribution_name: std::ptr::null(),
            distribution_code_name: std::ptr::null(),
            distribution_version: std::ptr::null(),
            app_name: storage.app_name.as_ptr(),
            modules: std::ptr::null(),
            min_log_level: 2,
            log_dir: storage.log_dir.as_ptr(),
            prebuilt_data_dir: storage
                .prebuilt_data
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
            staging_dir: storage
                .staging_data
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
        };
        Ok(storage)
    }
}

struct LoadedRime {
    _library: Library,
    api: *const RimeApi,
}

impl LoadedRime {
    fn load(explicit: Option<&Path>) -> Result<Self, String> {
        let candidates = RimeRuntimeResolver::default().library_candidates(explicit);
        let mut errors = Vec::new();

        for candidate in &candidates {
            // SAFETY: loading a user-specified or platform-discovered shared
            // library is the purpose of this backend. The handle is retained.
            let library = match unsafe { Library::new(&candidate.path) } {
                Ok(library) => library,
                Err(error) => {
                    errors.push(format!(
                        "{} ({:?}): {error}",
                        candidate.path.display(),
                        candidate.source
                    ));
                    continue;
                }
            };

            // SAFETY: the symbol name and ABI are defined by librime's public
            // C API. The Library remains owned by LoadedRime.
            let get_api = match unsafe {
                library.get::<unsafe extern "C" fn() -> *const RimeApi>(b"rime_get_api\0")
            } {
                Ok(symbol) => *symbol,
                Err(error) => {
                    errors.push(format!(
                        "{} ({:?}): missing rime_get_api: {error}",
                        candidate.path.display(),
                        candidate.source
                    ));
                    continue;
                }
            };
            let api = unsafe { get_api() };
            return Ok(Self {
                _library: library,
                api,
            });
        }

        Err(format!(
            "could not load librime; tried:\n{}",
            errors
                .into_iter()
                .map(|error| format!("  {error}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }

    fn api(&self) -> Result<&RimeApi, String> {
        if self.api.is_null() {
            return Err("rime_get_api returned a null pointer".to_owned());
        }

        // SAFETY: the API pointer is owned by the loaded librime and the
        // Library is kept alive for the entire LoadedRime value.
        let api = unsafe { &*self.api };
        let required_size = size_of::<RimeApi>() - size_of::<c_int>();
        if api.data_size < required_size as c_int {
            return Err(format!(
                "librime API is too old: data_size={}, required at least {}",
                api.data_size, required_size
            ));
        }
        Ok(api)
    }
}

/// Initialized librime engine. Dropping it finalizes the native instance.
pub struct RimeBackend {
    library: LoadedRime,
    traits: Box<RimeTraitsStorage>,
    session: Option<RimeSessionId>,
    initialized: bool,
}

impl RimeBackend {
    pub fn new(config: RimeConfig) -> Result<Self, String> {
        let shared_data = resolve_shared_data(&config.shared_data)?;
        ensure_directory(&config.user_data, "user data")?;
        if let Some(path) = &config.prebuilt_data {
            ensure_directory(path, "prebuilt data")?;
        }
        if let Some(path) = &config.staging_data {
            ensure_directory(path, "staging data")?;
        }
        if config.deploy && (config.prebuilt_data.is_none() || config.staging_data.is_none()) {
            return Err(
                "deploy requires explicit writable prebuilt_data and staging_data directories"
                    .to_owned(),
            );
        }

        let traits = RimeTraitsStorage::new(&config, &shared_data)?;
        let library = LoadedRime::load(config.library.as_deref())?;
        let mut backend = Self {
            library,
            traits,
            session: None,
            initialized: false,
        };

        let setup = required(backend.library.api()?.setup, "setup")?;
        unsafe {
            setup(&mut backend.traits.traits);
        }
        backend.initialized = true;

        if config.deploy {
            backend.deploy_data()?;
        }

        let initialize = required(backend.library.api()?.initialize, "initialize")?;
        unsafe {
            initialize(&mut backend.traits.traits);
        }
        let create_session = required(backend.library.api()?.create_session, "create_session")?;
        let session = unsafe { create_session() };
        if session == 0 {
            return Err("librime could not create a session".to_owned());
        }
        backend.session = Some(session);
        Ok(backend)
    }

    fn deploy_data(&mut self) -> Result<(), String> {
        let api = self.library.api()?;
        let deployer_initialize = required(api.deployer_initialize, "deployer_initialize")?;
        let prebuild = required(api.prebuild, "prebuild")?;
        let deploy = required(api.deploy, "deploy")?;

        unsafe {
            deployer_initialize(&mut self.traits.traits);
            if prebuild() == 0 {
                return Err("librime could not prebuild data".to_owned());
            }
            if deploy() == 0 {
                return Err("librime could not deploy data".to_owned());
            }
        }
        Ok(())
    }

    /// Rebuild and redeploy the configured Rime data using the current
    /// librime instance.
    pub fn redeploy(&mut self) -> Result<(), String> {
        self.deploy_data()
    }
}

impl Drop for RimeBackend {
    fn drop(&mut self) {
        if !self.initialized {
            return;
        }
        let Ok(api) = self.library.api() else {
            return;
        };
        if let (Some(session), Some(destroy_session)) = (self.session, api.destroy_session) {
            // SAFETY: this session id was returned by create_session and is
            // destroyed before the librime instance is finalized.
            unsafe {
                destroy_session(session);
            }
        }
        if let Some(finalize) = api.finalize {
            // SAFETY: setup completed successfully.
            unsafe { finalize() };
        }
    }
}

impl RimeBackend {
    fn session_id(&self) -> Result<RimeSessionId, String> {
        self.session
            .ok_or_else(|| "librime session is not available".to_owned())
    }

    pub fn process_key(&self, keycode: c_int, modifiers: c_int) -> Result<bool, String> {
        let process_key = required(self.library.api()?.process_key, "process_key")?;
        Ok(unsafe { process_key(self.session_id()?, keycode, modifiers) != 0 })
    }

    pub fn context(&self) -> Result<RimeContextSnapshot, String> {
        let api = self.library.api()?;
        let get_context = required(api.get_context, "get_context")?;
        let free_context = required(api.free_context, "free_context")?;
        let mut context = RimeContext {
            data_size: (size_of::<RimeContext>() - size_of::<c_int>()) as c_int,
            composition: RimeComposition::default(),
            menu: RimeMenu::default(),
            commit_text_preview: std::ptr::null_mut(),
            select_labels: std::ptr::null_mut(),
        };
        if unsafe { get_context(self.session_id()?, &mut context) } == 0 {
            return Err("librime could not return the session context".to_owned());
        }

        let result = (|| {
            // librime uses a null pointer when the composition is empty. This
            // is the normal state immediately after a candidate is committed
            // (for example, when Space commits the highlighted candidate).
            let preedit =
                optional_c_string(context.composition.preedit, "preedit")?.unwrap_or_default();
            let cursor_pos = utf8_boundary_at_or_before(
                &preedit,
                usize::try_from(context.composition.cursor_pos)
                    .unwrap_or_default()
                    .min(preedit.len()),
            );
            let candidate_count = context.menu.num_candidates.max(0) as usize;
            let candidates = if candidate_count == 0 || context.menu.candidates.is_null() {
                Vec::new()
            } else {
                // SAFETY: librime returned an array with num_candidates entries
                // and owns it until free_context.
                let candidates =
                    unsafe { std::slice::from_raw_parts(context.menu.candidates, candidate_count) };
                candidates
                    .iter()
                    .map(|candidate| {
                        Ok(RimeCandidate {
                            text: c_string_from_ptr(candidate.text, "candidate text")?,
                            comment: optional_c_string(candidate.comment, "candidate comment")?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?
            };
            Ok(RimeContextSnapshot {
                preedit,
                cursor_pos,
                candidates,
                highlighted_candidate_index: context.menu.highlighted_candidate_index,
                page_no: usize::try_from(context.menu.page_no).unwrap_or_default(),
                is_last_page: context.menu.is_last_page != 0,
            })
        })();

        unsafe {
            free_context(&mut context);
        }
        result
    }

    pub fn take_commit(&self) -> Result<Option<String>, String> {
        let api = self.library.api()?;
        let get_commit = required(api.get_commit, "get_commit")?;
        let free_commit = required(api.free_commit, "free_commit")?;
        let mut commit = RimeCommit {
            data_size: (size_of::<RimeCommit>() - size_of::<c_int>()) as c_int,
            text: std::ptr::null_mut(),
        };
        if unsafe { get_commit(self.session_id()?, &mut commit) } == 0 {
            return Ok(None);
        }

        let result = c_string_from_ptr(commit.text, "commit text");
        unsafe {
            free_commit(&mut commit);
        }
        result.map(Some)
    }

    pub fn clear_composition(&self) -> Result<(), String> {
        let clear_composition =
            required(self.library.api()?.clear_composition, "clear_composition")?;
        unsafe {
            clear_composition(self.session_id()?);
        }
        Ok(())
    }

    pub fn is_ascii_mode(&self) -> Result<bool, String> {
        let api = self.library.api()?;
        let get_status = required(api.get_status, "get_status")?;
        let free_status = required(api.free_status, "free_status")?;
        let mut status = RimeStatus {
            data_size: (size_of::<RimeStatus>() - size_of::<c_int>()) as c_int,
            ..Default::default()
        };
        if unsafe { get_status(self.session_id()?, &mut status) } == 0 {
            return Err("librime could not return the session status".to_owned());
        }
        let is_ascii_mode = status.is_ascii_mode != 0;
        unsafe {
            free_status(&mut status);
        }
        Ok(is_ascii_mode)
    }
}

fn ensure_directory(path: &Path, label: &str) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("failed to create {label} {}: {error}", path.display()))
}

pub fn resolve_shared_data(path: &Path) -> Result<PathBuf, String> {
    normalized_shared_data_path(path).ok_or_else(|| {
        format!(
            "shared data directory does not exist or is not a directory: {}",
            path.display()
        )
    })
}

fn normalized_shared_data_path(path: &Path) -> Option<PathBuf> {
    if !path.is_dir() {
        return None;
    }

    let nix_data = path.join("share/rime-data");
    if nix_data.is_dir() {
        return Some(nix_data);
    }

    Some(path.to_owned())
}

fn library_path_candidates(path: &Path, source: RimeRuntimeSource) -> Vec<RimeRuntimeCandidate> {
    if path.is_dir() {
        return platform_library_names()
            .into_iter()
            .map(|name| RimeRuntimeCandidate {
                path: path.join(name),
                source,
            })
            .collect();
    }
    vec![RimeRuntimeCandidate {
        path: path.to_owned(),
        source,
    }]
}

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn system_shared_data_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if cfg!(target_os = "linux") {
        directories.extend([
            PathBuf::from("/usr/share/rime-data"),
            PathBuf::from("/usr/local/share/rime-data"),
            PathBuf::from("/usr/share/rime"),
            PathBuf::from("/usr/local/share/rime"),
        ]);
    } else if cfg!(target_os = "macos") {
        directories.extend([
            PathBuf::from("/opt/homebrew/share/rime-data"),
            PathBuf::from("/usr/local/share/rime-data"),
        ]);
    }
    directories
}

fn deduplicate_candidates(candidates: Vec<RimeRuntimeCandidate>) -> Vec<RimeRuntimeCandidate> {
    let mut unique: Vec<RimeRuntimeCandidate> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if unique.iter().any(|seen| seen.path == candidate.path) {
            continue;
        }
        unique.push(candidate);
    }
    unique
}

fn deduplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::with_capacity(paths.len());
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

fn platform_library_names() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["rime.dll"]
    } else if cfg!(target_os = "macos") {
        vec!["librime.dylib", "librime.1.dylib"]
    } else {
        vec!["librime.so", "librime.so.1"]
    }
}

pub fn library_description(explicit: Option<&Path>) -> String {
    explicit
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "platform search path".to_owned())
}

fn required<T>(function: Option<T>, name: &str) -> Result<T, String> {
    function.ok_or_else(|| format!("librime API does not provide {name}"))
}

fn c_string_path(path: &Path, label: &str) -> Result<CString, String> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|error| format!("{label} path contains an embedded NUL: {error}"))
}

fn c_string_from_ptr(pointer: *const c_char, label: &str) -> Result<String, String> {
    if pointer.is_null() {
        return Err(format!("librime returned a null pointer for {label}"));
    }
    // SAFETY: librime returns NUL-terminated strings for these API fields;
    // the owning structure is freed by the caller after conversion.
    Ok(unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned())
}

fn optional_c_string(pointer: *const c_char, label: &str) -> Result<Option<String>, String> {
    if pointer.is_null() {
        return Ok(None);
    }
    c_string_from_ptr(pointer, label).map(Some)
}

fn utf8_boundary_at_or_before(text: &str, offset: usize) -> usize {
    if text.is_char_boundary(offset) {
        return offset;
    }
    (0..offset)
        .rev()
        .find(|candidate| text.is_char_boundary(*candidate))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_shared_data, RimeBackend, RimeConfig, RimeRuntimeResolver, RimeRuntimeSource,
        RIME_CONTROL_MASK, RIME_RELEASE_MASK,
    };
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn direct_data_directory_is_preserved() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        assert_eq!(resolve_shared_data(&path).unwrap(), path);
    }

    #[test]
    fn resolver_discovers_bundled_shared_data_next_to_executable() {
        let root = env::temp_dir().join(format!(
            "nvim-gpui-rime-resolver-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is before the Unix epoch")
                .as_nanos()
        ));
        let executable = root.join("bin/nvim-gpui");
        let shared_data = root.join("bin/rime/data");
        fs::create_dir_all(&shared_data).expect("create bundled data directory");

        let resolver = RimeRuntimeResolver::from_executable(executable);
        assert!(resolver
            .shared_data_candidates(None)
            .iter()
            .any(|candidate| {
                candidate.path == shared_data && candidate.source == RimeRuntimeSource::Bundled
            }));

        fs::remove_dir_all(root).expect("remove resolver test directory");
    }

    #[test]
    fn explicit_library_directory_keeps_explicit_precedence() {
        let root = env::temp_dir().join(format!(
            "nvim-gpui-rime-library-resolver-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is before the Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create explicit library directory");

        let candidates = RimeRuntimeResolver::from_executable(root.join("bin/nvim-gpui"))
            .library_candidates(Some(&root));
        assert!(!candidates.is_empty());
        assert!(candidates
            .iter()
            .all(|candidate| candidate.source == RimeRuntimeSource::Explicit));
        assert!(candidates
            .iter()
            .all(|candidate| candidate.path.parent() == Some(root.as_path())));

        fs::remove_dir_all(root).expect("remove library resolver test directory");
    }

    #[test]
    #[ignore = "requires a local librime shared library and Rime data"]
    fn backend_commits_nihao() {
        let library = env::var_os("NVIM_GPUI_RIME_LIBRARY").map(PathBuf::from);
        let shared_data = env::var_os("NVIM_GPUI_RIME_SHARED_DIR")
            .map(PathBuf::from)
            .expect("NVIM_GPUI_RIME_SHARED_DIR is required for the ignored smoke test");
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos();
        let root = env::temp_dir()
            .join("nvim-gpui-rime-backend-test")
            .join(format!("{}-{nonce}", std::process::id()));
        let user_data = root.join("user");
        let prebuilt_data = root.join("prebuilt");
        let staging_data = root.join("staging");

        let result = (|| {
            let backend = RimeBackend::new(RimeConfig {
                library,
                shared_data,
                user_data,
                prebuilt_data: Some(prebuilt_data),
                staging_data: Some(staging_data),
                deploy: true,
            })?;

            // F35 is intentionally outside the default Rime bindings. Both
            // the plain key and a modified combination must be reported as
            // unconsumed so the application can forward each original event
            // to Neovim once.
            assert!(!backend.process_key(0xffe0, 0)?);
            assert!(!backend.process_key(0xffe0, RIME_CONTROL_MASK)?);
            assert!(backend.take_commit()?.is_none());

            for key in b"nihao" {
                assert!(backend.process_key(*key as i32, 0)?);
            }
            let context = backend.context()?;
            assert_eq!(context.preedit, "ni hao");
            assert_eq!(context.cursor_pos, context.preedit.len());
            assert!(!context.candidates.is_empty());
            assert!(backend.process_key(b' ' as i32, 0)?);
            assert!(backend.context()?.preedit.is_empty());
            assert_eq!(backend.take_commit()?.as_deref(), Some("你好"));

            for key in b"nihao" {
                assert!(backend.process_key(*key as i32, 0)?);
            }
            let before = backend.context()?;
            assert!(backend.process_key(0xff51, 0)?);
            let moved = backend.context()?;
            assert!(moved.cursor_pos < before.cursor_pos);

            // The bundled default.yaml uses Shift_L as a press-and-release
            // ASCII mode switch. The release itself is a kNoop at the engine
            // API boundary even though it changes the status, so verify the
            // state rather than treating process_key's bool as consumption.
            assert!(!backend.is_ascii_mode()?);
            assert!(!backend.process_key(0xffe1, 0)?);
            assert!(!backend.process_key(0xffe1, RIME_RELEASE_MASK)?);
            assert!(backend.is_ascii_mode()?);
            Ok::<(), String>(())
        })();

        let _ = fs::remove_dir_all(&root);
        result.expect("librime backend smoke test failed");
    }
}
