use gpui::{
    deferred, div, fill, point, prelude::*, px, relative, rgb, AlignItems, App, Bounds, ClickEvent,
    CursorStyle, DispatchPhase, Element, ElementId, FocusHandle, Font, GlobalElementId,
    HitboxBehavior, Image, InspectorElementId, IntoElement, KeyDownEvent, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ShapedLine, SharedString, Stateful,
    Style, TextRun, Window,
};
use std::{ops::Range, rc::Rc, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;

#[cfg(target_os = "windows")]
use gpui::WindowControlArea;

pub(crate) const BACKGROUND: u32 = 0x1e1e2e;
pub(crate) const SURFACE: u32 = 0x181825;
pub(crate) const SURFACE_BRIGHT: u32 = 0x313244;
pub(crate) const TEXT: u32 = 0xcdd6f4;
pub(crate) const MUTED_TEXT: u32 = 0x7f849c;
pub(crate) const ACCENT: u32 = 0x89b4fa;
pub(crate) const IME_ACTIVE: u32 = 0xa6e3a1;

const MAX_VISIBLE_TEXT_INPUT_CHARS: usize = 48;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingTextInputState {
    pub(crate) value: String,
    pub(crate) cursor: usize,
    pub(crate) selection_anchor: Option<usize>,
    pub(crate) is_selecting: bool,
}

impl SettingTextInputState {
    pub(crate) fn new(value: String) -> Self {
        let cursor = value.len();
        Self {
            value,
            cursor,
            selection_anchor: None,
            is_selecting: false,
        }
    }

    pub(crate) fn selected_range(&self) -> Range<usize> {
        let anchor = self.selection_anchor.unwrap_or(self.cursor);
        anchor.min(self.cursor)..anchor.max(self.cursor)
    }

    pub(crate) fn has_selection(&self) -> bool {
        !self.selected_range().is_empty()
    }

    pub(crate) fn move_left(&mut self, extend: bool) {
        if !extend && self.has_selection() {
            self.move_to(self.selected_range().start, false);
        } else {
            self.move_to(previous_grapheme_boundary(&self.value, self.cursor), extend);
        }
    }

    pub(crate) fn move_right(&mut self, extend: bool) {
        if !extend && self.has_selection() {
            self.move_to(self.selected_range().end, false);
        } else {
            self.move_to(next_grapheme_boundary(&self.value, self.cursor), extend);
        }
    }

    pub(crate) fn move_home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub(crate) fn move_end(&mut self, extend: bool) {
        self.move_to(self.value.len(), extend);
    }

    pub(crate) fn move_to(&mut self, cursor: usize, extend: bool) {
        let cursor = cursor.min(self.value.len());
        if extend {
            self.selection_anchor.get_or_insert(self.cursor);
        } else {
            self.selection_anchor = None;
        }
        self.cursor = cursor;
        if self.selection_anchor == Some(self.cursor) {
            self.selection_anchor = None;
        }
    }

    pub(crate) fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.value.len();
    }

    pub(crate) fn insert_text(&mut self, text: &str) {
        let range = self.selected_range();
        self.value.replace_range(range.clone(), text);
        self.cursor = range.start + text.len();
        self.selection_anchor = None;
    }

    pub(crate) fn backspace(&mut self) {
        let range = if self.has_selection() {
            self.selected_range()
        } else {
            previous_grapheme_boundary(&self.value, self.cursor)..self.cursor
        };
        if !range.is_empty() {
            self.value.replace_range(range.clone(), "");
            self.cursor = range.start;
        }
        self.selection_anchor = None;
    }

    pub(crate) fn delete(&mut self) {
        let range = if self.has_selection() {
            self.selected_range()
        } else {
            self.cursor..next_grapheme_boundary(&self.value, self.cursor)
        };
        if !range.is_empty() {
            self.value.replace_range(range.clone(), "");
        }
        self.selection_anchor = None;
    }

    pub(crate) fn begin_mouse_selection(&mut self, index: usize, extend: bool) {
        self.move_to(index, extend);
        self.is_selecting = true;
    }

    pub(crate) fn extend_mouse_selection(&mut self, index: usize) {
        if self.is_selecting {
            self.move_to(index, true);
        }
    }

    pub(crate) fn end_mouse_selection(&mut self) {
        self.is_selecting = false;
    }
}

