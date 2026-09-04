use super::element::GridElement;
use super::*;

pub(crate) struct CursorGlyph {
    pub(crate) line: ShapedLine,
}

/// A cursor that is positioned in editor-wide screen coordinates.
///
/// GridElement keeps the cursor attached to the grid that owns it, which is
/// correct for a stationary cursor. During a cross-window move, however, the
/// cursor must travel between grid bounds. This small overlay lets the app
/// render that transition without duplicating a cursor in either grid.
pub struct CursorElement {
    position: CursorVisualPosition,
    local_position: CursorVisualPosition,
    animation: Option<CursorAnimation>,
    color: Hsla,
    glyph_foreground: Hsla,
    glyph_source: Option<GridElement>,
    cell_width: Pixels,
    line_height: Pixels,
    width: usize,
    height: usize,
    cursor_mode: CursorModeInfo,
    blink_started_at: Instant,
}

impl CursorElement {
    pub(crate) fn new(
        position: CursorVisualPosition,
        color: Hsla,
        cursor_mode: CursorModeInfo,
    ) -> Self {
        Self {
            position,
            local_position: position,
            animation: None,
            color,
            glyph_foreground: color,
            glyph_source: None,
            cell_width: px(10.0),
            line_height: px(22.0),
            width: 0,
            height: 0,
            cursor_mode,
            blink_started_at: Instant::now(),
        }
    }

    pub(crate) fn with_animation(mut self, animation: Option<CursorAnimation>) -> Self {
        self.animation = animation;
        self
    }

    pub(crate) fn with_local_position(mut self, position: CursorVisualPosition) -> Self {
        self.local_position = position;
        self
    }

    pub(crate) fn with_glyph_foreground(mut self, foreground: Hsla) -> Self {
        self.glyph_foreground = foreground;
        self
    }

    pub(crate) fn with_glyph_source(mut self, source: Option<GridElement>) -> Self {
        self.glyph_source = source;
        self
    }

    pub(crate) fn with_metrics(mut self, cell_width: Pixels, line_height: Pixels) -> Self {
        self.cell_width = cell_width;
        self.line_height = line_height;
        self
    }

    pub(crate) fn with_grid_size(mut self, width: usize, height: usize) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub(crate) fn with_blink_started_at(mut self, started_at: Instant) -> Self {
        self.blink_started_at = started_at;
        self
    }
}

pub struct CursorTrail {
    bounds: Bounds<Pixels>,
    opacity: f32,
}

pub struct CursorPrepaintState {
    trails: Vec<CursorTrail>,
    glyph_position: Option<CursorVisualPositionF>,
    glyph: Option<ShapedLine>,
}

impl IntoElement for CursorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CursorElement {
    type RequestLayoutState = ();
    type PrepaintState = CursorPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = (self.cell_width * self.width).into();
        style.size.height = (self.line_height * self.height).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let now = Instant::now();
        if self.cursor_mode.blink_enabled()
            && !self.cursor_mode.visible_at(self.blink_started_at, now)
        {
            window.request_animation_frame();
            return CursorPrepaintState {
                trails: Vec::new(),
                glyph_position: None,
                glyph: None,
            };
        }

        let (trails, glyph_position) =
            if let Some(animation) = self.animation.filter(|animation| animation.is_active(now)) {
                window.request_animation_frame();

                const TRAIL: [(u64, f32); 5] =
                    [(56, 0.05), (42, 0.08), (28, 0.13), (14, 0.22), (0, 1.0)];
                let trails = TRAIL
                    .into_iter()
                    .map(|(age_ms, opacity)| {
                        let sample_time = now
                            .checked_sub(Duration::from_millis(age_ms))
                            .unwrap_or(animation.started_at);
                        CursorTrail {
                            bounds: animated_cursor_bounds(
                                bounds,
                                self.cell_width,
                                self.line_height,
                                animation,
                                self.cursor_mode,
                                sample_time,
                            ),
                            opacity,
                        }
                    })
                    .collect::<Vec<_>>();
                (trails, Some(animation.position_at(now)))
            } else {
                (
                    vec![CursorTrail {
                        bounds: cursor_bounds_at(
                            bounds,
                            self.cell_width,
                            self.line_height,
                            self.position.into(),
                            self.cursor_mode,
                        ),
                        opacity: 1.0,
                    }],
                    Some(self.position.into()),
                )
            };

