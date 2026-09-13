use super::*;
use crate::input::{rime_key_event, rime_modifier_transition, InputContext, InputTarget};
use gpui::{AppContext, Context, ModifiersChangedEvent, Window};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::settings;
use nvim_gpui::rime::{RimeConfig, RimeRuntimeResolver, RimeService};

impl EditorRuntime {
    pub(crate) fn test_rime_configuration_with_settings(
        app_settings: settings::Settings,
    ) -> Result<(), String> {
        let config = rime_config_from_settings(&app_settings, Some(false))?;
        let service = RimeService::start(config)?;
        service.context().map(|_| ()).map_err(|error| {
            format!("Rime session was created but context loading failed: {error}")
        })
    }

    pub(crate) fn reset_rime_composition(&mut self) {
        self.input.rime_context = None;
        if let Some(service) = self.input.rime_service.as_ref() {
            if let Err(error) = service.clear_composition() {
                log::debug!(
                    target: "nvim_gpui::rime",
                    "could not clear Rime composition: {error}"
                );
            }
        }
    }

    pub(crate) fn disable_rime(&mut self, reason: &str) {
        self.reset_rime_composition();
        self.input.rime_service = None;
        self.input.rime_menu_open = false;
        self.input.rime_menu_message = None;
        self.input.input_router.disable_rime();
        self.protocol.disable_rime();
        log::warn!(target: "nvim_gpui::rime", "Rime disabled: {reason}");
    }

    pub(crate) fn toggle_rime(&mut self) -> bool {
        if self.input.rime_service.is_none() {
            log::debug!(
                target: "nvim_gpui::rime",
                "Rime toggle ignored because the backend is unavailable"
            );
            return false;
        }

        self.input.rime_menu_open = false;
        self.input.rime_menu_message = None;
        let context = self.input.input_router.rime_toggle_context();
        let enabled = !self.input.input_router.rime_enabled_for(context);
        self.input
            .input_router
            .set_rime_enabled_for(context, enabled);
        self.protocol.set_rime_enabled_for(context, enabled);
        self.reset_rime_composition();
        self.input.system_ime.clear();
        log::info!(
            target: "nvim_gpui::rime",
            "Rime {}",
            if enabled { "enabled" } else { "disabled" }
        );
        true
    }

    pub(crate) fn open_rime_menu(&mut self) -> bool {
        if self.input.rime_service.is_none() {
            return false;
        }
        self.input.rime_menu_open = true;
        self.input.rime_menu_message = None;
        true
    }

    pub(crate) fn close_rime_menu(&mut self) -> bool {
        if !self.input.rime_menu_open && self.input.rime_menu_message.is_none() {
            return false;
        }
        self.input.rime_menu_open = false;
        self.input.rime_menu_message = None;
        true
    }

    pub(crate) fn apply_ime_backend_setting(&mut self, backend: settings::ImeBackend) {
        let rime_enabled =
            backend == settings::ImeBackend::Rime && self.input.rime_service.is_some();
        if rime_enabled {
            // Selecting Rime enables the Insert-mode route. Other text
            // contexts intentionally remain English until toggled there.
            self.protocol
                .set_rime_enabled_for(InputContext::Insert, true);
            self.input
                .input_router
                .set_rime_enabled_for(InputContext::Insert, true);
        } else {
            self.protocol.disable_rime();
            self.input.input_router.disable_rime();
        }
        if !rime_enabled {
            self.reset_rime_composition();
            self.input.rime_menu_open = false;
            self.input.rime_menu_message = None;
        }
        self.input.system_ime.clear();
    }
}

impl NvimGpui {
    pub(crate) fn start_rime_initialization(&mut self, cx: &mut Context<Self>) {
        if self.editor.input.rime_init_task.is_some() {
            return;
        }

        let app_settings = self.app.settings.clone();
        let config = match rime_config_from_settings(&app_settings, None) {
            Ok(config) => config,
            Err(error) => {
                log::warn!(target: "nvim_gpui::rime", "Rime service unavailable: {error}");
                return;
            }
        };
        let task = cx.background_spawn(async move { RimeService::start(config) });
        self.editor.input.rime_init_task = Some(cx.spawn(async move |weak, cx| {
            let service_result = task.await;
            let _ = weak.update(cx, |view, cx| {
                view.editor.input.rime_init_task = None;
                view.editor.input.rime_service = match service_result {
                    Ok(service) => {
                        log::info!(target: "nvim_gpui::rime", "Rime service available");
                        Some(service)
                    }
                    Err(error) => {
                        log::warn!(target: "nvim_gpui::rime", "Rime service failed to start: {error}");
                        None
                    }
                };
                cx.notify();
            });
        }));
    }