fn previous_grapheme_boundary(value: &str, cursor: usize) -> usize {
    value[..cursor.min(value.len())]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_grapheme_boundary(value: &str, cursor: usize) -> usize {
    let cursor = cursor.min(value.len());
    value[cursor..]
        .grapheme_indices(true)
        .nth(1)
        .map_or(value.len(), |(index, _)| cursor + index)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingTextInputMouseEvent {
    Down { index: usize, shift: bool },
    Drag { index: usize },
    Up,
}

type SettingTextInputClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type SettingTextInputKeyHandler = Box<dyn Fn(&KeyDownEvent, &mut Window, &mut App)>;
type SettingTextInputMouseHandler = Box<dyn Fn(SettingTextInputMouseEvent, &mut Window, &mut App)>;
type SharedSettingTextInputMouseHandler =
    Rc<dyn Fn(SettingTextInputMouseEvent, &mut Window, &mut App)>;

pub(crate) struct SettingTextInputConfig {
    id: ElementId,
    state: SettingTextInputState,
    placeholder: SharedString,
    editing: bool,
    focus_handle: FocusHandle,
    on_click: SettingTextInputClickHandler,
    on_key_down: SettingTextInputKeyHandler,
    on_mouse: SettingTextInputMouseHandler,
}

impl SettingTextInputConfig {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        state: SettingTextInputState,
        placeholder: impl Into<SharedString>,
        editing: bool,
        focus_handle: FocusHandle,
    ) -> Self {
        Self {
            id: id.into(),
            state,
            placeholder: placeholder.into(),
            editing,
            focus_handle,
            on_click: Box::new(|_, _, _| {}),
            on_key_down: Box::new(|_, _, _| {}),
            on_mouse: Box::new(|_, _, _| {}),
        }
    }

    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Box::new(handler);
        self
    }

    pub(crate) fn on_key_down(
        mut self,
        handler: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_key_down = Box::new(handler);
        self
    }

    pub(crate) fn on_mouse(
        mut self,
        handler: impl Fn(SettingTextInputMouseEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_mouse = Box::new(handler);
        self
    }
}

#[derive(Clone, Debug)]
struct TextInputDisplay {
    value: SharedString,
    text: SharedString,
    actual_start: usize,
    actual_end: usize,
    leading_ellipsis_len: usize,
    trailing_ellipsis_len: usize,
    cursor: usize,
    is_placeholder: bool,
}

impl TextInputDisplay {
    fn new(value: &str, cursor: usize) -> Self {
        let value: SharedString = value.to_owned().into();
        let cursor = cursor.min(value.len());
        let boundaries: Vec<usize> = value
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(value.len()))
            .collect();
        let grapheme_count = boundaries.len().saturating_sub(1);
        if grapheme_count <= MAX_VISIBLE_TEXT_INPUT_CHARS {
            return Self {
                value: value.clone(),
                text: value.clone(),
                actual_start: 0,
                actual_end: value.len(),
                leading_ellipsis_len: 0,
                trailing_ellipsis_len: 0,
                cursor,
                is_placeholder: false,
            };
        }

        let visible_chars = MAX_VISIBLE_TEXT_INPUT_CHARS.saturating_sub(2);
        let cursor_grapheme = boundaries
            .partition_point(|&boundary| boundary < cursor)
            .min(grapheme_count);
        let mut start = cursor_grapheme.saturating_sub(visible_chars / 2);
        let mut end = (start + visible_chars).min(grapheme_count);
        if end - start < visible_chars {
            start = end.saturating_sub(visible_chars);
            end = (start + visible_chars).min(grapheme_count);
        }
        let leading_ellipsis_len = usize::from(start > 0) * '…'.len_utf8();
        let trailing_ellipsis_len = usize::from(end < grapheme_count) * '…'.len_utf8();
        let mut text = String::new();
        if leading_ellipsis_len > 0 {
            text.push('…');
        }
        text.push_str(&value[boundaries[start]..boundaries[end]]);
        if trailing_ellipsis_len > 0 {
            text.push('…');
        }

        Self {
            value,
            text: text.into(),
            actual_start: boundaries[start],
            actual_end: boundaries[end],
            leading_ellipsis_len,
            trailing_ellipsis_len,
            cursor,
            is_placeholder: false,
        }
    }

    fn map_to_empty(mut self) -> Self {
        self.actual_start = 0;
        self.actual_end = 0;
        self.leading_ellipsis_len = 0;
        self.trailing_ellipsis_len = 0;
        self.cursor = 0;
        self.is_placeholder = true;
        self
    }

    fn fit_to_width(&mut self, max_width: Pixels, mut measure: impl FnMut(&str) -> Pixels) {
        if self.is_placeholder || self.value.is_empty() {
            return;
        }

        let value = self.value.as_ref();
        let boundaries: Vec<usize> = value
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(value.len()))
            .collect();
        let grapheme_count = boundaries.len().saturating_sub(1);
        let cursor_grapheme = boundaries
            .partition_point(|&boundary| boundary < self.cursor)
            .min(grapheme_count);

        let candidate = |start: usize, end: usize| {
            let leading_ellipsis_len = usize::from(start > 0) * '…'.len_utf8();
            let trailing_ellipsis_len = usize::from(end < grapheme_count) * '…'.len_utf8();
            let mut text = String::new();
            if leading_ellipsis_len > 0 {
                text.push('…');
            }
            text.push_str(&value[boundaries[start]..boundaries[end]]);
            if trailing_ellipsis_len > 0 {
                text.push('…');
            }
            (text, leading_ellipsis_len, trailing_ellipsis_len)
        };

        let mut start = 0;
        let mut end = grapheme_count;
        while end.saturating_sub(start) > 1 {
            let (text, _, _) = candidate(start, end);
            if measure(&text) <= max_width {
                break;
            }

            if cursor_grapheme <= start {
                end -= 1;
            } else if cursor_grapheme >= end || cursor_grapheme - start >= end - cursor_grapheme {
                start += 1;
            } else {
                end -= 1;
            }
        }

        let (text, leading_ellipsis_len, trailing_ellipsis_len) = candidate(start, end);
        self.text = text.into();
        self.actual_start = boundaries[start];
        self.actual_end = boundaries[end];
        self.leading_ellipsis_len = leading_ellipsis_len;
        self.trailing_ellipsis_len = trailing_ellipsis_len;
    }

    fn cursor_display_index(&self) -> usize {
        self.map_full_to_display(self.cursor)
    }

    fn selection_display_range(&self, selection: Range<usize>) -> Range<usize> {
        self.map_full_to_display(selection.start)..self.map_full_to_display(selection.end)
    }

    fn map_full_to_display(&self, index: usize) -> usize {
        let index = index.min(self.actual_end.max(self.actual_start));
        if index <= self.actual_start {
            if self.leading_ellipsis_len > 0 && index < self.actual_start {
                return 0;
            }
            return self.leading_ellipsis_len;
        }
        if index >= self.actual_end {
            return self.text.len().saturating_sub(self.trailing_ellipsis_len);
        }
        self.leading_ellipsis_len + index - self.actual_start
    }

    fn map_display_to_full(&self, index: usize) -> usize {
        let index = index.min(self.text.len());
        let content_start = self.leading_ellipsis_len;
        let content_end = self.text.len().saturating_sub(self.trailing_ellipsis_len);
        if index <= content_start {
            return self.actual_start;
        }
        if index >= content_end {
            return self.actual_end;
        }
        self.actual_start + index - content_start
    }
}

