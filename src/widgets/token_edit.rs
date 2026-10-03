use gpui::{
    deferred, div, prelude::*, px, rgb, App, ElementId, FocusHandle, IntoElement, KeyDownEvent,
    ScrollHandle, SharedString, Window,
};
use std::rc::Rc;

use super::text_input::{inline_text_input, TextInputMouseEvent, TextInputState};
use super::{ACCENT, MUTED_TEXT, SURFACE, SURFACE_BRIGHT, TEXT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TokenEditEvent {
    Surface,
    Token(usize),
    Candidate(usize),
    Input(TextInputMouseEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TokenEditCandidate {
    pub(crate) label: SharedString,
    pub(crate) selected: bool,
    pub(crate) highlighted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TokenEditState {
    pub(crate) cursor: usize,
    pub(crate) list_open: bool,
    pub(crate) input: TextInputState,
    pub(crate) highlighted_candidate: usize,
}

type TokenEditEventHandler = Box<dyn Fn(&TokenEditEvent, &mut Window, &mut App)>;
type TokenEditKeyHandler = Box<dyn Fn(&KeyDownEvent, &mut Window, &mut App)>;
type SharedTokenEditEventHandler = Rc<dyn Fn(&TokenEditEvent, &mut Window, &mut App)>;

pub(crate) struct TokenEditConfig {
    id: ElementId,
    focus_handle: FocusHandle,
    tokens: Vec<SharedString>,
    placeholder: SharedString,
    candidates: Option<Vec<TokenEditCandidate>>,
    empty_candidates_label: SharedString,
    loading_label: SharedString,
    state: Option<TokenEditState>,
    scroll_handle: Option<ScrollHandle>,
    on_event: TokenEditEventHandler,
    on_key_down: TokenEditKeyHandler,
}

impl TokenEditConfig {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        focus_handle: FocusHandle,
        tokens: Vec<SharedString>,
        placeholder: impl Into<SharedString>,
        state: Option<TokenEditState>,
    ) -> Self {
        Self {
            id: id.into(),
            focus_handle,
            tokens,
            placeholder: placeholder.into(),
            candidates: None,
            empty_candidates_label: "No additional options available".into(),
            loading_label: "Loading options…".into(),
            state,
            scroll_handle: None,
            on_event: Box::new(|_, _, _| {}),
            on_key_down: Box::new(|_, _, _| {}),
        }
    }

    pub(crate) fn candidates(mut self, candidates: Option<Vec<TokenEditCandidate>>) -> Self {
        self.candidates = candidates;
        self
    }

    pub(crate) fn scroll_handle(mut self, scroll_handle: ScrollHandle) -> Self {
        self.scroll_handle = Some(scroll_handle);
        self
    }

    pub(crate) fn empty_candidates_label(mut self, label: impl Into<SharedString>) -> Self {
        self.empty_candidates_label = label.into();
        self
    }

    pub(crate) fn loading_label(mut self, label: impl Into<SharedString>) -> Self {
        self.loading_label = label.into();
        self
    }

    pub(crate) fn on_event(
        mut self,
        handler: impl Fn(&TokenEditEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_event = Box::new(handler);
        self
    }

    pub(crate) fn on_key_down(
        mut self,
        handler: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_key_down = Box::new(handler);
        self
    }
}

pub(crate) fn token_edit(config: TokenEditConfig) -> impl IntoElement {
    let TokenEditConfig {
        id,
        focus_handle,
        tokens,
        placeholder,
        candidates,
        empty_candidates_label,
        loading_label,
        state,
        scroll_handle,
        on_event,
        on_key_down,
    } = config;
    let editing = state.is_some();
    let cursor = state
        .as_ref()
        .map(|state| state.cursor)
        .unwrap_or(tokens.len());
    let list_open = state.as_ref().is_some_and(|state| state.list_open);
    let input = state.as_ref().map(|state| &state.input);
    let on_event: SharedTokenEditEventHandler = Rc::from(on_event);

    let mut token_field = div()
        .id(id.clone())
        .w_full()
        .min_w_0()
        .h(px(36.0))
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(rgb(if editing { ACCENT } else { SURFACE_BRIGHT }))
        .bg(rgb(SURFACE))
        .track_focus(&focus_handle)
        .focus(|style| style.border_color(rgb(ACCENT)))
        .hover(|style| style.border_color(rgb(ACCENT)))
        .cursor_pointer()
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation());

    let surface_on_event = on_event.clone();
    token_field = token_field
        .on_click(move |_, window, cx| {
            surface_on_event(&TokenEditEvent::Surface, window, cx);
        })
        .capture_key_down(on_key_down);

    for (index, token) in tokens.iter().cloned().enumerate() {
        if editing && cursor == index {
            if let Some(input) = input {
                token_field = token_field.child(token_edit_input(
                    input,
                    focus_handle.clone(),
                    on_event.clone(),
                ));
            }
        }

        let token_on_event = on_event.clone();
        token_field = token_field.child(
            div()
                .id(ElementId::NamedChild(
                    Box::new(id.clone()),
                    format!("token-{index}").into(),
                ))
                .max_w(px(240.0))
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(rgb(SURFACE_BRIGHT))
                .text_sm()
                .text_color(rgb(TEXT))
                .cursor_pointer()
                .child(token)
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    let event = TokenEditEvent::Token(index);
                    token_on_event(&event, window, cx);
                }),
        );
    }

    if editing && cursor == tokens.len() {
        if let Some(input) = input {
            token_field = token_field.child(token_edit_input(
                input,
                focus_handle.clone(),
                on_event.clone(),
            ));
        }
    }
    if tokens.is_empty() && input.is_none_or(|input| input.value.is_empty()) {
        token_field = token_field.child(
            div()
                .flex_1()
                .text_sm()
                .text_color(rgb(MUTED_TEXT))
                .child(placeholder),
        );
    }

    let mut editor = div()
        .relative()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .child(token_field);

    if list_open {
        let mut options = div()
            .id(ElementId::NamedChild(
                Box::new(id.clone()),
                "options".into(),
            ))
            .w_full()
            .flex()
            .flex_col()
            .max_h(px(320.0))
            .overflow_y_scroll()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation());
        match candidates {
            Some(candidates) => {
                let candidates_are_empty = candidates.is_empty();
                for (index, candidate) in candidates.into_iter().enumerate() {
                    let candidate_on_event = on_event.clone();
                    options = options.child(token_edit_candidate(
                        ElementId::NamedChild(
                            Box::new(id.clone()),
                            format!("candidate-{index}").into(),
                        ),
                        candidate,
                        move |_, window, cx| {
                            cx.stop_propagation();
                            let event = TokenEditEvent::Candidate(index);
                            candidate_on_event(&event, window, cx);
                        },
                    ));
                }
                if candidates_are_empty {
                    options = options.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_sm()
                            .text_color(rgb(MUTED_TEXT))
                            .child(empty_candidates_label),
                    );
                }
            }
            None => {
                options = options.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(rgb(MUTED_TEXT))
                        .child(loading_label),
                );
            }
        }

        if let Some(scroll_handle) = scroll_handle.as_ref() {
            options = options.track_scroll(scroll_handle);
        }
        editor = editor.child(
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
                    .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                    .child(options),
            )
            .with_priority(1),
        );
    }

    editor
}

