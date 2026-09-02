//! Kitty graphics protocol state and parsing.
//!
//! Neovim exposes terminal output sent with `nvim_ui_send` as a redraw event
//! when the UI advertises `stdout_tty`. This module keeps that byte stream
//! independent from GPUI: it decodes Kitty APC frames, stores image bytes and
//! remembers both normal and Unicode-placeholder placements. The GPUI layer
//! decides how to turn the stored image into a renderable image.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{collections::HashMap, fs, io::Cursor, path::Path};

const MAX_IMAGE_BYTES: usize = 256 * 1024 * 1024;
const MAX_IMAGE_CACHE_BYTES: usize = 128 * 1024 * 1024;
const KITTY_PLACEHOLDER: char = '\u{10eeee}';

/// An identifier assigned by the Kitty graphics protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageId(pub u32);

/// A Neovim grid identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlacementKey {
    pub image: ImageId,
    pub placement: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridAnchor {
    pub grid: GridId,
    pub row: u32,
    pub column: u32,
}

/// Formats accepted by GPUI's image decoder after Kitty transfer decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormatKind {
    Png,
    Jpeg,
    Webp,
    Gif,
    Bmp,
    Tiff,
}

impl ImageFormatKind {
    fn detect(bytes: &[u8]) -> Self {
        match image::guess_format(bytes).ok() {
            Some(image::ImageFormat::Jpeg) => Self::Jpeg,
            Some(image::ImageFormat::WebP) => Self::Webp,
            Some(image::ImageFormat::Gif) => Self::Gif,
            Some(image::ImageFormat::Bmp) => Self::Bmp,
            Some(image::ImageFormat::Tiff) => Self::Tiff,
            _ => Self::Png,
        }
    }