struct SettingTextInputText {
    display: TextInputDisplay,
    editing: bool,
    selection: Range<usize>,
    focus_handle: FocusHandle,
    on_mouse: Option<SettingTextInputMouseHandler>,
}

struct SettingTextInputPrepaintState {
    line: ShapedLine,
    hitbox: gpui::Hitbox,
}

impl IntoElement for SettingTextInputText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SettingTextInputText {
    type RequestLayoutState = ();
    type PrepaintState = SettingTextInputPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let font = text_style.font();
        let color = text_style.color;
        self.display.fit_to_width(bounds.size.width, |text| {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text.to_owned().into(), font_size, &[run], None)
                .width
        });
        let text = self.display.text.clone();
        let selection = if self.editing {
            self.display.selection_display_range(self.selection.clone())
        } else {
            0..0
        };
        let run = TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if selection.is_empty() {
            vec![run]
        } else {
            let selected_run = TextRun {
                background_color: None,
                color: rgb(BACKGROUND).into(),
                ..run.clone()
            };
            [
                (0..selection.start, run.clone()),
                (selection.clone(), selected_run),
                (selection.end..text.len(), run),
            ]
            .into_iter()
            .filter(|(range, _)| !range.is_empty())
            .map(|(range, mut run)| {
                run.len = range.end - range.start;
                run
            })
            .collect()
        };
        let line = window
            .text_system()
            .shape_line(text, font_size, &runs, None);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        SettingTextInputPrepaintState { line, hitbox }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line = &mut prepaint.line;
        let line_height = window.line_height();
        let text_top = bounds.origin.y + (bounds.size.height - line_height) / 2.0;
        let origin = point(bounds.origin.x, text_top);
        let selection = if self.editing {
            self.display.selection_display_range(self.selection.clone())
        } else {
            0..0
        };
        if !selection.is_empty() {
            window.paint_quad(fill(
                Bounds::from_corners(
                    point(origin.x + line.x_for_index(selection.start), origin.y),
                    point(
                        origin.x + line.x_for_index(selection.end),
                        origin.y + line_height,
                    ),
                ),
                rgb(ACCENT),
            ));
        }
        let _ = line.paint(origin, line_height, window, cx);

        if self.editing && self.selection.is_empty() && self.focus_handle.is_focused(window) {
            let cursor_x = line.x_for_index(self.display.cursor_display_index());
            window.paint_quad(fill(
                Bounds::new(
                    point(origin.x + cursor_x, origin.y),
                    gpui::size(px(1.0), line_height),
                ),
                rgb(ACCENT),
            ));
        }

        if let Some(on_mouse) = self.on_mouse.take() {
            let on_mouse: SharedSettingTextInputMouseHandler = Rc::from(on_mouse);
            let display = self.display.clone();
            let layout = line.clone();
            let down_hitbox = prepaint.hitbox.clone();
            let down_on_mouse = on_mouse.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == MouseButton::Left
                    && down_hitbox.is_hovered(window)
                {
                    let index =
                        text_input_index_for_position(event.position, &display, &layout, bounds);
                    down_on_mouse(
                        SettingTextInputMouseEvent::Down {
                            index,
                            shift: event.modifiers.shift,
                        },
                        window,
                        cx,
                    );
                }
            });

            let display = self.display.clone();
            let layout = line.clone();
            let move_hitbox = prepaint.hitbox.clone();
            let move_on_mouse = on_mouse.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.dragging()
                    && move_hitbox.is_hovered(window)
                {
                    let index =
                        text_input_index_for_position(event.position, &display, &layout, bounds);
                    move_on_mouse(SettingTextInputMouseEvent::Drag { index }, window, cx);
                }
            });

            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                    on_mouse(SettingTextInputMouseEvent::Up, window, cx);
                }
            });
        }
    }
}

