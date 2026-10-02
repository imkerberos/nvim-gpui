mod combo;
mod controls;
mod images;
mod layout;
mod text_input;
mod token_edit;

pub(crate) use combo::{combo_box, combo_option};
#[cfg(target_os = "macos")]
pub(crate) use controls::checkbox;
pub(crate) use controls::{option_button, titlebar_button};
pub(crate) use images::{logo_image, titlebar_logo_image};
pub(crate) use layout::{row, section};
pub(crate) use text_input::{text_input, TextInputConfig, TextInputMouseEvent, TextInputState};
pub(crate) use token_edit::{
    token_edit, TokenEditCandidate, TokenEditConfig, TokenEditEvent, TokenEditState,
};

pub(crate) const BACKGROUND: u32 = 0x1e1e2e;
pub(crate) const SURFACE: u32 = 0x181825;
pub(crate) const SURFACE_BRIGHT: u32 = 0x313244;
pub(crate) const TEXT: u32 = 0xcdd6f4;
pub(crate) const MUTED_TEXT: u32 = 0x7f849c;
pub(crate) const ACCENT: u32 = 0x89b4fa;
pub(crate) const IME_ACTIVE: u32 = 0xa6e3a1;
pub(crate) const WARNING: u32 = 0xf9e2af;

const MAX_VISIBLE_TEXT_INPUT_CHARS: usize = 48;