fn token_edit_input(
    input: &TextInputState,
    focus_handle: FocusHandle,
    on_event: SharedTokenEditEventHandler,
) -> impl IntoElement {
    inline_text_input(input.clone(), focus_handle, move |event, window, cx| {
        on_event(&TokenEditEvent::Input(event), window, cx);
    })
}

fn token_edit_candidate(
    id: impl Into<ElementId>,
    candidate: TokenEditCandidate,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let highlighted = candidate.highlighted;
    let selected = candidate.selected;
    div()
        .id(id)
        .w_full()
        .flex()
        .items_center()
        .px_3()
        .py_2()
        .rounded_sm()
        .text_sm()
        .text_color(rgb(if highlighted { super::BACKGROUND } else { TEXT }))
        .bg(rgb(if highlighted {
            ACCENT
        } else if selected {
            SURFACE_BRIGHT
        } else {
            SURFACE
        }))
        .cursor_pointer()
        .hover(|style| {
            style.bg(rgb(if highlighted {
                0xa6c8ff
            } else {
                SURFACE_BRIGHT
            }))
        })
        .on_click(on_click)
        .child(div().flex_1().child(candidate.label))
        .child(
            div()
                .w(px(20.0))
                .text_right()
                .text_color(rgb(if highlighted {
                    super::BACKGROUND
                } else {
                    ACCENT
                }))
                .child(if selected { "✓" } else { "" }),
        )
}