    /// Map to the equivalent GPUI image format.
    pub fn gpui_format(self) -> gpui::ImageFormat {
        match self {
            Self::Png => gpui::ImageFormat::Png,
            Self::Jpeg => gpui::ImageFormat::Jpeg,
            Self::Webp => gpui::ImageFormat::Webp,
            Self::Gif => gpui::ImageFormat::Gif,
            Self::Bmp => gpui::ImageFormat::Bmp,
            Self::Tiff => gpui::ImageFormat::Tiff,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageAsset {
    pub encoded: Vec<u8>,
    pub format: ImageFormatKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlacement {
    pub key: PlacementKey,
    pub anchor: GridAnchor,
    pub columns: u32,
    pub rows: u32,
    pub z_index: i32,
    /// `U=1` placements are located by placeholder cells, not by the Kitty
    /// cursor. Their anchor is `GridId(0)` until the grid is scanned.
    pub virtual_placeholder: bool,
}

impl ImagePlacement {
    pub fn is_virtual_placeholder(&self) -> bool {
        self.virtual_placeholder
    }
}

/// Changes produced while consuming a `ui_send` byte stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KittyEvent {
    AssetUpdated {
        image: ImageId,
        format: ImageFormatKind,
    },
    AssetDeleted {
        image: ImageId,
    },
    AssetsCleared,
    /// A terminal response that must be passed back through
    /// `nvim_ui_term_event("termresponse", ...)`.
    TerminalResponse(String),
}

#[derive(Debug)]
pub struct ImageStore {
    assets: HashMap<ImageId, ImageAsset>,
    placements: HashMap<PlacementKey, ImagePlacement>,
    asset_last_used: HashMap<ImageId, u64>,
    asset_bytes: usize,
    next_asset_use: u64,
    max_asset_bytes: usize,
    parser: KittyGraphicsParser,
}

impl Default for ImageStore {
    fn default() -> Self {
        Self {
            assets: HashMap::new(),
            placements: HashMap::new(),
            asset_last_used: HashMap::new(),
            asset_bytes: 0,
            next_asset_use: 0,
            max_asset_bytes: MAX_IMAGE_CACHE_BYTES,
            parser: KittyGraphicsParser::default(),
        }
    }
}

impl ImageStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_cache_size_mb(&mut self, megabytes: u32) -> Vec<ImageId> {
        let bytes = usize::try_from(megabytes)
            .unwrap_or(128)
            .saturating_mul(1024 * 1024);
        self.max_asset_bytes = bytes.max(16 * 1024 * 1024);
        self.prune_unplaced_assets(None)
    }

    pub fn insert_asset(&mut self, id: ImageId, encoded: Vec<u8>) {
        let format = ImageFormatKind::detect(&encoded);
        let _ = self.insert_asset_with_format(id, encoded, format);
    }

    pub fn insert_asset_with_format(
        &mut self,
        id: ImageId,
        encoded: Vec<u8>,
        format: ImageFormatKind,
    ) -> Vec<ImageId> {
        if let Some(previous) = self.assets.insert(id, ImageAsset { encoded, format }) {
            self.asset_bytes = self.asset_bytes.saturating_sub(previous.encoded.len());
        }
        self.asset_bytes = self
            .asset_bytes
            .saturating_add(self.assets.get(&id).map_or(0, |asset| asset.encoded.len()));
        self.touch_asset(id);
        self.prune_unplaced_assets(Some(id))
    }

    pub fn asset(&self, id: ImageId) -> Option<&ImageAsset> {
        self.assets.get(&id)
    }

    pub fn place(&mut self, placement: ImagePlacement) {
        self.touch_asset(placement.key.image);
        self.placements.insert(placement.key, placement);
    }

    pub fn placement(&self, key: PlacementKey) -> Option<&ImagePlacement> {
        self.placements.get(&key)
    }

    pub fn placements(&self) -> impl Iterator<Item = &ImagePlacement> {
        self.placements.values()
    }

    pub fn virtual_placements(&self) -> impl Iterator<Item = &ImagePlacement> {
        self.placements
            .values()
            .filter(|placement| placement.is_virtual_placeholder())
    }

    pub fn has_virtual_placements(&self) -> bool {
        self.placements
            .values()
            .any(ImagePlacement::is_virtual_placeholder)
    }

    pub fn remove_placement(&mut self, key: PlacementKey) -> Option<ImagePlacement> {
        self.placements.remove(&key)
    }

    pub fn clear(&mut self) {
        self.assets.clear();
        self.placements.clear();
        self.asset_last_used.clear();
        self.asset_bytes = 0;
        self.next_asset_use = 0;
        self.parser = KittyGraphicsParser::default();
    }

    fn touch_asset(&mut self, id: ImageId) {
        self.next_asset_use = self.next_asset_use.saturating_add(1);
        self.asset_last_used.insert(id, self.next_asset_use);
    }

    fn has_placements(&self, image: ImageId) -> bool {
        self.placements
            .keys()
            .any(|placement| placement.image == image)
    }

    fn remove_asset(&mut self, image: ImageId) -> bool {
        let Some(asset) = self.assets.remove(&image) else {
            return false;
        };
        self.asset_bytes = self.asset_bytes.saturating_sub(asset.encoded.len());
        self.asset_last_used.remove(&image);
        true
    }

    fn clear_assets_and_placements(&mut self) {
        self.assets.clear();
        self.placements.clear();
        self.asset_last_used.clear();
        self.asset_bytes = 0;
        self.next_asset_use = 0;
    }

    /// Evict the least recently used images that no longer have a placement.
    /// `protected` is used for a just-transmitted image so an oversized upload
    /// is not immediately removed before its AssetUpdated event is delivered.
    fn prune_unplaced_assets(&mut self, protected: Option<ImageId>) -> Vec<ImageId> {
        let mut evicted = Vec::new();
        while self.asset_bytes > self.max_asset_bytes {
            let candidate = self
                .assets
                .keys()
                .copied()
                .filter(|image| Some(*image) != protected && !self.has_placements(*image))
                .min_by_key(|image| self.asset_last_used.get(image).copied().unwrap_or(0));
            let Some(image) = candidate else {
                break;
            };
            if self.remove_asset(image) {
                evicted.push(image);
            }
        }
        evicted
    }

    fn prune_after_delete(&mut self, events: &mut Vec<KittyEvent>) {
        for image in self.prune_unplaced_assets(None) {
            events.push(KittyEvent::AssetDeleted { image });
        }
    }

    /// Consume arbitrary chunks of terminal data sent by Neovim.
    pub fn consume_ui_data(&mut self, data: &str, grid: GridId) -> Vec<KittyEvent> {
        let mut parser = std::mem::take(&mut self.parser);
        let events = parser.consume(data.as_bytes(), grid, self);
        self.parser = parser;
        events
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct KittyCursor {
    row: u32,
    column: u32,
}

#[derive(Debug)]
struct Transfer {
    image: ImageId,
    format: u32,
    transmission: String,
    width: Option<u32>,
    height: Option<u32>,
    encoded_payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct KittyGraphicsParser {
    stream: Vec<u8>,
    cursor: KittyCursor,
    transfer: Option<Transfer>,
}

impl KittyGraphicsParser {
    fn consume(&mut self, data: &[u8], grid: GridId, store: &mut ImageStore) -> Vec<KittyEvent> {
        self.stream.extend_from_slice(data);
        let mut events = Vec::new();

        loop {
            if self.stream.starts_with(b"\x1b_G") {
                let Some(end) = find_st_string_end(&self.stream, 3) else {
                    break;
                };
                let frame = self.stream[3..end].to_vec();
                self.stream.drain(..end + 2);
                self.handle_graphics_frame(&frame, grid, store, &mut events);
                continue;
            }

            // Snacks wraps terminal data in a tmux DCS when it detects TMUX.
            // Accept it even when the host process itself is not a terminal.
            if self.stream.starts_with(b"\x1bPtmux;") {
                let start = b"\x1bPtmux;".len();
                let Some(end) = find_tmux_st_string_end(&self.stream, start) else {
                    break;
                };
                let inner = unescape_tmux(&self.stream[start..end]);
                self.stream.drain(..end + 2);
                self.stream.splice(0..0, inner);
                continue;
            }

            if self.stream.starts_with(b"\x1b[") {
                let Some(final_index) = self.stream[2..]
                    .iter()
                    .position(|byte| (0x40..=0x7e).contains(byte))
                    .map(|index| index + 2)
                else {
                    break;
                };
                let sequence = self.stream[2..final_index].to_vec();
                let final_byte = self.stream[final_index];
                self.stream.drain(..final_index + 1);
                self.handle_csi(&sequence, final_byte, &mut events);
                continue;
            }

            if self.stream.first() == Some(&0x1b) {
                // Unknown escape sequence: discard its introducer and keep
                // looking for the next Kitty/CSI sequence.
                self.stream.drain(..1);
            } else if !self.stream.is_empty() {
                self.stream.drain(..1);
            } else {
                break;
            }
        }

        // Do not let malformed terminal output grow without bound. Keep a
        // possible partial ESC prefix, which is all the parser needs between
        // ui_send notifications.
        if self.stream.len() > 64 * 1024 {
            let keep = self.stream.ends_with(b"\x1b") || self.stream.ends_with(b"\x1b[");
            if keep {
                let suffix = if self.stream.ends_with(b"\x1b[") {
                    2
                } else {
                    1
                };
                self.stream = self.stream[self.stream.len() - suffix..].to_vec();
            } else {
                self.stream.clear();
            }
        }

        events
    }

    fn handle_csi(&mut self, params: &[u8], final_byte: u8, events: &mut Vec<KittyEvent>) {
        match final_byte {
            b'H' | b'f' => {
                let values = parse_csi_numbers(params);
                let row = values
                    .first()
                    .copied()
                    .filter(|value| *value > 0)
                    .unwrap_or(1);
                let column = values
                    .get(1)
                    .copied()
                    .filter(|value| *value > 0)
                    .unwrap_or(1);
                self.cursor = KittyCursor {
                    row: row.saturating_sub(1),
                    column: column.saturating_sub(1),
                };
            }
            // Snacks uses this query to detect the terminal. The response
            // string intentionally includes the DCS envelope: Neovim's
            // TermResponse autocmd matches the inner `P>|...` sequence.
            b'q' if params == b">" => events.push(KittyEvent::TerminalResponse(
                "\x1bP>|kitty 0.40.0\x1b\\".to_owned(),
            )),
            _ => {}
        }
    }

    fn handle_graphics_frame(
        &mut self,
        frame: &[u8],
        grid: GridId,
        store: &mut ImageStore,
        events: &mut Vec<KittyEvent>,
    ) {
        let (control, payload) = frame
            .iter()
            .position(|byte| *byte == b';')
            .map(|separator| (&frame[..separator], &frame[separator + 1..]))
            .unwrap_or((frame, &[]));
        let controls = control
            .split(|byte| *byte == b',')
            .filter_map(|entry| {
                let separator = entry.iter().position(|byte| *byte == b'=')?;
                let key = std::str::from_utf8(&entry[..separator]).ok()?.to_owned();
                let value = std::str::from_utf8(&entry[separator + 1..])
                    .ok()?
                    .to_owned();
                Some((key, value))
            })
            .collect::<HashMap<_, _>>();

        let action = controls.get("a").map(String::as_str);
        match action {
            // Kitty treats a transfer frame with no `a` control as the
            // default transmit action. Snacks uses this compact form for
            // local file transfers (`t=f`).
            Some("T") | None if controls.contains_key("t") => {
                self.handle_transfer_start(&controls, payload, store, events)
            }
            None if self.transfer.is_some() => {
                self.handle_transfer_continuation(&controls, payload, store, events)
            }
            Some("p") => self.handle_place(&controls, grid, store),
            Some("d") => self.handle_delete(&controls, store, events),
            _ => {}
        }
    }

    fn handle_transfer_start(
        &mut self,
        controls: &HashMap<String, String>,
        payload: &[u8],
        store: &mut ImageStore,
        events: &mut Vec<KittyEvent>,
    ) {
        let Some(image) = parse_control_u32(controls, "i").map(ImageId) else {
            return;
        };
        let transfer = Transfer {
            image,
            format: parse_control_u32(controls, "f").unwrap_or(100),
            transmission: controls.get("t").cloned().unwrap_or_else(|| "d".to_owned()),
            width: parse_control_u32(controls, "s"),
            height: parse_control_u32(controls, "v"),
            encoded_payload: payload.to_vec(),
        };
        let more = parse_control_u32(controls, "m") == Some(1);
        if more {
            self.transfer = Some(transfer);
        } else {
            self.finish_transfer(transfer, store, events);
        }
    }

    fn handle_transfer_continuation(
        &mut self,
        controls: &HashMap<String, String>,
        payload: &[u8],
        store: &mut ImageStore,
        events: &mut Vec<KittyEvent>,
    ) {
        let Some(mut transfer) = self.transfer.take() else {
            return;
        };
        if transfer.encoded_payload.len().saturating_add(payload.len()) > MAX_IMAGE_BYTES * 2 {
            return;
        }
        transfer.encoded_payload.extend_from_slice(payload);
        if parse_control_u32(controls, "m") == Some(1) {
            self.transfer = Some(transfer);
        } else {
            self.finish_transfer(transfer, store, events);
        }
    }

    fn finish_transfer(
        &mut self,
        transfer: Transfer,
        store: &mut ImageStore,
        events: &mut Vec<KittyEvent>,
    ) {
        let Ok(mut bytes) = STANDARD.decode(&transfer.encoded_payload) else {
            return;
        };

        if transfer.transmission == "f" {
            let Ok(path) = String::from_utf8(bytes) else {
                return;
            };
            let path = Path::new(&path);
            let Ok(metadata) = fs::metadata(path) else {
                return;
            };
            if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES as u64 {
                return;
            }
            let Ok(file_bytes) = fs::read(path) else {
                return;
            };
            bytes = file_bytes;
        }

        let Some((bytes, format)) =
            normalize_image(transfer.format, bytes, transfer.width, transfer.height)
        else {
            return;
        };
        let evicted = store.insert_asset_with_format(transfer.image, bytes, format);
        for image in evicted {
            events.push(KittyEvent::AssetDeleted { image });
        }
        events.push(KittyEvent::AssetUpdated {
            image: transfer.image,
            format,
        });
    }

    fn handle_place(
        &mut self,
        controls: &HashMap<String, String>,
        grid: GridId,
        store: &mut ImageStore,
    ) {
        let Some(image) = parse_control_u32(controls, "i").map(ImageId) else {
            return;
        };
        if store.asset(image).is_none() {
            return;
        }
        let placement = parse_control_u32(controls, "p").unwrap_or(0);
        let virtual_placeholder = parse_control_u32(controls, "U") == Some(1);
        store.place(ImagePlacement {
            key: PlacementKey { image, placement },
            anchor: if virtual_placeholder {
                GridAnchor {
                    grid: GridId(0),
                    row: 0,
                    column: 0,
                }
            } else {
                GridAnchor {
                    grid,
                    row: self.cursor.row,
                    column: self.cursor.column,
                }
            },
            columns: parse_control_u32(controls, "c").unwrap_or(1).max(1),
            rows: parse_control_u32(controls, "r").unwrap_or(1).max(1),
            z_index: controls
                .get("z")
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or(0),
            virtual_placeholder,
        });
    }

    fn handle_delete(
        &mut self,
        controls: &HashMap<String, String>,
        store: &mut ImageStore,
        events: &mut Vec<KittyEvent>,
    ) {
        let delete_kind = controls.get("d").map(String::as_str).unwrap_or("a");
        match delete_kind {
            "a" => {
                // Lowercase delete commands remove visible placements but
                // keep image data cached for a later `a=p`. Snacks relies on
                // this when it closes the last preview placement and then
                // reuses the same image id when the preview is opened again.
                store.placements.clear();
                store.prune_after_delete(events);
            }
            "i" => {
                let Some(image) = parse_control_u32(controls, "i").map(ImageId) else {
                    return;
                };
                if controls.contains_key("p") {
                    // Kitty's lowercase `d=i,p=<placement>` removes only
                    // that placement and keeps the transmitted image data.
                    // Snacks reuses the same image id when a picker preview
                    // is shown again, so deleting the asset here makes the
                    // later `a=p` silently fail.
                    let Some(placement) = parse_control_u32(controls, "p") else {
                        return;
                    };
                    store.remove_placement(PlacementKey { image, placement });
                } else {
                    // Lowercase `d=i` is a soft delete: remove all
                    // placements for the image, but retain its data.
                    store.placements.retain(|key, _| key.image != image);
                }
                store.prune_after_delete(events);
            }
            "A" => {
                store.clear_assets_and_placements();
                events.push(KittyEvent::AssetsCleared);
            }
            "I" => {
                let Some(image) = parse_control_u32(controls, "i").map(ImageId) else {
                    return;
                };
                if let Some(placement) = parse_control_u32(controls, "p") {
                    store.remove_placement(PlacementKey { image, placement });
                    if !store.has_placements(image) && store.remove_asset(image) {
                        events.push(KittyEvent::AssetDeleted { image });
                    }
                } else {
                    store.placements.retain(|key, _| key.image != image);
                    if store.remove_asset(image) {
                        events.push(KittyEvent::AssetDeleted { image });
                    }
                }
            }
            "p" => {
                let Some(image) = parse_control_u32(controls, "i").map(ImageId) else {
                    return;
                };
                let placement = parse_control_u32(controls, "p").unwrap_or(0);
                store.remove_placement(PlacementKey { image, placement });
            }
            _ => {}
        }
    }
}

fn parse_control_u32(controls: &HashMap<String, String>, key: &str) -> Option<u32> {
    controls.get(key)?.parse().ok()
}

fn normalize_image(
    kitty_format: u32,
    bytes: Vec<u8>,
    width: Option<u32>,
    height: Option<u32>,
) -> Option<(Vec<u8>, ImageFormatKind)> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }

    if kitty_format == 100 || !matches!(kitty_format, 24 | 32) {
        let format = ImageFormatKind::detect(&bytes);
        return Some((bytes, format));
    }

    let (width, height) = (width?, height?);
    let channels = if kitty_format == 24 { 3 } else { 4 };
    let expected = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(channels)?;
    if expected != bytes.len() {
        return None;
    }

    let mut rgba = Vec::with_capacity(expected / channels * 4);
    for pixel in bytes.chunks_exact(channels) {
        rgba.extend_from_slice(&pixel[..3]);
        rgba.push(if channels == 4 { pixel[3] } else { 255 });
    }
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .ok()?;
    Some((encoded.into_inner(), ImageFormatKind::Png))
}

fn find_st_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    bytes[start..]
        .windows(2)
        .position(|window| window == b"\x1b\\")
        .map(|offset| start + offset)
}

fn find_tmux_st_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    while index + 1 < bytes.len() {
        if bytes[index] != 0x1b {
            index += 1;
            continue;
        }
        if bytes[index + 1] == 0x1b {
            index += 2;
        } else if bytes[index + 1] == b'\\' {
            return Some(index);
        } else {
            index += 1;
        }
    }
    None
}

fn unescape_tmux(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&0x1b) {
            result.push(0x1b);
            index += 2;
        } else {
            result.push(bytes[index]);
            index += 1;
        }
    }
    result
}

