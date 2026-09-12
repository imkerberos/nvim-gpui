use super::NvimGpui;
#[cfg(test)]
use crate::editor::image_store::ImageId;
use crate::editor::image_store::KittyEvent;
use crate::editor::{ProtocolOutcome, RedrawCommit};
use crate::{
    grid,
    input::InputTarget,
    nvim::{DisconnectReason, NvimEvent},
};
use gpui::{Context, Image};
use std::{sync::Arc, time::Instant};

impl NvimGpui {
    #[cfg(test)]
    /// Apply a protocol event through the same path used by the runtime.
    pub(crate) fn apply_nvim_event_for_test(&mut self, event: NvimEvent) {
        let _ = self.process_nvim_event(event);
    }

    pub(crate) fn handle_nvim_event(&mut self, event: NvimEvent, cx: &mut Context<Self>) {
        if let Some(reason) = self.process_nvim_event(event) {
            self.handle_disconnect(reason, cx);
        }
    }

    fn process_nvim_event(&mut self, event: NvimEvent) -> Option<DisconnectReason> {
        let previous_cursor = self.current_cursor_screen_position();
        let outcome = self.editor.protocol.apply(event);
        self.apply_protocol_outcome(outcome, previous_cursor)
    }

    fn apply_protocol_outcome(
        &mut self,
        outcome: ProtocolOutcome,
        previous_cursor: Option<grid::CursorVisualPosition>,
    ) -> Option<DisconnectReason> {
        match outcome {
            ProtocolOutcome::ApiReady(version) => {
                log::info!(
                    target: "nvim_gpui::app",
                    "Neovim API ready: version={version}, api_level={}",
                    version.api_level
                );
                self.app.session.api_level = Some(version.api_level);
                self.app.session.nvim_version = Some(version);
                self.app.session.rpc_status = format!(
                    "rpc: Neovim {} / API {}",
                    self.app
                        .session
                        .nvim_version
                        .as_ref()
                        .expect("version was just stored"),
                    self.app
                        .session
                        .api_level
                        .expect("API level was just stored")
                );
            }
            ProtocolOutcome::UiAttached {
                width,
                height,
                redraw,
            } => {
                log::info!(
                    target: "nvim_gpui::app",
                    "Neovim UI attached: width={width}, height={height}"
                );
                self.apply_redraw_commit(redraw, previous_cursor);
                self.app.session.rpc_status = format!("rpc: attached {width}×{height}");
            }
            ProtocolOutcome::PendingChanged => {
                self.editor.input.ime_coordinates_dirty = true;
            }
            ProtocolOutcome::Flushed(redraw) => {
                self.apply_redraw_commit(redraw, previous_cursor);
                self.invalidate_presentation_snapshot();
                self.editor.input.ime_coordinates_dirty = true;
            }
            ProtocolOutcome::Error(error) => {
                log::error!(target: "nvim_gpui::app", "Neovim event error: {error}");
                self.app.session.rpc_status = format!("rpc error: {error}");
            }
            ProtocolOutcome::Disconnected(reason) => {
                log::info!(
                    target: "nvim_gpui::app",
                    "Neovim disconnected: reason={reason:?}"
                );
                self.app.session.rpc_status = "rpc: disconnected".to_owned();
                return Some(reason);
            }
        }
        None
    }

    fn apply_redraw_commit(
        &mut self,
        redraw: RedrawCommit,
        previous_cursor: Option<grid::CursorVisualPosition>,
    ) {
        self.editor.input.input_router = redraw.input_router;
        self.window.window_title = redraw.window_title;
        self.window.window_icon = redraw.window_icon;
        self.editor.input.mouse_option = redraw.mouse_option;
        self.editor.input.mouse_enabled = redraw.mouse_enabled;
        self.editor.input.nvim_mode = redraw.nvim_mode;
        if redraw.cursor_timing_reset {
            self.editor.cursor.cursor_blink_started_at = Instant::now();
        }

        for grid in redraw.destroyed_grids {
            self.editor.presentation.viewport_animations.remove(&grid);
        }
        self.apply_viewport_commits(redraw.grid_commits);
        self.apply_kitty_events(redraw.kitty_events);
        if self.editor.input.input_router.target() != InputTarget::Rime {
            self.reset_rime_composition();
        }
        if self.editor.input.input_router.target() != InputTarget::SystemIme {
            self.editor.input.system_ime.clear();
        }
        if redraw.font_changed || redraw.font_wide_changed {
            self.editor.resolved_grid_font = None;
            self.editor.resolved_grid_wide_font = None;
            self.editor.input.ime_coordinates_dirty = true;
            self.app.last_resize = None;
        }
        if redraw.linespace_changed {
            self.editor.input.ime_coordinates_dirty = true;
            self.app.last_resize = None;
        }
        if redraw.display_options_changed {
            self.editor.shaping_cache.borrow_mut().clear();
        }

        // The old animation code needs the committed cursor position. Keep
        // its retargeting behavior in the application bridge until the animation
        // itself is extracted from `grid_state`.
        if previous_cursor != self.current_cursor_screen_position() {
            self.update_cursor_animation_from(previous_cursor);
        }
    }