fn text_input_index_for_position(
    position: gpui::Point<Pixels>,
    display: &TextInputDisplay,
    line: &ShapedLine,
    bounds: Bounds<Pixels>,
) -> usize {
    let x = if position.x <= bounds.left() {
        px(0.0)
    } else if position.x >= bounds.right() {
        line.width
    } else {
        position.x - bounds.left()
    };
    display.map_display_to_full(line.closest_index_for_x(x))
}

pub(crate) fn setting_text_input(config: SettingTextInputConfig) -> Stateful<gpui::Div> {
    let SettingTextInputConfig {
        id,
        state,
        placeholder,
        editing,
        focus_handle,
        on_click,
        on_key_down,
        on_mouse,
    } = config;
    let display_is_placeholder = !editing && state.value.is_empty();
    let display_value = if display_is_placeholder {
        placeholder.clone()
    } else {
        state.value.clone().into()
    };
    let display = TextInputDisplay::new(
        display_value.as_ref(),
        if editing {
            state.cursor
        } else {
            display_value.len()
        },
    );
    let display = if display_is_placeholder {
        display.map_to_empty()
    } else {
        display
    };
    let selection = if display_is_placeholder {
        0..0
    } else {
        state.selected_range()
    };
    let text = SettingTextInputText {
        display,
        editing,
        selection,
        focus_handle: focus_handle.clone(),
        on_mouse: Some(on_mouse),
    };
    div()
        .id(id)
        .w_full()
        .min_w_0()
        .flex_grow()
        .flex_shrink()
        .flex_basis(relative(0.0))
        .overflow_hidden()
        .h(px(36.0))
        .flex()
        .items_center()
        .px_3()
        .rounded_sm()
        .border_1()
        .border_color(rgb(if editing { ACCENT } else { SURFACE_BRIGHT }))
        .bg(rgb(SURFACE))
        .text_sm()
        .text_color(rgb(if display_is_placeholder {
            MUTED_TEXT
        } else {
            TEXT
        }))
        .track_focus(&focus_handle)
        .focus(|style| style.border_color(rgb(ACCENT)))
        .hover(|style| style.border_color(rgb(ACCENT)))
        .cursor(CursorStyle::IBeam)
        .on_click(on_click)
        .capture_key_down(on_key_down)
        .child(
            div()
                .w_full()
                .min_w_0()
                .flex_1()
                .h_full()
                .overflow_hidden()
                .child(text),
        )
}