fn parse_csi_numbers(params: &[u8]) -> Vec<u32> {
    params
        .split(|byte| *byte == b';' || *byte == b':')
        .filter_map(|value| {
            let value = value.strip_prefix(b">").unwrap_or(value);
            std::str::from_utf8(value).ok()?.parse().ok()
        })
        .collect()
}

/// Return the one-based placeholder row/column encoded in a Kitty cell.
/// The caller obtains the image id from the cell's foreground highlight.
pub fn placeholder_position(text: &str) -> Option<(u32, u32)> {
    let mut chars = text.chars();
    (chars.next() == Some(KITTY_PLACEHOLDER)).then_some(())?;
    let row = diacritic_index(chars.next()?)?;
    let column = diacritic_index(chars.next()?)?;
    Some((row, column))
}

/// Whether a grid cell contains Kitty's private-use placeholder character.
pub fn is_kitty_placeholder(text: &str) -> bool {
    text.starts_with(KITTY_PLACEHOLDER)
}

fn diacritic_index(character: char) -> Option<u32> {
    DIACRITICS
        .split(',')
        .position(|code| u32::from_str_radix(code, 16).ok() == Some(character as u32))
        .and_then(|index| u32::try_from(index + 1).ok())
}

// This is the same stable ordering used by Snacks image placement.
const DIACRITICS: &str = "0305,030D,030E,0310,0312,033D,033E,033F,0346,034A,034B,034C,0350,0351,0352,0357,035B,0363,0364,0365,0366,0367,0368,0369,036A,036B,036C,036D,036E,036F,0483,0484,0485,0486,0487,0592,0593,0594,0595,0597,0598,0599,059C,059D,059E,059F,05A0,05A1,05A8,05A9,05AB,05AC,05AF,05C4,0610,0611,0612,0613,0614,0615,0616,0617,0657,0658,0659,065A,065B,065D,065E,06D6,06D7,06D8,06D9,06DA,06DB,06DC,06DF,06E0,06E1,06E2,06E4,06E7,06E8,06EB,06EC,0730,0732,0733,0735,0736,073A,073D,073F,0740,0741,0743,0745,0747,0749,074A,07EB,07EC,07ED,07EE,07EF,07F0,07F1,07F3,0816,0817,0818,0819,081B,081C,081D,081E,081F,0820,0821,0822,0823,0825,0826,0827,0829,082A,082B,082C,082D,0951,0953,0954,0F82,0F83,135D,135E,135F,17DD,193A,1A17,1A75,1A76,1A77,1A78,1A79,1A7A,1A7B,1A7C,1B6B,1B6D,1B6E,1B6F,1B70,1B71,1B72,1B73,1CD0,1CD1,1CD2,1CDA,1CDB,1CE0,1DC0,1DC1,1DC3,1DC4,1DC5,1DC6,1DC7,1DC8,1DC9,1DCB,1DCC,1DD1,1DD2,1DD3,1DD4,1DD5,1DD6,1DD7,1DD8,1DD9,1DDA,1DDB,1DDC,1DDD,1DDE,1DDF,1DE0,1DE1,1DE2,1DE3,1DE4,1DE5,1DE6,1DFE,20D0,20D1,20D4,20D5,20D6,20D7,20DB,20DC,20E1,20E7,20E9,20F0,2CEF,2CF0,2CF1,2DE0,2DE1,2DE2,2DE3,2DE4,2DE5,2DE6,2DE7,2DE8,2DE9,2DEA,2DEB,2DEC,2DED,2DEE,2DEF,2DF0,2DF1,2DF2,2DF3,2DF4,2DF5,2DF6,2DF7,2DF8,2DF9,2DFA,2DFB,2DFC,2DFD,2DFE,2DFF,A66F,A67C,A67D,A6F0,A6F1,A8E0,A8E1,A8E2,A8E3,A8E4,A8E5,A8E6,A8E7,A8E8,A8E9,A8EA,A8EB,A8EC,A8ED,A8EE,A8EF,A8F0,A8F1,A8F2,A8F3,A8F4,A8F5,A8F6,A8F7,A8F8,A8F9,A8FA,A8FB,A8FC,A8FD,A8FE,A8FF,AAB0,AAB2,AAB3,AAB7,AAB8,AABE,AABF,AAC1,FE20,FE21,FE22,FE23,FE24,FE25,FE26,10A0F,10A38,1D185,1D186,1D187,1D188,1D189,1D1AA,1D1AB,1D1AC,1D1AD,1D242,1D243,1D244";