    fn send_rime_commit(&mut self, text: String, cx: &mut Context<Self>) -> Result<(), String> {
        let Some(nvim) = self.app.session.nvim.as_ref() else {
            return Err("Neovim is unavailable".to_owned());
        };
        let bytes = text.len();
        let response = nvim.send_paste(text)?;
        log::debug!(
            target: "nvim_gpui::rime",
            "forwarding Rime commit through nvim_paste: bytes={bytes}"
        );
        cx.spawn(async move |_weak, _cx| match response.recv().await {
            Ok(Ok(rmpv::Value::Boolean(false))) => log::warn!(
                target: "nvim_gpui::rime",
                "Neovim rejected Rime commit"
            ),
            Ok(Ok(value)) => log::debug!(
                target: "nvim_gpui::rime",
                "Neovim accepted Rime commit: {value:?}"
            ),
            Ok(Err(error)) => log::error!(
                target: "nvim_gpui::rime",
                "Rime commit failed in Neovim: {error}"
            ),
            Err(error) => log::error!(
                target: "nvim_gpui::rime",
                "Rime commit response was lost: {error}"
            ),
        })
        .detach();
        Ok(())
    }

    fn handle_rime_keycode(
        &mut self,
        keycode: i32,
        modifiers: i32,
        reset_on_unconsumed: bool,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        let Some(service) = self.editor.input.rime_service.as_ref() else {
            return Ok(false);
        };
        let result = service.process_key(keycode, modifiers)?;
        let consumed = result.consumed;
        let context = result.context;
        let commit = result.commit;
        log::debug!(
            target: "nvim_gpui::rime",
            "processed key: keycode={keycode}, consumed={consumed}, has_commit={}",
            commit.is_some()
        );
        if !consumed {
            // A key can be left unconsumed while librime still has commit
            // text buffered (for example, when punctuation commits the
            // candidate). Forward that commit first, then return false so
            // the original key continues through Neovim's normal path.
            if let Some(text) = commit {
                log::debug!(
                    target: "nvim_gpui::rime",
                    "Rime produced a commit before forwarding an unconsumed key: bytes={}",
                    text.len()
                );
                self.send_rime_commit(text, cx)?;
            }
            if reset_on_unconsumed {
                self.editor.reset_rime_composition();
            }
            cx.notify();
            return Ok(false);
        }
        self.editor.input.rime_context =
            (!context.preedit.is_empty() || !context.candidates.is_empty()).then_some(context);
        if let Some(text) = commit {
            log::debug!(
                target: "nvim_gpui::rime",
                "Rime produced a commit: bytes={}",
                text.len()
            );
            self.editor.input.rime_context = None;
            self.send_rime_commit(text, cx)?;
        }
        cx.notify();
        Ok(true)
    }

    pub(crate) fn handle_rime_key(
        &mut self,
        keystroke: &gpui::Keystroke,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        let Some((keycode, modifiers)) = rime_key_event(keystroke) else {
            log::debug!(
                target: "nvim_gpui::rime",
                "Rime key mapping rejected: key={:?}, key_char={:?}, modifiers={:?}",
                keystroke.key,
                keystroke.key_char,
                keystroke.modifiers
            );
            self.editor.reset_rime_composition();
            return Ok(false);
        };
        self.handle_rime_keycode(keycode, modifiers, true, cx)
    }

    pub(crate) fn on_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.editor.input.last_modifiers;
        self.editor.input.last_modifiers = event.modifiers;

        if self.editor.input.input_router.target() != InputTarget::Rime
            || self.editor.input.rime_service.is_none()
            || self.editor.input.rime_deploy_task.is_some()
        {
            return;
        }