        let glyph = (self.cursor_mode.shape == CursorShape::Block)
            .then(|| {
                self.glyph_source.as_mut()?.cursor_glyph(
                    window,
                    self.local_position,
                    self.glyph_foreground,
                )
            })
            .flatten()
            .map(|glyph| glyph.line);

        CursorPrepaintState {
            trails,
            glyph_position,
            glyph,
        }
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for trail in prepaint.trails.drain(..) {
                let radius = px((f32::from(trail.bounds.size.width)
                    .min(f32::from(trail.bounds.size.height))
                    .mul_add(0.18, 0.0))
                .clamp(2.0, 6.0));
                window.paint_quad(
                    fill(trail.bounds, self.color.opacity(trail.opacity))
                        .corner_radii(Corners::all(radius)),
                );
            }

            if let (Some(glyph), Some(position)) = (prepaint.glyph.take(), prepaint.glyph_position)
            {
                let origin = point(
                    bounds.origin.x + self.cell_width * position.col,
                    bounds.origin.y + self.line_height * position.row,
                );
                glyph
                    .paint(origin, self.line_height, window, _cx)
                    .expect("failed to paint cursor glyph");
            }
        });
    }
}

#[cfg(test)]
pub(super) fn cursor_bounds(
    grid_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    position: CursorVisualPosition,
    mode: CursorModeInfo,
) -> Bounds<Pixels> {
    cursor_bounds_at(grid_bounds, cell_width, line_height, position.into(), mode)
}

#[cfg(test)]
pub(crate) fn cursor_colors(
    model: &GridModel,
    position: CursorVisualPosition,
    mode: CursorModeInfo,
) -> (Hsla, Hsla) {
    cursor_colors_with_context(model, position, mode, HighlightContext::Main)
}

pub(crate) fn cursor_colors_with_context(
    model: &GridModel,
    position: CursorVisualPosition,
    mode: CursorModeInfo,
    context: HighlightContext,
) -> (Hsla, Hsla) {
    let default_colors = resolve_highlight(model, DEFAULT_HIGHLIGHT, context);
    let default_background = default_colors
        .background
        .unwrap_or_else(|| rgb(DEFAULT_BACKGROUND).into());
    let cell_highlight = model
        .rows()
        .get(position.row)
        .and_then(|row| row.cells().get(position.col))
        .map(|cell| cell.highlight)
        .unwrap_or(DEFAULT_HIGHLIGHT);

    match mode.attr_id {
        // Neovim defines attr_id 0 as a request to swap the current cell's
        // foreground and background, rather than as a normal highlight id.
        Some(DEFAULT_HIGHLIGHT) => {
            let cell_style = resolve_highlight(model, cell_highlight, context);
            (
                cell_style.background.unwrap_or(default_colors.foreground),
                cell_style.foreground,
            )
        }
        Some(attr_id) => {
            let style = resolve_highlight(model, attr_id, context);
            (
                style.foreground,
                style.background.unwrap_or(default_background),
            )
        }
        None => (default_background, rgb(BLUE_FOREGROUND).into()),
    }
}