pub(crate) fn setting_combo_box(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    open: bool,
    options: impl IntoElement,
    icon_font: Font,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let mut combo = div().relative().w_full().flex().flex_col();
    combo = combo.child(
        div()
            .id(id)
            .w_full()
            .h(px(36.0))
            .flex()
            .items_center()
            .px_3()
            .rounded_sm()
            .border_1()
            .border_color(rgb(if open { ACCENT } else { SURFACE_BRIGHT }))
            .bg(rgb(SURFACE))
            .text_sm()
            .text_color(rgb(TEXT))
            .cursor_pointer()
            .hover(|style| style.border_color(rgb(ACCENT)))
            .on_click(on_click)
            .child(div().flex_1().child(label))
            .child(
                div()
                    .font(icon_font)
                    .text_color(rgb(MUTED_TEXT))
                    .child(if open { "" } else { "" }),
            ),
    );

    if open {
        combo = combo.child(
            deferred(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(40.0))
                    .w_full()
                    .p_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(SURFACE_BRIGHT))
                    .bg(rgb(SURFACE))
                    .child(options),
            )
            .with_priority(1),
        );
    }

    combo
}

pub(crate) fn setting_combo_option(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    div()
        .id(id)
        .w_full()
        .flex()
        .items_center()
        .px_3()
        .py_2()
        .rounded_sm()
        .text_sm()
        .text_color(rgb(TEXT))
        .bg(rgb(if selected { SURFACE_BRIGHT } else { SURFACE }))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_BRIGHT)))
        .on_click(on_click)
        .child(div().flex_1().child(label))
        .child(
            div()
                .w(px(20.0))
                .text_right()
                .text_color(rgb(ACCENT))
                .child(if selected { "✓" } else { "" }),
        )
}

