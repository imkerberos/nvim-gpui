//! Compatibility adapters for versioned Neovim UI protocol payloads.
//!
//! Neovim keeps the UI event names stable, but some multigrid events gained
//! parameters in later releases. The rest of the client should consume one
//! normalized event shape, so those wire-format differences belong here.

use rmpv::Value;

use super::protocol::{
    bool_value, parse_i64_value, parse_u64_value, parse_window_id, string_value,
};
use super::{NvimEvent, NvimFloatAnchor, NvimFloatPosition, NvimProtocolInfo, NvimVersion};

const MESSAGE_GRID_ZINDEX: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NvimUiProtocol {
    Legacy,
    Extended,
    Unknown,
}

/// Normalizes version-specific Neovim UI protocol payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NvimProtocolAdapter {
    ui_protocol: NvimUiProtocol,
}

impl NvimProtocolAdapter {
    pub(super) fn from_protocol(protocol: &NvimProtocolInfo) -> Self {
        Self::from_version(protocol.version)
    }

    pub(super) fn unknown() -> Self {
        Self {
            ui_protocol: NvimUiProtocol::Unknown,
        }
    }

    fn from_version(version: NvimVersion) -> Self {
        let ui_protocol = if version.major > 0 || version.minor >= 12 {
            NvimUiProtocol::Extended
        } else {
            NvimUiProtocol::Legacy
        };
        Self { ui_protocol }
    }

    pub(super) fn parse_win_float_pos(&self, args: &[Value]) -> Result<NvimEvent, String> {
        let legacy = match self.ui_protocol {
            NvimUiProtocol::Legacy => {
                expect_arg_count(args, 8, 8, "win_float_pos")?;
                true
            }
            NvimUiProtocol::Extended => {
                expect_arg_count(args, 11, usize::MAX, "win_float_pos")?;
                false
            }
            NvimUiProtocol::Unknown => match args.len() {
                8 => true,
                count if count >= 11 => false,
                count => {
                    return Err(format!(
                        "redraw event win_float_pos expects 8 or at least 11 arguments, got {count}"
                    ));
                }
            },
        };

        let grid = parse_u64_value(&args[0], "win_float_pos grid")?;
        let win = parse_window_id(&args[1])?;
        let anchor_grid = parse_u64_value(&args[3], "win_float_pos anchor grid")?;
        let anchor_row = parse_i64_value(&args[4], "win_float_pos anchor row")?;
        let anchor_col = parse_i64_value(&args[5], "win_float_pos anchor column")?;
        let mouse_enabled = bool_value(&args[6])
            .ok_or_else(|| "win_float_pos has an invalid mouse flag".to_owned())?;
        let zindex = parse_i64_value(&args[7], "win_float_pos z-index")?;

        let (position, compindex) = if legacy {
            let anchor = parse_float_anchor(&args[2])?;
            (
                NvimFloatPosition::Anchored {
                    anchor,
                    anchor_grid,
                    row: anchor_row,
                    col: anchor_col,
                },
                -1,
            )
        } else {
            (
                NvimFloatPosition::Screen {
                    row: parse_i64_value(&args[9], "win_float_pos screen row")?,
                    col: parse_i64_value(&args[10], "win_float_pos screen column")?,
                },
                parse_i64_value(&args[8], "win_float_pos composition index")?,
            )
        };

        Ok(NvimEvent::WinFloatPos {
            grid,
            win,
            position,
            mouse_enabled,
            zindex,
            compindex,
        })
    }

    pub(super) fn parse_msg_set_pos(&self, args: &[Value]) -> Result<NvimEvent, String> {
        let legacy = match self.ui_protocol {
            NvimUiProtocol::Legacy => {
                expect_arg_count(args, 4, 4, "msg_set_pos")?;
                true
            }
            NvimUiProtocol::Extended => {
                expect_arg_count(args, 6, usize::MAX, "msg_set_pos")?;
                false
            }
            NvimUiProtocol::Unknown => match args.len() {
                4 => true,
                count if count >= 6 => false,
                count => {
                    return Err(format!(
                        "redraw event msg_set_pos expects 4 or at least 6 arguments, got {count}"
                    ));
                }
            },
        };

        Ok(NvimEvent::MsgSetPos {
            grid: parse_u64_value(&args[0], "msg_set_pos grid")?,
            row: parse_u64_value(&args[1], "msg_set_pos row")?,
            scrolled: bool_value(&args[2])
                .ok_or_else(|| "msg_set_pos has an invalid scrolled flag".to_owned())?,
            sep_char: string_value(&args[3])
                .ok_or_else(|| "msg_set_pos has an invalid separator character".to_owned())?,
            zindex: if legacy {
                MESSAGE_GRID_ZINDEX
            } else {
                parse_i64_value(&args[4], "msg_set_pos z-index")?
            },
            // Legacy Neovim does not expose its internal compositor index.
            // The application falls back to z-index ordering for this value.
            compindex: if legacy {
                -1
            } else {
                parse_i64_value(&args[5], "msg_set_pos composition index")?
            },
        })
    }
}

fn parse_float_anchor(value: &Value) -> Result<NvimFloatAnchor, String> {
    match string_value(value).as_deref() {
        Some("NW") => Ok(NvimFloatAnchor::NorthWest),
        Some("NE") => Ok(NvimFloatAnchor::NorthEast),
        Some("SW") => Ok(NvimFloatAnchor::SouthWest),
        Some("SE") => Ok(NvimFloatAnchor::SouthEast),
        Some(anchor) => Err(format!("win_float_pos has an invalid anchor {anchor:?}")),
        None => Err("win_float_pos anchor is not a string".to_owned()),
    }
}

fn expect_arg_count(
    args: &[Value],
    minimum: usize,
    maximum: usize,
    event: &str,
) -> Result<(), String> {
    if (minimum..=maximum).contains(&args.len()) {
        return Ok(());
    }
    if minimum == maximum {
        return Err(format!(
            "redraw event {event} expects {minimum} arguments, got {}",
            args.len()
        ));
    }
    Err(format!(
        "redraw event {event} expects at least {minimum} arguments, got {}",
        args.len()
    ))
}
