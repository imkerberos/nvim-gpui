use std::sync::Arc;

use gpui::Image;

pub(crate) fn logo_image() -> Arc<Image> {
    Arc::new(Image::from_bytes(
        gpui::ImageFormat::Png,
        include_bytes!("../../assets/icons/neovim-gpui.png").to_vec(),
    ))
}

pub(crate) fn titlebar_logo_image() -> Arc<Image> {
    // Keep a separate, padded asset so the compact titlebar logo does not
    // visually fill the titlebar while the application icon can use the
    // available canvas more fully.
    Arc::new(Image::from_bytes(
        gpui::ImageFormat::Png,
        include_bytes!("../../assets/icons/neovim-gpui-titlebar.png").to_vec(),
    ))
}