fn animated_cursor_bounds(
    grid_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    animation: CursorAnimation,
    mode: CursorModeInfo,
    now: Instant,
) -> Bounds<Pixels> {
    let progress = animation.progress(now);
    let position = animation.position_at(now);
    let from = animation.from;
    let to = animation.to;

    let base = cursor_bounds_at(grid_bounds, cell_width, line_height, position, mode);
    if progress >= 1.0 {
        return base;
    }

    // Estimate the instantaneous velocity from two nearby animation samples.
    // The velocity, rather than only the total distance, makes a short key
    // press feel soft and makes a large jump visibly stretch at launch.
    let previous_time = now
        .checked_sub(Duration::from_millis(8))
        .unwrap_or(animation.started_at);
    let previous_position = animation.position_at(previous_time);
    let interval = 0.008;
    let velocity_col = (position.col - previous_position.col) / interval;
    let velocity_row = (position.row - previous_position.row) / interval;
    let delta_x = (to.col - from.col) * f32::from(cell_width);
    let delta_y = (to.row - from.row) * f32::from(line_height);
    let distance = (delta_x / f32::from(cell_width))
        .abs()
        .max((delta_y / f32::from(line_height)).abs());
    if distance == 0.0 && velocity_col == 0.0 && velocity_row == 0.0 {
        return base;
    }

    // A jelly cursor stretches with its current speed and relaxes towards the
    // target. The distance term keeps a jump between split windows readable,
    // while the cap prevents a redraw storm from producing a huge cursor.
    let velocity = velocity_col.hypot(velocity_row);
    let speed_factor = (velocity / 12.0).clamp(0.0, 1.0);
    let distance_factor = (distance / 8.0).clamp(0.0, 1.0);
    let settle = (PI * progress).sin().max(0.0);
    let stretch_ratio =
        (0.055 + 0.22 * speed_factor + 0.06 * distance_factor + 0.025 * settle).min(0.4);

    let base_width = f32::from(base.size.width);
    let base_height = f32::from(base.size.height);
    let horizontal = velocity_col.abs().max(delta_x.abs()) >= velocity_row.abs().max(delta_y.abs());
    let (x, y, width, height) = if horizontal {
        let extra = f32::from(cell_width) * stretch_ratio;
        let height = (base_height * (1.0 - stretch_ratio * 0.42)).max(1.0);
        let direction = if velocity_col.abs() > 0.001 {
            velocity_col
        } else {
            delta_x
        };
        (
            f32::from(base.origin.x) - if direction > 0.0 { extra } else { 0.0 },
            f32::from(base.origin.y) + (base_height - height) / 2.0,
            base_width + extra,
            height,
        )
    } else {
        let extra = f32::from(line_height) * stretch_ratio;
        let width = (base_width * (1.0 - stretch_ratio * 0.42)).max(1.0);
        let direction = if velocity_row.abs() > 0.001 {
            velocity_row
        } else {
            delta_y
        };
        (
            f32::from(base.origin.x) + (base_width - width) / 2.0,
            f32::from(base.origin.y) - if direction > 0.0 { extra } else { 0.0 },
            width,
            base_height + extra,
        )
    };

    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

pub(super) fn jelly_progress(progress: f32) -> f32 {
    if progress >= 1.0 {
        return 1.0;
    }

    // A restrained ease-out-back curve: the cursor settles a few percent
    // past its destination and returns, which reads as a soft jelly motion
    // rather than a rigid linear slide. The animation is still clamped to a
    // small overshoot so a large cursor jump cannot leave the viewport.
    let x = progress - 1.0;
    let overshoot = 0.75;
    let curve = 1.0 + (overshoot + 1.0) * x.powi(3) + overshoot * x.powi(2);
    curve.clamp(0.0, 1.025)
}

fn cursor_bounds_at(
    grid_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    position: CursorVisualPositionF,
    mode: CursorModeInfo,
) -> Bounds<Pixels> {
    let percentage = f32::from(mode.cell_percentage) / 100.0;
    let origin = point(
        grid_bounds.origin.x + cell_width * position.col,
        grid_bounds.origin.y + line_height * position.row,
    );
    let full_width = cell_width * position.width.max(1.0);

    let (origin, size) = match mode.shape {
        CursorShape::Block => (origin, size(full_width, line_height)),
        CursorShape::Horizontal => (
            point(origin.x, origin.y + line_height * (1.0 - percentage)),
            size(full_width, line_height * percentage),
        ),
        CursorShape::Vertical => (origin, size(full_width * percentage, line_height)),
    };

    Bounds::new(origin, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_cursor_stretches_towards_its_previous_position() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(400.0), px(200.0)));
        let mode = CursorModeInfo::default();
        let animation = CursorAnimation::new(
            CursorVisualPosition {
                row: 0,
                col: 1,
                width: 1,
            },
            CursorVisualPosition {
                row: 0,
                col: 8,
                width: 1,
            },
        );
        let now = animation.started_at + Duration::from_millis(24);
        let position = animation.position_at(now);
        let base = cursor_bounds_at(bounds, px(10.0), px(20.0), position, mode);
        let stretched = animated_cursor_bounds(bounds, px(10.0), px(20.0), animation, mode, now);

        assert!(stretched.size.width > base.size.width);
        assert!(stretched.size.height < base.size.height);
        assert!(stretched.origin.x < base.origin.x);
    }
}
