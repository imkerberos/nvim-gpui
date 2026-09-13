use crate::{
    app::{NvimGpui, THEMED_TITLEBAR_HEIGHT},
    editor::image_store::ImageId,
    grid,
    grid::GridElement,
    input as input_core,
    input::InputTarget,
    settings,
    widgets::{ACCENT, BACKGROUND, MUTED_TEXT, SURFACE, SURFACE_BRIGHT},
};
use gpui::{
    div, font, img, point, prelude::*, px, rgb, size, App, Bounds, Context, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, FontFallbacks, Image, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent, Task, Window,
};
use nvim_gpui::rime::{RimeContextSnapshot, RimeService};
use std::{
    collections::HashMap,
    ops::Range,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

mod compositor;
mod compositor_layers;
mod compositor_state;
pub(crate) mod image_store;
mod input;
mod layout;
mod protocol;
mod render;
mod rime;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use layout::parse_guifont_spec;
pub(crate) use layout::{initial_window_size_for_grid, GuiFontSpec};
pub(crate) use protocol::{
    GridCommit, GridLayerKind, GridPlacement, MultiCursorPosition, ProtocolOutcome, ProtocolState,
    RedrawCommit,
};

pub(crate) const VIEWPORT_SCROLL_DURATION: Duration = Duration::from_millis(140);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditorState {
    pub(crate) mode: String,
    pub(crate) file: &'static str,
    pub(crate) line: usize,
    pub(crate) column: usize,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            mode: "NORMAL".to_owned(),
            file: "src/main.rs",
            line: 1,
            column: 1,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ImageLayer {
    pub(crate) image: ImageId,
    pub(crate) grid: u64,
    pub(crate) row: usize,
    pub(crate) column: usize,
    pub(crate) columns: u32,
    pub(crate) rows: u32,
    pub(crate) z_index: i32,
}

#[derive(Clone)]
pub(crate) struct ViewportAnimation {
    pub(crate) previous_grid: Rc<grid::GridModel>,
    pub(crate) scroll_delta: i64,
    pub(crate) started_at: Instant,
    pub(crate) presented: bool,
}

impl ViewportAnimation {
    fn progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.started_at).as_secs_f32()
            / VIEWPORT_SCROLL_DURATION.as_secs_f32())
        .min(1.0)
    }

    pub(crate) fn is_active(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }

    pub(crate) fn mark_presented(&mut self, now: Instant) {
        if !self.presented {
            self.started_at = now;
            self.presented = true;
        }
    }

    pub(crate) fn offsets(
        &self,
        now: Instant,
        max_delta: usize,
        line_height: Pixels,
    ) -> (Pixels, Pixels) {
        let progress = self.progress(now);
        let progress = progress * progress * (3.0 - 2.0 * progress);
        let delta = self
            .scroll_delta
            .clamp(-(max_delta as i64), max_delta as i64) as f32;
        (
            px(-delta * progress * f32::from(line_height)),
            px(delta * (1.0 - progress) * f32::from(line_height)),
        )
    }
}

#[derive(Default)]
pub(crate) struct RenderRuntime {
    pub(crate) viewport_animations: HashMap<u64, ViewportAnimation>,
    pub(crate) image_sources: HashMap<ImageId, Arc<Image>>,
    pub(crate) presentation_snapshot: Option<Rc<compositor::PresentationSnapshot>>,
}

pub(crate) struct InputRuntime {
    pub(crate) input_router: input_core::InputRouter,
    pub(crate) last_modifiers: gpui::Modifiers,
    pub(crate) rime_service: Option<RimeService>,
    pub(crate) rime_context: Option<RimeContextSnapshot>,
    pub(crate) rime_menu_open: bool,
    pub(crate) rime_menu_message: Option<String>,
    pub(crate) system_ime: input_core::SystemImeState,
    pub(crate) rime_init_task: Option<Task<()>>,
    pub(crate) rime_deploy_task: Option<Task<()>>,
    pub(crate) mouse_option: String,
    pub(crate) mouse_enabled: bool,
    pub(crate) mouse_capture: Option<u64>,
    pub(crate) nvim_mode: String,
    pub(crate) scroll_remainder: gpui::Point<f32>,
    pub(crate) ime_input_grid: Option<u64>,
    pub(crate) ime_coordinates_dirty: bool,
}

impl Default for InputRuntime {
    fn default() -> Self {
        Self {
            input_router: input_core::InputRouter::default(),
            last_modifiers: gpui::Modifiers::none(),
            rime_service: None,
            rime_context: None,
            rime_menu_open: false,
            rime_menu_message: None,
            system_ime: input_core::SystemImeState::default(),
            rime_init_task: None,
            rime_deploy_task: None,
            mouse_option: "nvi".to_owned(),
            mouse_enabled: true,
            mouse_capture: None,
            nvim_mode: "n".to_owned(),
            scroll_remainder: point(0.0, 0.0),
            ime_input_grid: None,
            ime_coordinates_dirty: true,
        }
    }
}

pub(crate) struct CursorRuntime {
    pub(crate) cursor_blink_started_at: Instant,
    pub(crate) cursor_animation: Option<grid::CursorAnimation>,
    pub(crate) multicursor_namespace_task: Option<Task<()>>,
    pub(crate) multicursor_reconcile_task: Option<Task<()>>,
    pub(crate) multicursor_reconcile_dirty: bool,
}

impl Default for CursorRuntime {
    fn default() -> Self {
        Self {
            cursor_blink_started_at: Instant::now(),
            cursor_animation: None,
            multicursor_namespace_task: None,
            multicursor_reconcile_task: None,
            multicursor_reconcile_dirty: false,
        }
    }
}

pub(crate) struct EditorRuntime {
    pub(crate) protocol: ProtocolState,
    pub(crate) presentation: RenderRuntime,
    pub(crate) input: InputRuntime,
    pub(crate) cursor: CursorRuntime,
    pub(crate) resolved_grid_font: Option<GuiFontSpec>,
    pub(crate) resolved_grid_wide_font: Option<GuiFontSpec>,
    pub(crate) shaping_cache: grid::SharedShapedLineCache,
    pub(crate) nerd_font_family: Option<String>,
    pub(crate) glyph_coverage_cache: grid::SharedGlyphCoverageCache,
    pub(crate) bundled_nerd_font_registered: bool,
}

impl Default for EditorRuntime {
    fn default() -> Self {
        Self {
            protocol: ProtocolState::default(),
            presentation: RenderRuntime::default(),
            input: InputRuntime::default(),
            cursor: CursorRuntime::default(),
            resolved_grid_font: None,
            resolved_grid_wide_font: None,
            shaping_cache: grid::ShapedLineCache::shared(),
            nerd_font_family: None,
            glyph_coverage_cache: grid::GlyphCoverageCache::shared(),
            bundled_nerd_font_registered: false,
        }
    }
}

impl EditorRuntime {
    pub(crate) fn apply_runtime_settings(&mut self, settings: &settings::Settings) {
        self.nerd_font_family = self
            .bundled_nerd_font_registered
            .then(|| settings.nerd_font.family().to_owned());
        self.shaping_cache.borrow_mut().clear();
        self.glyph_coverage_cache.borrow_mut().clear();
        for image in self
            .protocol
            .presentation
            .image_store
            .set_cache_size_mb(settings.image_cache_size_mb)
        {
            self.presentation.image_sources.remove(&image);
        }
    }
}
