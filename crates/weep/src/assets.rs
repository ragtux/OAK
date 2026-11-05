//! What the binary carries: the icons, the two font families and the
//! bundled themes and keymap.
//!
//! Icons come from two places, both drawn on a 16-unit grid with 1 px
//! strokes so they mix in one toolbar: JetBrains' Apache-licensed `expui`
//! set (`assets/icons/expui`, each file with its own copyright header) and
//! WEEP's own drawings (`assets/icons/weep`). gpui paints an SVG as a mask in
//! the current text colour, so every icon is monochrome. The fonts are Inter
//! and JetBrains Mono, both SIL OFL, the two faces Fleet's UI and editor use.

use anyhow::Result;
use gpui::{px, svg, AssetSource, Hsla, SharedString, Styled, Svg};
use std::borrow::Cow;

pub struct Assets;

macro_rules! bundle {
    ($($path:literal),* $(,)?) => {
        const FILES: &[(&str, &[u8])] = &[
            $(($path, include_bytes!(concat!("../assets/", $path)))),*
        ];
    };
}

bundle!(
    "icons/weep/logo.svg",
    "icons/weep/panel-left.svg",
    "icons/weep/panel-bottom.svg",
    "icons/weep/panel-right.svg",
    "icons/weep/grid.svg",
    "icons/weep/terminal.svg",
    "icons/weep/user-add.svg",
    "icons/weep/git-branch.svg",
    "icons/weep/caption-minimize.svg",
    "icons/weep/caption-maximize.svg",
    "icons/weep/caption-restore.svg",
    "icons/weep/caption-close.svg",
    "icons/weep/panel-left-24.svg",
    "icons/weep/panel-bottom-24.svg",
    "icons/weep/panel-right-24.svg",
    "icons/weep/breakpoint.svg",
    "icons/weep/console.svg",
    "icons/weep/diff.svg",
    "icons/weep/snapshot.svg",
    "icons/weep/preview.svg",
    "icons/weep/check.svg",
    "icons/weep/arrow-up.svg",
    "icons/weep/arrow-down.svg",
    "icons/weep/match-case.svg",
    "icons/weep/whole-word.svg",
    "icons/weep/regex.svg",
    "icons/weep/sliders.svg",
    "icons/weep/circle.svg",
    "icons/weep/exclamation.svg",
    "icons/weep/run-gutter.svg",
    "icons/weep/file-rust.svg",
    "icons/weep/file-toml.svg",
    "icons/weep/file-json.svg",
    "icons/weep/file-markdown.svg",
    "icons/weep/file-text.svg",
    "icons/expui/general/menu.svg",
    "icons/expui/general/search.svg",
    "icons/expui/general/settings.svg",
    "icons/expui/general/chevronDown.svg",
    "icons/expui/general/chevronRight.svg",
    "icons/expui/general/chevronUp.svg",
    "icons/expui/general/chevronLeft.svg",
    "icons/expui/general/closeSmall.svg",
    "icons/expui/general/close.svg",
    "icons/expui/general/add.svg",
    "icons/expui/general/moreHorizontal.svg",
    "icons/expui/general/moreVertical.svg",
    "icons/expui/general/splitVertically.svg",
    "icons/expui/general/copy.svg",
    "icons/expui/general/edit.svg",
    "icons/expui/general/delete.svg",
    "icons/expui/general/filter.svg",
    "icons/expui/general/collapseAll.svg",
    "icons/expui/general/expandAll.svg",
    "icons/expui/general/refresh.svg",
    "icons/expui/general/undo.svg",
    "icons/expui/general/redo.svg",
    "icons/expui/general/pin.svg",
    "icons/expui/general/locked.svg",
    "icons/expui/general/show.svg",
    "icons/expui/general/hide.svg",
    "icons/expui/general/user.svg",
    "icons/expui/general/history.svg",
    "icons/expui/general/layout.svg",
    "icons/expui/general/questionMark.svg",
    "icons/expui/run/run.svg",
    "icons/expui/run/debug.svg",
    "icons/expui/run/stop.svg",
    "icons/expui/run/rerun.svg",
    "icons/expui/run/restart.svg",
    "icons/expui/actions/lightning.svg",
    "icons/expui/actions/replace.svg",
    "icons/expui/toolwindows/notifications.svg",
    "icons/expui/toolwindows/problems.svg",
    "icons/expui/toolwindows/find.svg",
    "icons/expui/toolwindows/project.svg",
    "icons/expui/toolwindows/structure.svg",
    "icons/expui/toolwindows/vcs.svg",
    "icons/expui/toolwindows/commit.svg",
    "icons/expui/toolwindows/build.svg",
    "icons/expui/nodes/folder.svg",
    "icons/expui/nodes/module.svg",
    "icons/expui/nodes/symlink.svg",
    "icons/expui/nodes/unknown.svg",
    // M7: the completion rows' kind icons and the problems widget's
    // weak-warning icon.
    "icons/expui/nodes/function.svg",
    "icons/expui/nodes/method.svg",
    "icons/expui/nodes/field.svg",
    "icons/expui/nodes/property.svg",
    "icons/expui/nodes/variable.svg",
    "icons/expui/nodes/class.svg",
    "icons/expui/nodes/interface.svg",
    "icons/expui/nodes/enum.svg",
    "icons/expui/nodes/constant.svg",
    "icons/expui/nodes/parameter.svg",
    "icons/expui/nodes/tag.svg",
    "icons/expui/status/warningOutline.svg",
    "icons/expui/fileTypes/toml.svg",
    "icons/expui/fileTypes/json.svg",
    "icons/expui/fileTypes/markdown.svg",
    "icons/expui/fileTypes/text.svg",
    "icons/expui/fileTypes/yaml.svg",
    "icons/expui/fileTypes/html.svg",
    "icons/expui/fileTypes/css.svg",
    "icons/expui/fileTypes/javaScript.svg",
    "icons/expui/fileTypes/typeScript.svg",
    "icons/expui/fileTypes/shell.svg",
    "icons/expui/fileTypes/archive.svg",
    "icons/expui/fileTypes/image.svg",
    "icons/expui/fileTypes/xml.svg",
    "icons/expui/fileTypes/gitignore.svg",
    "icons/expui/fileTypes/unknown.svg",
    "icons/expui/fileTypes/anyType.svg",
    "icons/expui/fileTypes/config.svg",
    "icons/expui/fileTypes/properties.svg",
    "icons/expui/fileTypes/csv.svg",
    "icons/expui/fileTypes/sql.svg",
    "icons/expui/fileTypes/font.svg",
    "icons/expui/fileTypes/docker.svg",
    "icons/expui/status/error.svg",
    "icons/expui/status/warning.svg",
    "icons/expui/status/info.svg",
    "icons/expui/status/success.svg",
    "icons/expui/gutter/fold.svg",
    "icons/expui/gutter/unfold.svg",
);

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