        // GPUI represents modifier-only input as an aggregate state change.
        // Reconstruct the individual press/release events so Rime can run
        // bindings such as its press-and-release Shift ASCII switcher.
        let transitions = [
            ("shift", previous.shift, event.modifiers.shift),
            ("control", previous.control, event.modifiers.control),
            ("alt", previous.alt, event.modifiers.alt),
            ("platform", previous.platform, event.modifiers.platform),
        ];
        for (key, was_pressed, is_pressed) in transitions {
            if was_pressed == is_pressed {
                continue;
            }
            let Some((keycode, modifiers)) =
                rime_modifier_transition(key, previous, event.modifiers)
            else {
                continue;
            };
            match self.handle_rime_keycode(keycode, modifiers, false, cx) {
                Ok(true) => window.prevent_default(),
                Ok(false) => log::debug!(
                    target: "nvim_gpui::rime",
                    "Rime did not consume modifier transition: key={key}, pressed={is_pressed}"
                ),
                Err(error) => {
                    self.editor.disable_rime(&error);
                    break;
                }
            }
        }
    }

    pub(crate) fn redeploy_rime(&mut self, cx: &mut Context<Self>) {
        if self.editor.input.rime_deploy_task.is_some() {
            return;
        }
        self.editor.reset_rime_composition();
        let Some(service) = self.editor.input.rime_service.take() else {
            self.editor.input.rime_menu_message = Some("Rime backend is unavailable".to_owned());
            self.editor.input.rime_menu_open = true;
            cx.notify();
            return;
        };

        let config = match rime_config_from_settings(&self.app.settings, Some(true)) {
            Ok(config) => config,
            Err(error) => {
                self.editor.input.rime_service = Some(service);
                self.editor.input.rime_menu_message =
                    Some(format!("Rime redeploy failed: {error}"));
                self.editor.input.rime_menu_open = true;
                cx.notify();
                return;
            }
        };
        self.editor.input.rime_menu_message = Some("Deploying Rime data…".to_owned());
        self.editor.input.rime_menu_open = true;
        cx.notify();

        let task = cx.background_spawn(async move {
            let result = service.redeploy(config);
            (service, result)
        });
        self.editor.input.rime_deploy_task = Some(cx.spawn(async move |weak, cx| {
            let (service, redeploy_result) = task.await;
            let _ = weak.update(cx, |view, cx| {
                view.editor.input.rime_deploy_task = None;
                let (service, message) = match redeploy_result {
                    Ok(()) => {
                        log::info!(
                            target: "nvim_gpui::rime",
                            "Rime data redeployed from titlebar menu"
                        );
                        (
                            Some(service),
                            "Rime data redeployed successfully.".to_owned(),
                        )
                    }
                    Err(error) => {
                        log::error!(
                            target: "nvim_gpui::rime",
                            "Rime service could not be recreated after deploy: {error}"
                        );
                        drop(service);
                        (None, format!("Rime redeploy failed: {error}"))
                    }
                };
                view.editor.input.rime_service = service;
                view.editor.input.rime_menu_message = Some(message);
                view.editor.input.rime_menu_open = true;
                cx.notify();
            });
        }));
    }

    pub(crate) fn open_rime_user_data_directory(&mut self, cx: &mut Context<Self>) {
        let result = settings::rime_user_data_directory()
            .ok_or_else(|| "could not determine the Rime user data directory".to_owned())
            .and_then(|path| crate::logging::open_directory(&path));

        match result {
            Ok(()) => {
                log::info!(target: "nvim_gpui::rime", "opened Rime user data directory");
                self.editor.input.rime_menu_open = false;
                self.editor.input.rime_menu_message = None;
            }
            Err(error) => {
                log::error!(
                    target: "nvim_gpui::rime",
                    "could not open Rime user data directory: {error}"
                );
                self.editor.input.rime_menu_message =
                    Some(format!("Could not open user settings: {error}"));
                self.editor.input.rime_menu_open = true;
            }
        }
        cx.notify();
    }
}

fn rime_config_from_settings(
    app_settings: &settings::Settings,
    deploy_override: Option<bool>,
) -> Result<RimeConfig, String> {
    let bundled_runtime = cfg!(any(target_os = "macos", target_os = "windows"));
    let resolver = RimeRuntimeResolver::default();
    let shared_data = if bundled_runtime {
        resolver.resolve_shared_data(None)?
    } else if !app_settings.rime_data_dir.trim().is_empty() {
        PathBuf::from(app_settings.rime_data_dir.trim())
    } else if app_settings.rime_library_auto_detect {
        resolver.resolve_shared_data(None)?
    } else {
        env::var_os("NVIM_GPUI_RIME_SHARED_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| "Rime shared data directory is not configured".to_owned())?
    };
    let user_data = settings::rime_user_data_directory()
        .ok_or_else(|| "Rime user data directory is not available".to_owned())?;
    // Keep librime's writable staging output under its default user-data
    // location without exposing the internal directory as a setting.
    let staging_data = user_data.join("build");
    let deploy = deploy_override.unwrap_or_else(|| {
        env::var("NVIM_GPUI_RIME_DEPLOY")
            .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
            .unwrap_or_else(|_| !directory_has_entries(&staging_data))
    });

    Ok(RimeConfig {
        library: if bundled_runtime {
            Some(resolver.resolve_library_directory(None)?)
        } else if app_settings.rime_library_auto_detect {
            env::var_os("NVIM_GPUI_RIME_LIBRARY")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        } else {
            configured_path(&app_settings.rime_library_dir, "NVIM_GPUI_RIME_LIBRARY")
        },
        shared_data,
        user_data,
        staging_data: Some(staging_data),
        deploy,
    })
}

fn configured_path(setting: &str, environment: &str) -> Option<PathBuf> {
    (!setting.trim().is_empty())
        .then(|| PathBuf::from(setting.trim()))
        .or_else(|| env::var_os(environment).map(PathBuf::from))
}

fn directory_has_entries(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some()
}