#[cfg(test)]
mod tests {
    use super::{
        placeholder_position, GridAnchor, GridId, ImageFormatKind, ImageId, ImagePlacement,
        ImageStore, KittyEvent, PlacementKey,
    };
    use base64::Engine as _;

    #[test]
    fn one_asset_can_have_multiple_grid_placements() {
        let image = ImageId(7);
        let first_key = PlacementKey {
            image,
            placement: 1,
        };
        let second_key = PlacementKey {
            image,
            placement: 2,
        };
        let mut store = ImageStore::new();

        store.insert_asset(image, vec![1, 2, 3]);
        store.place(ImagePlacement {
            key: first_key,
            anchor: GridAnchor {
                grid: GridId(1),
                row: 2,
                column: 3,
            },
            columns: 4,
            rows: 5,
            z_index: -1,
            virtual_placeholder: false,
        });
        store.place(ImagePlacement {
            key: second_key,
            anchor: GridAnchor {
                grid: GridId(2),
                row: 8,
                column: 13,
            },
            columns: 2,
            rows: 2,
            z_index: 0,
            virtual_placeholder: false,
        });

        assert_eq!(store.asset(image).map(|asset| asset.encoded.len()), Some(3));
        assert_eq!(store.placement(first_key).map(|item| item.rows), Some(5));
        assert_eq!(
            store.placement(second_key).map(|item| item.anchor.grid),
            Some(GridId(2))
        );
        assert!(store.remove_placement(first_key).is_some());
        assert!(store.placement(first_key).is_none());
        assert!(store.placement(second_key).is_some());
    }