/// The static faces registered at startup, so both families resolve on a
/// machine that has neither installed.
pub fn fonts() -> Vec<Cow<'static, [u8]>> {
    vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Italic.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-BoldItalic.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf")),
        Cow::Borrowed(include_bytes!(
            "../assets/fonts/JetBrainsMono-BoldItalic.ttf"
        )),
    ]
}

pub const UI_FONT: &str = "Inter";
pub const MONO_FONT: &str = "JetBrains Mono";

/// Bundled themes by the name `--theme` takes.
pub const THEMES: &[(&str, &str)] = &[
    ("dark", include_str!("../assets/themes/dark.json")),
    (
        "dark_purple",
        include_str!("../assets/themes/dark_purple.json"),
    ),
    ("light", include_str!("../assets/themes/light.json")),
];

pub const DEFAULT_KEYMAP: &str = include_str!("../assets/keymap/default.json");

/// An icon by its path under `assets/icons`, `size` pixels square, painted in `color`.
pub fn icon(path: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{path}.svg")))
        .size(px(size))
        .flex_shrink_0()
        .text_color(color)
}

/// The icon a file row and a tab show for `name`. gpui paints an SVG as a
/// single-colour mask, so only outline drawings survive: the expui
/// file-type icons are filled shapes and would come out as blobs, hence
/// WEEP's own monograms and one generic document for everything else.
pub fn file_icon(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "rs" => "weep/file-rust",
        "toml" => "weep/file-toml",
        "json" | "jsonc" | "json5" => "weep/file-json",
        "md" | "markdown" => "weep/file-markdown",
        _ => "weep/file-text",
    }
}
