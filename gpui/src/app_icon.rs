//! The app's own artwork: the embedded asset source for the logo, and the Dock
//! icon for runs that are not inside a `.app` bundle (a bundle gets its icon
//! from `Info.plist`, so there this is a harmless no-op replacement).

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

const LOGO_SVG: &[u8] = include_bytes!("../assets/logo.svg");
#[cfg(target_os = "macos")]
const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Hugeicons the web app uses, exported by `scripts/export-gpui-icons.ts`.
const ICONS: &[(&str, &[u8])] = &[
    ("icons/alert-circle.svg", include_bytes!("../assets/icons/alert-circle.svg")),
    ("icons/alert01.svg", include_bytes!("../assets/icons/alert01.svg")),
    ("icons/arrow-down01.svg", include_bytes!("../assets/icons/arrow-down01.svg")),
    ("icons/arrow-right01.svg", include_bytes!("../assets/icons/arrow-right01.svg")),
    ("icons/arrow-up01.svg", include_bytes!("../assets/icons/arrow-up01.svg")),
    ("icons/cancel01.svg", include_bytes!("../assets/icons/cancel01.svg")),
    ("icons/center-focus.svg", include_bytes!("../assets/icons/center-focus.svg")),
    ("icons/chart01.svg", include_bytes!("../assets/icons/chart01.svg")),
    ("icons/check-list.svg", include_bytes!("../assets/icons/check-list.svg")),
    ("icons/checkmark-circle02.svg", include_bytes!("../assets/icons/checkmark-circle02.svg")),
    ("icons/clipboard.svg", include_bytes!("../assets/icons/clipboard.svg")),
    ("icons/code-square.svg", include_bytes!("../assets/icons/code-square.svg")),
    ("icons/column-delete.svg", include_bytes!("../assets/icons/column-delete.svg")),
    ("icons/column-insert.svg", include_bytes!("../assets/icons/column-insert.svg")),
    ("icons/copy01.svg", include_bytes!("../assets/icons/copy01.svg")),
    ("icons/delete02.svg", include_bytes!("../assets/icons/delete02.svg")),
    ("icons/external-link.svg", include_bytes!("../assets/icons/external-link.svg")),
    ("icons/file-add.svg", include_bytes!("../assets/icons/file-add.svg")),
    ("icons/file-export.svg", include_bytes!("../assets/icons/file-export.svg")),
    ("icons/file-text.svg", include_bytes!("../assets/icons/file-text.svg")),
    ("icons/file01.svg", include_bytes!("../assets/icons/file01.svg")),
    ("icons/floppy-disk.svg", include_bytes!("../assets/icons/floppy-disk.svg")),
    ("icons/folder-add.svg", include_bytes!("../assets/icons/folder-add.svg")),
    ("icons/folder-open.svg", include_bytes!("../assets/icons/folder-open.svg")),
    ("icons/folder01.svg", include_bytes!("../assets/icons/folder01.svg")),
    ("icons/heading01.svg", include_bytes!("../assets/icons/heading01.svg")),
    ("icons/heading02.svg", include_bytes!("../assets/icons/heading02.svg")),
    ("icons/heading03.svg", include_bytes!("../assets/icons/heading03.svg")),
    ("icons/image01.svg", include_bytes!("../assets/icons/image01.svg")),
    ("icons/information-circle.svg", include_bytes!("../assets/icons/information-circle.svg")),
    ("icons/keyboard.svg", include_bytes!("../assets/icons/keyboard.svg")),
    ("icons/left-to-right-list-bullet.svg", include_bytes!("../assets/icons/left-to-right-list-bullet.svg")),
    ("icons/left-to-right-list-number.svg", include_bytes!("../assets/icons/left-to-right-list-number.svg")),
    ("icons/link01.svg", include_bytes!("../assets/icons/link01.svg")),
    ("icons/list-tree.svg", include_bytes!("../assets/icons/list-tree.svg")),
    ("icons/minus-sign.svg", include_bytes!("../assets/icons/minus-sign.svg")),
    ("icons/more-horizontal.svg", include_bytes!("../assets/icons/more-horizontal.svg")),
    ("icons/multiplication-sign.svg", include_bytes!("../assets/icons/multiplication-sign.svg")),
    ("icons/paint-board.svg", include_bytes!("../assets/icons/paint-board.svg")),
    ("icons/pdf01.svg", include_bytes!("../assets/icons/pdf01.svg")),
    ("icons/pencil-edit01.svg", include_bytes!("../assets/icons/pencil-edit01.svg")),
    ("icons/plus-sign.svg", include_bytes!("../assets/icons/plus-sign.svg")),
    ("icons/puzzle.svg", include_bytes!("../assets/icons/puzzle.svg")),
    ("icons/quote-down.svg", include_bytes!("../assets/icons/quote-down.svg")),
    ("icons/row-delete.svg", include_bytes!("../assets/icons/row-delete.svg")),
    ("icons/row-insert.svg", include_bytes!("../assets/icons/row-insert.svg")),
    ("icons/search-add.svg", include_bytes!("../assets/icons/search-add.svg")),
    ("icons/search-minus.svg", include_bytes!("../assets/icons/search-minus.svg")),
    ("icons/search01.svg", include_bytes!("../assets/icons/search01.svg")),
    ("icons/settings01.svg", include_bytes!("../assets/icons/settings01.svg")),
    ("icons/sidebar-left.svg", include_bytes!("../assets/icons/sidebar-left.svg")),
    ("icons/source-code.svg", include_bytes!("../assets/icons/source-code.svg")),
    ("icons/table01.svg", include_bytes!("../assets/icons/table01.svg")),
    ("icons/text-bold.svg", include_bytes!("../assets/icons/text-bold.svg")),
    ("icons/text-font.svg", include_bytes!("../assets/icons/text-font.svg")),
    ("icons/text-italic.svg", include_bytes!("../assets/icons/text-italic.svg")),
    ("icons/text-strikethrough.svg", include_bytes!("../assets/icons/text-strikethrough.svg")),
    ("icons/tick02.svg", include_bytes!("../assets/icons/tick02.svg")),
    ("icons/trash.svg", include_bytes!("../assets/icons/trash.svg")),
    ("icons/unlink01.svg", include_bytes!("../assets/icons/unlink01.svg")),
];

