use super::super::*;

pub(crate) struct DebugWindow {
    source: Entity<NvimGpui>,
    _source_subscription: Subscription,
}

impl DebugWindow {
    pub(crate) fn new(source: Entity<NvimGpui>, cx: &mut Context<Self>) -> Self {
        let source_subscription = cx.observe(&source, |_, _, cx| cx.notify());
        Self {
            source,
            _source_subscription: source_subscription,
        }
    }
}

impl Render for DebugWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.source.read(cx);
        let guifont = view
            .resolved_grid_font
            .as_ref()
            .map(|font| format!("{}:h{}", font.family, font.size))
            .or_else(|| view.guifont.clone())
            .unwrap_or_else(|| "system monospace (resolving)".to_owned());
        let guifontwide = view
            .resolved_grid_wide_font
            .as_ref()
            .map(|font| format!("{}:h{}", font.family, font.size))
            .or_else(|| view.guifontwide.clone())
            .unwrap_or_else(|| "same as guifont (fallback)".to_owned());
        let grid_size = view
            .grid_size
            .map(|(width, height)| format!("{width}×{height}"))
            .unwrap_or_else(|| "pending".to_owned());
        let ime_status = if view.system_ime.is_empty() {
            "IME: system".to_owned()
        } else {
            format!("IME composing: {}", view.system_ime.text())
        };
        let debug_row = |label: &'static str, value: String| {
            div()
                .w_full()
                .flex()
                .items_start()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(ACCENT))
                        .child(format!("{label}: ")),
                )
                .child(div().flex_1().whitespace_normal().child(value))
        };

        let debug_content = div()
            .flex_1()
            .flex()
            .flex_col()
            .justify_start()
            .overflow_hidden()
            .px_3()
            .py_2()
            .bg(rgb(SURFACE))
            .text_color(rgb(MUTED_TEXT))
            .border_b_1()
            .border_color(rgb(SURFACE_BRIGHT))
            .child(
                div()
                    .w_full()
                    .text_color(rgb(ACCENT))
                    .child("DEBUG  nvim-gpui"),
            )
            .child(debug_row("RPC", view.rpc_status.clone()))
            .child(debug_row("Grid", grid_size))
            .child(debug_row("guifont", guifont))
            .child(debug_row("guifontwide", guifontwide))
            .child(debug_row("File", view.state.file.to_owned()))
            .child(debug_row(
                "State",
                format!(
                    "{} {}:{}",
                    view.state.mode, view.state.line, view.state.column
                ),
            ))
            .child(debug_row("Input", ime_status))
            .child(debug_row(
                "API",
                view.api_level.unwrap_or_default().to_string(),
            ));
        let mut root = div().size_full().flex().flex_col().bg(rgb(SURFACE));
        if themed_titlebar_enabled() {
            root = root.child(themed_titlebar(
                "nvim-gpui debug".to_owned(),
                SURFACE,
                TEXT,
                None,
            ));
        }
        root.child(debug_content)
    }
}