    #[test]
    fn parses_kitty_transfer_and_placeholder_placement() {
        let mut store = ImageStore::new();
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAQMAAAAl21bKAAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAAGUExURf8AAP///0EdNBEAAAABYktHRAH/Ai3eAAAACklEQVQI12NgAAAAAgAB4iG8MwAAAABJRU5ErkJggg==";
        let events = store.consume_ui_data(
            &format!("\x1b_Ga=T,f=100,t=d,i=7,m=0;{png}\x1b\\\x1b_Ga=p,U=1,i=7,p=9,c=3,r=2\x1b\\"),
            GridId(1),
        );

        assert!(events.contains(&KittyEvent::AssetUpdated {
            image: ImageId(7),
            format: super::ImageFormatKind::Png,
        }));
        assert!(image::load_from_memory(
            &store
                .asset(ImageId(7))
                .expect("asset should be stored")
                .encoded
        )
        .is_ok());
        assert!(store
            .placement(PlacementKey {
                image: ImageId(7),
                placement: 9
            })
            .is_some());
    }

    #[test]
    fn deleting_a_placement_keeps_the_image_for_reuse() {
        let mut store = ImageStore::new();
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAQMAAAAl21bKAAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAAGUExURf8AAP///0EdNBEAAAABYktHRAH/Ai3eAAAACklEQVQI12NgAAAAAgAB4iG8MwAAAABJRU5ErkJggg==";
        let key = PlacementKey {
            image: ImageId(7),
            placement: 9,
        };

        store.consume_ui_data(
            &format!("\x1b_Ga=T,f=100,t=d,i=7,m=0;{png}\x1b\\\x1b_Ga=p,U=1,i=7,p=9,c=3,r=2\x1b\\"),
            GridId(1),
        );
        store.consume_ui_data("\x1b_Ga=d,d=i,i=7,p=9\x1b\\", GridId(1));

        assert!(store.asset(ImageId(7)).is_some());
        assert!(store.placement(key).is_none());

        store.consume_ui_data("\x1b_Ga=p,U=1,i=7,p=9,c=3,r=2\x1b\\", GridId(1));
        assert!(store.placement(key).is_some());
    }