pub(crate) fn setting_checkbox(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    div()
        .id(id)
        .w_full()
        .h(px(36.0))
        .flex()
        .items_center()
        .gap_2()
        .text_sm()
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .on_click(on_click)
        .child(
            div()
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .border_1()
                .border_color(rgb(if checked { ACCENT } else { SURFACE_BRIGHT }))
                .bg(rgb(if checked { ACCENT } else { SURFACE }))
                .text_sm()
                .text_color(rgb(BACKGROUND))
                .child(if checked { "✓" } else { "" }),
        )
        .child(label)
}

pub(crate) fn setting_option_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    div()
        .id(id)
        .mx(px(2.0))
        .px_3()
        .py_2()
        .rounded_sm()
        .text_sm()
        .flex_shrink_0()
        .bg(rgb(if selected { ACCENT } else { SURFACE_BRIGHT }))
        .text_color(rgb(if selected { BACKGROUND } else { TEXT }))
        .hover(|style| style.bg(rgb(if selected { 0xa6c8ff } else { 0x45475a })))
        .on_click(move |_, _, cx| on_click(cx))
        .child(label)
}

pub(crate) fn logo_image() -> Arc<Image> {
    Arc::new(Image::from_bytes(
        gpui::ImageFormat::Png,
        include_bytes!("../assets/icons/neovim-gpui.png").to_vec(),
    ))
}

pub(crate) fn setting_section(title: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .w_full()
        .mt_4()
        .child(div().text_sm().text_color(rgb(ACCENT)).child(title))
        .child(div().w_full().mt_2().pl_3().child(content))
}

pub(crate) fn setting_row(
    label: &'static str,
    description: &'static str,
    controls: impl IntoElement,
) -> impl IntoElement {
    let mut controls_wrapper = div().min_w_0().mt_2().flex().flex_row();
    controls_wrapper.style().align_self = Some(AlignItems::Stretch);

    div()
        .w_full()
        .flex()
        .flex_col()
        .px_3()
        .py_3()
        .child(
            div().text_base().child(label).child(
                div()
                    .mt_1()
                    .text_sm()
                    .text_color(rgb(MUTED_TEXT))
                    .child(description),
            ),
        )
        .child(controls_wrapper.child(controls))
}

pub(crate) fn titlebar_button(
    label: &'static str,
    foreground: u32,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(label)
        .h(px(24.0))
        .px_2()
        .flex()
        .items_center()
        .rounded_sm()
        .text_sm()
        .text_color(rgb(foreground))
        .hover(|style| style.bg(rgb(SURFACE_BRIGHT)).text_color(rgb(foreground)))
        .on_click(move |_, _, cx| on_click(cx))
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::TextInputDisplay;

    #[test]
    fn text_input_display_keeps_long_value_cursor_context() {
        let value = "a/very/long/path/".repeat(8);
        let display = TextInputDisplay::new(&value, value.len());
        let text = display.text.as_ref();

        assert!(text.starts_with('…'));
        assert!(text.ends_with("path/"));
        assert_eq!(
            display.map_display_to_full(display.leading_ellipsis_len),
            display.actual_start
        );
        assert_eq!(display.map_display_to_full(text.len()), display.actual_end);
    }

    #[test]
    fn text_input_display_maps_selection_across_visible_boundaries() {
        let value = "prefix/".to_owned() + &"middle/".repeat(10) + "suffix";
        let cursor = value.len();
        let display = TextInputDisplay::new(&value, cursor);
        let selection = display.selection_display_range(0..value.len());

        assert_eq!(selection.start, 0);
        assert_eq!(
            selection.end,
            display.text.len() - display.trailing_ellipsis_len
        );
    }

    #[test]
    fn placeholder_display_maps_mouse_positions_to_the_empty_value() {
        let display = TextInputDisplay::new("Path not configured", "Path not configured".len())
            .map_to_empty();

        assert_eq!(display.map_display_to_full(0), 0);
        assert_eq!(display.map_display_to_full(display.text.len()), 0);
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn window_control_button(
    label: &'static str,
    area: WindowControlArea,
    background: u32,
    foreground: u32,
) -> impl IntoElement {
    div()
        .w(px(46.0))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .window_control_area(area)
        .child(label)
}