    fn apply_kitty_events(&mut self, events: Vec<KittyEvent>) {
        for event in events {
            match event {
                KittyEvent::AssetUpdated { image, .. } => {
                    if let Some(asset) = self.editor.protocol.presentation.image_store.asset(image)
                    {
                        let source =
                            Image::from_bytes(asset.format.gpui_format(), asset.encoded.clone());
                        self.editor
                            .presentation
                            .image_sources
                            .insert(image, Arc::new(source));
                    }
                }
                KittyEvent::AssetDeleted { image } => {
                    self.editor.presentation.image_sources.remove(&image);
                }
                KittyEvent::AssetsCleared => {
                    self.editor.presentation.image_sources.clear();
                }
                KittyEvent::TerminalResponse(response) => {
                    if let Some(nvim) = self.app.session.nvim.as_ref() {
                        if let Err(error) = nvim.send_term_event("termresponse", response) {
                            log::error!(
                                target: "nvim_gpui::nvim",
                                "failed to forward Neovim terminal response: {error}"
                            );
                            self.app.session.rpc_status =
                                format!("rpc terminal response error: {error}");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_resize_is_not_visible_before_flush() {
        let mut app = NvimGpui::default();

        app.apply_nvim_event_for_test(NvimEvent::GridResized {
            grid: 1,
            width: 12,
            height: 3,
        });

        assert_eq!(
            (
                app.editor.protocol.presentation.grid.width(),
                app.editor.protocol.presentation.grid.height()
            ),
            (80, 24)
        );
        assert_eq!(app.editor.protocol.presentation.grid_size, None);
        assert_eq!(
            app.editor.protocol.presentation.pending_grid_size,
            Some(Some((12, 3)))
        );

        app.apply_nvim_event_for_test(NvimEvent::Flush);

        assert_eq!(
            (
                app.editor.protocol.presentation.grid.width(),
                app.editor.protocol.presentation.grid.height()
            ),
            (12, 3)
        );
        assert_eq!(app.editor.protocol.presentation.grid_size, Some((12, 3)));
        assert!(app.editor.protocol.presentation.pending_grid_size.is_none());
    }

    #[test]
    fn ui_send_is_not_visible_before_flush() {
        let mut app = NvimGpui::default();
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAQMAAAAl21bKAAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAAGUExURf8AAP///0EdNBEAAAABYktHRAH/Ai3eAAAACklEQVQI12NgAAAAAgAB4iG8MwAAAABJRU5ErkJggg==";
        let data = format!("\x1b_Ga=T,f=100,t=d,i=7,m=0;{png}\x1b\\",);

        app.apply_nvim_event_for_test(NvimEvent::UiSend { data });

        assert!(app
            .editor
            .protocol
            .presentation
            .image_store
            .asset(ImageId(7))
            .is_none());
        assert_eq!(app.editor.protocol.presentation.pending_ui_data.len(), 1);

        app.apply_nvim_event_for_test(NvimEvent::Flush);

        assert!(app
            .editor
            .protocol
            .presentation
            .image_store
            .asset(ImageId(7))
            .is_some());
        assert!(app.editor.protocol.presentation.pending_ui_data.is_empty());
        assert!(app.editor.presentation.presentation_snapshot.is_none());
    }

    #[test]
    fn discard_pending_redraw_drops_unflushed_ui_data() {
        let mut app = NvimGpui::default();

        app.apply_nvim_event_for_test(NvimEvent::UiSend {
            data: "\x1b[>q".to_owned(),
        });
        app.discard_pending_redraw();

        assert!(app.editor.protocol.presentation.pending_ui_data.is_empty());
        assert!(app
            .editor
            .protocol
            .presentation
            .image_store
            .asset(ImageId(7))
            .is_none());
    }
}