    #[test]
    fn soft_deleting_an_image_keeps_the_asset_for_reuse() {
        let mut store = ImageStore::new();
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAQMAAAAl21bKAAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAAGUExURf8AAP///0EdNBEAAAABYktHRAH/Ai3eAAAACklEQVQI12NgAAAAAgAB4iG8MwAAAABJRU5ErkJggg==";
        let key = PlacementKey {
            image: ImageId(12),
            placement: 14,
        };

        store.consume_ui_data(
            &format!(
                "\x1b_Ga=T,f=100,t=d,i=12,m=0;{png}\x1b\\\x1b_Ga=p,U=1,i=12,p=14,c=3,r=2\x1b\\"
            ),
            GridId(1),
        );
        store.consume_ui_data("\x1b_Ga=d,d=i,i=12\x1b\\", GridId(1));

        assert!(store.asset(ImageId(12)).is_some());
        assert!(store.placement(key).is_none());

        store.consume_ui_data("\x1b_Ga=p,U=1,i=12,p=14,c=3,r=2\x1b\\", GridId(1));
        assert!(store.placement(key).is_some());
    }

    #[test]
    fn soft_deleted_assets_are_evicted_when_the_cache_is_full() {
        let mut store = ImageStore::new();
        store.max_asset_bytes = 2;
        store.insert_asset_with_format(ImageId(21), vec![1, 2], ImageFormatKind::Png);
        store.consume_ui_data("\x1b_Ga=p,U=1,i=21,p=1,c=1,r=1\x1b\\", GridId(1));
        store.consume_ui_data("\x1b_Ga=d,d=i,i=21\x1b\\", GridId(1));

        assert!(store.asset(ImageId(21)).is_some());
        assert!(store
            .placements()
            .all(|placement| placement.key.image != ImageId(21)));

        let evicted = store.insert_asset_with_format(ImageId(22), vec![3, 4], ImageFormatKind::Png);

        assert_eq!(evicted, vec![ImageId(21)]);
        assert!(store.asset(ImageId(21)).is_none());
        assert!(store.asset(ImageId(22)).is_some());
    }

