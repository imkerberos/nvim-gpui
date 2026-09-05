//! Input routing boundaries for the editor surface.
//!
//! The router keeps platform IME handling separate from Neovim key handling.
//! Rime is an optional native backend, while the system IME remains the
//! fallback text backend exposed by GPUI's `EntityInputHandler`.

use gpui::{Keystroke, Modifiers, MouseButton, Pixels, Point, ScrollDelta};
use std::ops::Range;

// These values mirror librime's public key_table.h ABI. Keep the input
// module independent of the library crate because it is compiled by both the
// reusable library target and the nvim-gpui binary target.
const RIME_SHIFT_MASK: i32 = 1 << 0;
const RIME_CONTROL_MASK: i32 = 1 << 2;
const RIME_ALT_MASK: i32 = 1 << 3;
const RIME_SUPER_MASK: i32 = 1 << 26;
const RIME_RELEASE_MASK: i32 = 1 << 30;

pub use gpui::{EntityInputHandler, UTF16Selection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputContext {
    Normal,
    Insert,
    CommandLine,
    Prompt,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputTarget {
    Neovim,
    SystemIme,
    Rime,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputRouterConfig {
    pub rime_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputRouter {
    config: InputRouterConfig,
    context: InputContext,
}

impl Default for InputRouter {
    fn default() -> Self {
        Self::new(InputRouterConfig::default())
    }
}

impl InputRouter {
    pub fn new(config: InputRouterConfig) -> Self {
        Self {
            config,
            context: InputContext::Normal,
        }
    }

    pub fn context(&self) -> InputContext {
        self.context
    }

    pub fn set_context(&mut self, context: InputContext) {
        self.context = context;
    }

    pub fn config(&self) -> InputRouterConfig {
        self.config
    }

    pub fn set_config(&mut self, config: InputRouterConfig) {
        self.config = config;
    }

    pub fn set_nvim_mode(&mut self, mode: &str) {
        self.context = context_for_nvim_mode(mode);
    }

    pub fn target(&self) -> InputTarget {
        match self.context {
            InputContext::Normal => InputTarget::Neovim,
            InputContext::Insert
            | InputContext::CommandLine
            | InputContext::Prompt
            | InputContext::Terminal => {
                if self.config.rime_enabled {
                    InputTarget::Rime
                } else {
                    InputTarget::SystemIme
                }
            }
        }
    }
}

pub fn context_for_nvim_mode(mode: &str) -> InputContext {
    match mode.chars().next() {
        Some('i' | 'R' | 's' | 'S') => InputContext::Insert,
        Some('c') => InputContext::CommandLine,
        Some('r') => InputContext::Prompt,
        Some('t') => InputContext::Terminal,
        _ => InputContext::Normal,
    }
}

pub fn should_route_key_to_neovim(target: InputTarget, keystroke: &Keystroke) -> bool {
    match target {
        InputTarget::Neovim => true,
        InputTarget::SystemIme => {
            let is_control_key = keystroke.modifiers.control
                || keystroke.modifiers.alt
                || keystroke.modifiers.function
                || keystroke.modifiers.platform
                || matches!(
                    keystroke.key.as_str(),
                    "backspace"
                        | "delete"
                        | "enter"
                        | "escape"
                        | "tab"
                        | "left"
                        | "right"
                        | "up"
                        | "down"
                        | "pageup"
                        | "pagedown"
                        | "home"
                        | "end"
                        | "insert"
                );

            let is_printable_key = keystroke.is_ime_in_progress()
                || keystroke.key_char.is_some()
                || keystroke.key == "space";

            // A normal printable key may arrive with a key_char, while a
            // composing key may not have one yet. Both must be left to the
            // platform input handler; otherwise the character is committed
            // once by KeyDownEvent and a second time by the IME callback.
            is_control_key || !is_printable_key
        }
        InputTarget::Rime => false,
    }
}

/// Convert a GPUI key event into librime's platform-independent key event.
///
/// Rime must see the modifier mask for switcher hotkeys and schema bindings;
/// returning `None` is reserved for GPUI's physical `fn` modifier, which has
/// no librime equivalent. Whether a modified key is consumed remains a Rime
/// decision. An unconsumed event is forwarded to Neovim by the caller exactly
/// once.
pub fn rime_key_event(keystroke: &Keystroke) -> Option<(i32, i32)> {
    let modifiers = rime_modifier_mask(keystroke.modifiers)?;

    let keycode = match keystroke.key.as_str() {
        // librime consumes X11 keysyms for non-printable keys, even on
        // platforms that do not use X11 for their native window system.
        "backspace" => 0xff08,
        "delete" => 0xffff,
        "tab" => 0xff09,
        "enter" | "return" => 0xff0d,
        "escape" => 0xff1b,
        // Depending on the platform event path, GPUI may expose Space as the
        // named key or as the literal character.
        "space" | " " => 0x20,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "home" => 0xff50,
        "end" => 0xff57,
        "insert" => 0xff63,
        "shift" => 0xffe1,
        "control" => 0xffe3,
        "alt" => 0xffe9,
        "platform" => 0xffeb,
        key if key.len() > 1 && key.starts_with('f') => {
            let function_key = key[1..].parse::<i32>().ok()?;
            if !(1..=35).contains(&function_key) {
                return None;
            }
            0xffbe + function_key - 1
        }
        _ => {
            // `key` is the unshifted physical key in GPUI's normalized
            // keystroke. Prefer it over key_char so Shift+A becomes the
            // librime event `a + Shift`, not a different keycode `A`.
            let key = if keystroke.key.chars().count() == 1 {
                keystroke.key.as_str()
            } else {
                keystroke.key_char.as_deref()?
            };
            let character = key.chars().next()?;
            if !character.is_ascii() || !character.is_ascii_graphic() {
                return None;
            }
            character as i32
        }
    };

    Some((keycode, modifiers))
}

/// Convert GPUI's modifier state to librime's modifier mask.
pub fn rime_modifier_mask(modifiers: Modifiers) -> Option<i32> {
    if modifiers.function {
        // GPUI's physical Fn key is not represented by librime's key mask.
        return None;
    }

    let mut mask = 0;
    if modifiers.shift {
        mask |= RIME_SHIFT_MASK;
    }
    if modifiers.control {
        mask |= RIME_CONTROL_MASK;
    }
    if modifiers.alt {
        mask |= RIME_ALT_MASK;
    }
    if modifiers.platform {
        mask |= RIME_SUPER_MASK;
    }
    Some(mask)
}

/// Build the press/release event for a modifier-only transition.
///
/// GPUI reports a standalone modifier through `ModifiersChangedEvent`, not a
/// `KeyDownEvent`. The caller supplies the modifier state before a press or
/// after a release, matching the X11-style event that librime expects.
pub fn rime_modifier_event(key: &str, modifiers: Modifiers, release: bool) -> Option<(i32, i32)> {
    let keycode = match key {
        "shift" => 0xffe1,
        "control" => 0xffe3,
        "alt" => 0xffe9,
        "platform" => 0xffeb,
        _ => return None,
    };
    let mut mask = rime_modifier_mask(modifiers)?;
    if release {
        mask |= RIME_RELEASE_MASK;
    }
    Some((keycode, mask))
}

/// Convert one aggregate GPUI modifier transition into a librime event.
///
/// GPUI reports the new aggregate state, so remove the modifier represented by
/// `key` from the mask. This gives librime the state before that modifier's
/// press and the state after its release, including the release bit needed by
/// Rime's press-and-release ASCII mode switch.
pub fn rime_modifier_transition(
    key: &str,
    previous: Modifiers,
    current: Modifiers,
) -> Option<(i32, i32)> {
    let was_pressed = match key {
        "shift" => previous.shift,
        "control" => previous.control,
        "alt" => previous.alt,
        "platform" => previous.platform,
        _ => return None,
    };
    let is_pressed = match key {
        "shift" => current.shift,
        "control" => current.control,
        "alt" => current.alt,
        "platform" => current.platform,
        _ => unreachable!(),
    };
    if was_pressed == is_pressed {
        return None;
    }

    let modifiers = match key {
        "shift" => Modifiers {
            shift: false,
            ..current
        },
        "control" => Modifiers {
            control: false,
            ..current
        },
        "alt" => Modifiers {
            alt: false,
            ..current
        },
        "platform" => Modifiers {
            platform: false,
            ..current
        },
        _ => unreachable!(),
    };
    rime_modifier_event(key, modifiers, !is_pressed)
}

pub fn key_to_nvim_input(keystroke: &Keystroke) -> String {
    if !keystroke.modifiers.control
        && !keystroke.modifiers.alt
        && !keystroke.modifiers.platform
        && !keystroke.modifiers.function
    {
        if let Some(key_char) = keystroke.key_char.as_ref() {
            if !matches!(keystroke.key.as_str(), "enter" | "tab") {
                return key_char.clone();
            }
        }
        if keystroke.key.chars().count() == 1 {
            if keystroke.modifiers.shift {
                return keystroke.key.to_uppercase();
            }
            return keystroke.key.clone();
        }
    }

    let key = match keystroke.key.as_str() {
        "backspace" => "BS",
        "delete" => "Del",
        "enter" => "CR",
        "escape" => "Esc",
        "tab" => "Tab",
        "space" => "Space",
        "left" => "Left",
        "right" => "Right",
        "up" => "Up",
        "down" => "Down",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "home" => "Home",
        "end" => "End",
        "insert" => "Insert",
        key => key,
    };

    let mut notation = String::from("<");
    if keystroke.modifiers.control {
        notation.push_str("C-");
    }
    if keystroke.modifiers.alt {
        notation.push_str("M-");
    }
    if keystroke.modifiers.platform {
        notation.push_str("D-");
    }
    if keystroke.modifiers.shift && key.len() > 1 {
        notation.push_str("S-");
    }
    notation.push_str(key);
    notation.push('>');
    notation
}

/// Return the button name expected by `nvim_input_mouse()`.
pub fn nvim_mouse_button(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
        MouseButton::Navigate(gpui::NavigationDirection::Back) => "x1",
        MouseButton::Navigate(gpui::NavigationDirection::Forward) => "x2",
    }
}

/// Convert GPUI modifiers to Neovim's mouse modifier notation.
pub fn nvim_mouse_modifiers(modifiers: Modifiers) -> String {
    let mut result = String::new();
    if modifiers.control {
        result.push('C');
    }
    if modifiers.alt {
        result.push('A');
    }
    if modifiers.shift {
        result.push('S');
    }
    if modifiers.platform {
        result.push('D');
    }
    result
}

/// Express a platform scroll event in terminal-line units.
///
/// GPUI keeps the sign supplied by the platform backend: positive `y` is
/// wheel-up/content-up for the Linux and Windows backends, and AppKit's
/// native sign on macOS. In particular, do not invert this on macOS: that is
/// what preserves the user's natural-scrolling preference.
pub fn scroll_delta_to_lines(delta: ScrollDelta, line_height: Pixels) -> Point<f32> {
    let delta = delta.pixel_delta(line_height);
    Point {
        x: f32::from(delta.x) / f32::from(line_height),
        y: f32::from(delta.y) / f32::from(line_height),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemImeState {
    text: String,
    selected_range: Range<usize>,
    marked_range: Option<Range<usize>>,
}

impl Default for SystemImeState {
    fn default() -> Self {
        Self {
            text: String::new(),
            selected_range: 0..0,
            marked_range: None,
        }
    }
}

impl SystemImeState {
    pub fn text_for_range(&self, range_utf16: Range<usize>) -> (String, Range<usize>) {
        let range = self.range_from_utf16(&range_utf16);
        (
            self.text[range.clone()].to_owned(),
            self.range_to_utf16(&range),
        )
    }

    pub fn selected_text_range(&self) -> UTF16Selection {
        UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: false,
        }
    }

    pub fn selected_range_utf8(&self) -> Range<usize> {
        self.selected_range.clone()
    }

    pub fn marked_text_range(&self) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    /// Return the marked range in the representation used by Rust strings.
    ///
    /// GPUI's input-handler API exposes UTF-16 ranges to the platform, while
    /// the temporary inline-composition renderer works with string byte
    /// offsets. Keep both views explicit so the renderer does not have to
    /// reverse the conversion.
    pub fn marked_range_utf8(&self) -> Option<Range<usize>> {
        self.marked_range.clone()
    }

    pub fn replace_text(&mut self, range_utf16: Option<Range<usize>>, new_text: &str) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        self.text.replace_range(range.clone(), new_text);
        let cursor = range.start + new_text.len();
        self.selected_range = cursor..cursor;
        self.marked_range = None;
    }

    pub fn replace_and_mark_text(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        self.text.replace_range(range.clone(), new_text);

        if new_text.is_empty() {
            self.marked_range = None;
        } else {
            self.marked_range = Some(range.start..range.start + new_text.len());
        }

        self.selected_range = new_selected_range_utf16
            .map(|selected| {
                let selected = utf16_range_to_utf8(new_text, selected);
                range.start + selected.start..range.start + selected.end
            })
            .unwrap_or_else(|| {
                let cursor = range.start + new_text.len();
                cursor..cursor
            });
    }

    pub fn unmark(&mut self) {
        self.marked_range = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.selected_range = 0..0;
        self.marked_range = None;
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf16_to_utf8_offset(&self.text, range.start)..utf16_to_utf8_offset(&self.text, range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf8_to_utf16_offset(&self.text, range.start)..utf8_to_utf16_offset(&self.text, range.end)
    }
}

fn utf16_range_to_utf8(text: &str, range: Range<usize>) -> Range<usize> {
    utf16_to_utf8_offset(text, range.start)..utf16_to_utf8_offset(text, range.end)
}

pub fn utf8_to_utf16_offset(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset.min(text.len())]
        .chars()
        .map(char::len_utf16)
        .sum()
}

fn utf16_to_utf8_offset(text: &str, utf16_offset: usize) -> usize {
    let mut current_utf16 = 0;
    for (byte_offset, character) in text.char_indices() {
        if current_utf16 >= utf16_offset {
            return byte_offset;
        }
        current_utf16 += character.len_utf16();
        if current_utf16 > utf16_offset {
            return byte_offset;
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::{
        context_for_nvim_mode, key_to_nvim_input, nvim_mouse_button, nvim_mouse_modifiers,
        rime_key_event, rime_modifier_transition, scroll_delta_to_lines,
        should_route_key_to_neovim, InputContext, InputRouter, InputRouterConfig, InputTarget,
        SystemImeState, RIME_ALT_MASK, RIME_CONTROL_MASK, RIME_RELEASE_MASK, RIME_SHIFT_MASK,
        RIME_SUPER_MASK,
    };
    use gpui::{point, px, Keystroke, Modifiers, MouseButton, ScrollDelta};

    #[test]
    fn normal_mode_always_routes_keys_to_neovim() {
        let router = InputRouter::new(InputRouterConfig { rime_enabled: true });

        assert_eq!(router.target(), InputTarget::Neovim);
    }

    #[test]
    fn insert_mode_uses_system_ime_when_rime_is_disabled() {
        let mut router = InputRouter::default();
        router.set_context(InputContext::Insert);

        assert_eq!(router.target(), InputTarget::SystemIme);
    }

    #[test]
    fn insert_mode_uses_rime_when_enabled() {
        let mut router = InputRouter::new(InputRouterConfig { rime_enabled: true });
        router.set_context(InputContext::Insert);

        assert_eq!(router.target(), InputTarget::Rime);
    }

    #[test]
    fn text_input_contexts_use_rime_when_enabled() {
        let mut router = InputRouter::new(InputRouterConfig { rime_enabled: true });
        router.set_context(InputContext::CommandLine);
        assert_eq!(router.target(), InputTarget::Rime);

        for context in [InputContext::Prompt, InputContext::Terminal] {
            router.set_context(context);
            assert_eq!(router.target(), InputTarget::Rime);
        }
    }

    #[test]
    fn nvim_modes_select_the_expected_system_input_context() {
        assert_eq!(context_for_nvim_mode("n"), InputContext::Normal);
        assert_eq!(context_for_nvim_mode("i"), InputContext::Insert);
        assert_eq!(context_for_nvim_mode("c"), InputContext::CommandLine);
        assert_eq!(context_for_nvim_mode("t"), InputContext::Terminal);
    }

    #[test]
    fn system_ime_state_round_trips_utf16_ranges() {
        let mut state = SystemImeState::default();
        state.replace_and_mark_text(None, "你a😀", Some(1..2));

        let (text, actual_range) = state.text_for_range(0..4);

        assert_eq!(text, "你a😀");
        assert_eq!(actual_range, 0..4);
        assert_eq!(state.marked_text_range(), Some(0..4));
        assert_eq!(state.marked_range_utf8(), Some(0..8));
        assert_eq!(state.selected_range_utf8(), 3..4);
        assert_eq!(state.selected_text_range().range, 1..2);
    }

    #[test]
    fn nvim_key_notation_keeps_text_and_encodes_control_keys() {
        let text = Keystroke::parse("a").expect("text key should parse");
        assert_eq!(key_to_nvim_input(&text), "a");

        let shifted = Keystroke::parse("shift-a").expect("shifted text key should parse");
        assert_eq!(key_to_nvim_input(&shifted), "A");

        let escape = Keystroke::parse("escape").expect("escape should parse");
        assert_eq!(key_to_nvim_input(&escape), "<Esc>");

        let control = Keystroke::parse("ctrl-w").expect("control key should parse");
        assert_eq!(key_to_nvim_input(&control), "<C-w>");
    }

    #[test]
    fn rime_key_event_accepts_text_and_plain_editing_keys() {
        assert_eq!(
            rime_key_event(&Keystroke::parse("a").expect("text key should parse")),
            Some((b'a' as i32, 0))
        );
        assert_eq!(
            rime_key_event(&Keystroke::parse("shift-a").expect("shifted key should parse")),
            Some((b'a' as i32, RIME_SHIFT_MASK))
        );
        assert_eq!(
            rime_key_event(&Keystroke::parse("space").expect("space should parse")),
            Some((b' ' as i32, 0))
        );
        assert_eq!(
            rime_key_event(&Keystroke {
                key: " ".to_owned(),
                key_char: Some(" ".to_owned()),
                modifiers: Modifiers::none(),
            }),
            Some((b' ' as i32, 0))
        );
        assert_eq!(
            rime_key_event(&Keystroke::parse("backspace").expect("backspace should parse")),
            Some((0xff08, 0))
        );
        assert_eq!(
            rime_key_event(&Keystroke::parse("down").expect("down should parse")),
            Some((0xff54, 0))
        );
        assert_eq!(
            rime_key_event(&Keystroke::parse("pageup").expect("pageup should parse")),
            Some((0xff55, 0))
        );
        assert_eq!(
            rime_key_event(&Keystroke::parse("ctrl-a").expect("control key should parse")),
            Some((b'a' as i32, RIME_CONTROL_MASK))
        );
        assert_eq!(
            rime_key_event(&Keystroke::parse("ctrl-shift-`").expect("switcher key should parse")),
            Some((b'`' as i32, RIME_CONTROL_MASK | RIME_SHIFT_MASK))
        );
        assert_eq!(
            rime_key_event(&Keystroke::parse("alt-f12").expect("function key should parse")),
            Some((0xffc9, RIME_ALT_MASK))
        );
        assert_eq!(
            rime_key_event(&Keystroke::parse("cmd-\\").expect("platform key should parse")),
            Some((b'\\' as i32, RIME_SUPER_MASK))
        );
    }

    #[test]
    fn standalone_modifier_transitions_are_expressed_for_rime() {
        let none = Modifiers::none();
        let shift = Modifiers::shift();
        assert_eq!(
            rime_modifier_transition("shift", none, shift),
            Some((0xffe1, 0))
        );
        assert_eq!(
            rime_modifier_transition("shift", shift, none),
            Some((0xffe1, RIME_RELEASE_MASK))
        );

        let control = Modifiers::control();
        assert_eq!(
            rime_modifier_transition(
                "shift",
                control,
                Modifiers {
                    shift: true,
                    ..control
                }
            ),
            Some((0xffe1, RIME_CONTROL_MASK))
        );
    }

    #[test]
    fn nvim_mouse_input_uses_button_and_modifier_notation() {
        assert_eq!(nvim_mouse_button(MouseButton::Left), "left");
        assert_eq!(
            nvim_mouse_button(MouseButton::Navigate(gpui::NavigationDirection::Forward)),
            "x2"
        );
        assert_eq!(
            nvim_mouse_modifiers(Modifiers {
                control: true,
                alt: true,
                shift: true,
                platform: true,
                function: false,
            }),
            "CASD"
        );
    }

    #[test]
    fn scroll_delta_is_converted_without_changing_its_native_sign() {
        assert_eq!(
            scroll_delta_to_lines(ScrollDelta::Lines(point(2.0, -3.0)), px(20.0)),
            point(2.0, -3.0)
        );
        assert_eq!(
            scroll_delta_to_lines(ScrollDelta::Pixels(point(px(10.0), px(-40.0))), px(20.0)),
            point(0.5, -2.0)
        );
    }

    #[test]
    fn system_ime_owns_printable_keys_but_not_control_keys() {
        let text = Keystroke::parse("a").expect("text key should parse");
        let committed_text = Keystroke::parse("a->a").expect("committed text key should parse");
        let space = Keystroke::parse("space").expect("space should parse");
        let escape = Keystroke::parse("escape").expect("escape should parse");
        let enter = Keystroke::parse("enter").expect("enter should parse");
        let alt_x = Keystroke::parse("alt-x").expect("Alt shortcut should parse");

        assert!(!should_route_key_to_neovim(InputTarget::SystemIme, &text));
        assert!(!should_route_key_to_neovim(
            InputTarget::SystemIme,
            &committed_text
        ));
        assert!(!should_route_key_to_neovim(InputTarget::SystemIme, &space));
        assert!(should_route_key_to_neovim(InputTarget::SystemIme, &escape));
        assert!(should_route_key_to_neovim(InputTarget::SystemIme, &enter));
        assert!(should_route_key_to_neovim(InputTarget::SystemIme, &alt_x));
    }
}
