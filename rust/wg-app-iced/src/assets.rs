//! The application icon, decoded once from the PNG that the Slint app uses too.

use iced::widget::image;
use iced::window;
use std::io::Cursor;
use std::sync::OnceLock;

const ICON_PNG: &[u8] = include_bytes!("../../wg-app/assets/icon.png");

struct Icon {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    handle: image::Handle,
}

fn decode() -> Option<Icon> {
    let mut reader = png::Decoder::new(Cursor::new(ICON_PNG)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    let handle = image::Handle::from_rgba(info.width, info.height, buf.clone());
    Some(Icon { width: info.width, height: info.height, rgba: buf, handle })
}

fn icon() -> Option<&'static Icon> {
    static ICON: OnceLock<Option<Icon>> = OnceLock::new();
    ICON.get_or_init(decode).as_ref()
}

/// The icon for a window's title bar and taskbar button.
pub fn window_icon() -> Option<window::Icon> {
    let i = icon()?;
    window::icon::from_rgba(i.rgba.clone(), i.width, i.height).ok()
}

/// The icon as an image widget source. The handle is created once: a new one per frame
/// would be uploaded again every time.
pub fn image_handle() -> Option<image::Handle> {
    icon().map(|i| i.handle.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_icon_decodes() {
        let i = icon().expect("icon.png is a 8-bit RGBA PNG");
        assert_eq!((i.width, i.height), (256, 256));
        assert_eq!(i.rgba.len(), 256 * 256 * 4);
        assert!(window_icon().is_some());
        assert!(image_handle().is_some());
    }
}