/// Path the logo is registered under, for `gpui::svg().path(..)`.
pub const LOGO_PATH: &str = "logo.svg";

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == LOGO_PATH {
            return Ok(Some(Cow::Borrowed(LOGO_SVG)));
        }
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[allow(unexpected_cfgs)]
/// Replace the generic executable icon in the Dock and the app switcher.
#[cfg(target_os = "macos")]
pub fn set_dock_icon() {
    use cocoa::appkit::{NSApp, NSImage};
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSData;
    use objc::{msg_send, sel, sel_impl};

    // SAFETY: called on the main thread inside `Application::run`, with a
    // buffer that lives for the whole program; AppKit copies the bytes.
    unsafe {
        let data = NSData::dataWithBytes_length_(
            nil,
            ICON_PNG.as_ptr() as *const std::ffi::c_void,
            ICON_PNG.len() as u64,
        );
        let image: id = <id as NSImage>::initWithData_(NSImage::alloc(nil), data);
        if image != nil {
            let app = NSApp();
            let _: () = msg_send![app, setApplicationIconImage: image];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_dock_icon() {}

/// A Hugeicon by its kebab-case file name (`"settings01"`), as a sized,
/// tinted SVG. Strokes are 1.5 wide in the 24px design grid, like the web app.
pub fn icon(name: &str, size: f32) -> gpui::Svg {
    use gpui::{px, Styled};
    gpui::svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_shrink_0()
}