    #[test]
    fn parses_snacks_compact_file_transfer_without_an_action() {
        let mut store = ImageStore::new();
        let path = std::env::temp_dir().join("nvim-gpui-kitty-test.png");
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABAQMAAAAl21bKAAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAAGUExURf8AAP///0EdNBEAAAABYktHRAH/Ai3eAAAACklEQVQI12NgAAAAAgAB4iG8MwAAAABJRU5ErkJggg==";
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .expect("test PNG should decode");
        std::fs::write(&path, bytes).expect("test PNG should be written");
        let encoded_path =
            base64::engine::general_purpose::STANDARD.encode(path.to_string_lossy().as_bytes());
        let events = store.consume_ui_data(
            &format!("\x1b_Gt=f,f=100,i=12;{encoded_path}\x1b\\"),
            GridId(1),
        );
        let _ = std::fs::remove_file(path);

        assert!(events.contains(&KittyEvent::AssetUpdated {
            image: ImageId(12),
            format: super::ImageFormatKind::Png,
        }));
    }

    #[test]
    fn parses_cursor_position_and_terminal_detection() {
        let mut store = ImageStore::new();
        let events = store.consume_ui_data("\x1b[4;6H\x1b[>q", GridId(1));
        assert_eq!(
            events,
            vec![KittyEvent::TerminalResponse(
                "\x1bP>|kitty 0.40.0\x1b\\".to_owned()
            )]
        );
    }

    #[test]
    fn parses_a_tmux_wrapped_kitty_frame() {
        let mut store = ImageStore::new();
        let inner = b"\x1b_Ga=T,f=100,t=d,i=8,m=0;iVBORw0KGgo=\x1b\\";
        let mut wrapped = b"\x1bPtmux;".to_vec();
        for byte in inner {
            if *byte == 0x1b {
                wrapped.push(0x1b);
            }
            wrapped.push(*byte);
        }
        wrapped.extend_from_slice(b"\x1b\\");
        let data = String::from_utf8(wrapped).expect("test stream is UTF-8");

        let events = store.consume_ui_data(&data, GridId(1));
        assert!(events.contains(&KittyEvent::AssetUpdated {
            image: ImageId(8),
            format: super::ImageFormatKind::Png,
        }));
    }

    #[test]
    fn decodes_snacks_placeholder_position() {
        let text = format!("{}{}{}", '\u{10eeee}', '\u{0305}', '\u{030e}');
        assert_eq!(placeholder_position(&text), Some((1, 3)));
    }
}
