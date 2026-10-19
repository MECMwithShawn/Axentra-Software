//! The window icon: the Axentra Glass + Precision "Outlook" icon (calendar
//! page behind an envelope), rendered by the suite icon kit
//! (Axentra-Software/Icon-Kit, app id `outlook`). Axentra's own artwork, not
//! any Microsoft product logo.

/// 256 px PNG from the icon kit; the window manager scales it as needed.
pub const APP_ICON_PNG: &[u8] = include_bytes!("../../../packaging/linux/icons/axentra-outlook-256.png");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icon_is_a_256px_png() {
        assert_eq!(&APP_ICON_PNG[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes(APP_ICON_PNG[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(APP_ICON_PNG[20..24].try_into().unwrap());
        assert_eq!((width, height), (256, 256));
    }
}
