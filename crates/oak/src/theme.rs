//! The colours and fonts the shell paints from, loaded from a Fleet-format
//! theme (`oak-theme`) and turned into gpui values once.
//!
//! The bundled themes under `assets/themes` carry the values JetBrains
//! Fleet 1.48 computes for its own `dark`, `dark_purple` and `light` themes,
//! read off its theme files on 2026-11-01 (`docs/fleet-probe.md`), keyed by
//! Fleet's own names so a theme file written for Fleet loads here unchanged.
//! Every token below names the Fleet key it comes from.

use crate::assets::{self, MONO_FONT, UI_FONT};
use anyhow::{anyhow, Context as _, Result};
use gpui::{FontWeight, Global, Hsla, Rgba, SharedString};
use oak_theme::Color;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Fleet's sizes, in logical pixels at zoom 1.0. Measured on Fleet 1.48.261
/// rendering at 2.76 device pixels per logical pixel (display 1.38 × zoom 2.0)
/// on KDE, then at 1.5 (display 1.5 × zoom 1.0) on Windows; see
/// `docs/fleet-probe.md` for the raw numbers and what is still estimated.
/// The app paints from [`Metrics`], these values times the zoom level.
pub mod metrics {
    /// The row above the islands: the logo, the panel toggles, the workspace
    /// name, the run and search controls.
    pub const TOOLBAR_HEIGHT: f32 = 36.;
    pub const TOOLBAR_BUTTON: f32 = 28.;
    pub const TOOLBAR_GAP: f32 = 4.;
    /// Window edge to the logo's 24 px box (Windows probe: logo centred at 16.7).
    pub const TOOLBAR_PAD_X: f32 = 4.;
    /// Either side of the 1 px separator (Windows probe: button centre to line 22.7 = 14 + 4 + 4.7).
    pub const TOOLBAR_SEPARATOR_MARGIN: f32 = 4.;
    pub const LOGO_SIZE: f32 = 24.;
    pub const ICON_SIZE: f32 = 16.;
    /// Windows caption buttons drawn into the toolbar: each 46.67 wide, the
    /// toolbar's full height, a 10 px glyph (batch 1).
    pub const CAPTION_BUTTON_WIDTH: f32 = 46.67;
    pub const CAPTION_GLYPH: f32 = 10.;
    /// The window title in the toolbar: the workspace name truncated at 270,
    /// 16 to the branch icon, 8 to the branch name, centred on the window (batch 1).
    pub const TITLE_NAME_MAX_WIDTH: f32 = 270.;
    pub const TITLE_BRANCH_GAP: f32 = 16.;
    pub const TITLE_BRANCH_ICON_GAP: f32 = 8.;
    /// The margin between the window edge and an island, and between two islands;
    /// the gap is also the splitter's hit zone (batch 2).
    pub const ISLAND_MARGIN: f32 = 8.;
    pub const ISLAND_RADIUS: f32 = 8.;
    /// Fleet's Files island in a fresh 1280 × 800 window (Windows probe, batch 1).
    pub const LEFT_PANEL_WIDTH: f32 = 360.;
    /// The right island in a fresh window is the left's width; the bottom island
    /// is 300 tall in an 800 window (batch 2).
    pub const RIGHT_PANEL_WIDTH: f32 = 360.;
    pub const BOTTOM_PANEL_HEIGHT: f32 = 300.;
    /// A panel dragged narrower than this collapses; it reopens at this width (batch 2).
    pub const PANEL_MIN_WIDTH: f32 = 200.;
    /// Estimate: the bottom panel's minimum was not measured (batch 2 hid it below 254 px tall).
    pub const PANEL_MIN_HEIGHT: f32 = 100.;
    /// A tool panel's own tab row ("Files", "Search", "Git", "History", "+").
    pub const PANEL_TAB_HEIGHT: f32 = 28.;
    pub const PANEL_TAB_RADIUS: f32 = 6.;
    /// An unselected panel tab's padding; the selected one is tighter around its
    /// semibold label (Files 13.3-55.3 around text 20-48.7; Search's text starts
    /// 10.7 after it, Git's 20.7 after Search's). Estimate until a probe selects
    /// another tab (`docs/fleet-probe.md`, batch 3 follow-up).
    pub const PANEL_TAB_PAD_X: f32 = 9.;
    pub const PANEL_TAB_SELECTED_PAD_X: f32 = 6.;
    pub const PANEL_TAB_GAP: f32 = 2.;
    /// The `+` box sits 4 after the last tab (its glyph is 21.3 past History's text).
    pub const PANEL_TAB_PLUS_GAP: f32 = 4.;
    pub const PANEL_HEADER_PAD_TOP: f32 = 6.;
    pub const PANEL_HEADER_PAD_BOTTOM: f32 = 5.;
    pub const PANEL_PAD_X: f32 = 6.;
    /// An empty panel: a 24 px icon, 20, the 16 px title on a 20 line, 6, the
    /// 13 px description on a 16 line, 10, a 24 px button; left-aligned in a box
    /// inset 15 % of the island's width, vertically centred (batch 2).
    pub const EMPTY_PANEL_ICON: f32 = 24.;
    pub const EMPTY_PANEL_GAP_ICON: f32 = 20.;
    pub const EMPTY_PANEL_TITLE_SIZE: f32 = 16.;
    pub const EMPTY_PANEL_TITLE_LINE: f32 = 20.;
    pub const EMPTY_PANEL_GAP_TITLE: f32 = 6.;
    pub const EMPTY_PANEL_TEXT_LINE: f32 = 16.;
    pub const EMPTY_PANEL_GAP_TEXT: f32 = 10.;
    pub const EMPTY_PANEL_INSET: f32 = 0.15;
    /// Fleet's secondary button ("Open Tool", batch 2): 24 tall, 92.7 wide for
    /// its 63.3 px label: 6 px padding, a 4 px gap, the chevron in a 12 px box.
    /// The radius is an estimate.
    pub const BUTTON_HEIGHT: f32 = 24.;
    pub const BUTTON_PAD_X: f32 = 6.;
    pub const BUTTON_ICON_GAP: f32 = 4.;
    pub const BUTTON_CHEVRON_BOX: f32 = 12.;
    pub const BUTTON_RADIUS: f32 = 4.;
    /// The keyboard focus ring: 2 px of `focusOutline` outside a 1 px `border.focused` (batch 2).
    pub const FOCUS_OUTLINE: f32 = 2.;
    pub const TREE_ROW_HEIGHT: f32 = 26.;
    /// The selected or hovered row's background is 24 tall inside the 26 px pitch.
    pub const TREE_ROW_INSET: f32 = 1.;
    pub const TREE_ROW_RADIUS: f32 = 4.;
    pub const TREE_ROOT_PAD_LEFT: f32 = 14.;
    pub const TREE_PAD_LEFT: f32 = 5.;
    pub const TREE_INDENT: f32 = 16.;
    pub const TREE_ICON_GAP: f32 = 5.;
    /// The editor island's tab row: tabs 28 tall in a 40 px band.
    pub const EDITOR_TAB_BAR_HEIGHT: f32 = 40.;
    pub const EDITOR_TAB_HEIGHT: f32 = 28.;
    pub const EDITOR_TAB_RADIUS: f32 = 6.;
    /// The selected tab's border is 2 device px at 1.5, i.e. 1.33 logical (Windows probe).
    pub const EDITOR_TAB_BORDER: f32 = 4. / 3.;
    /// Inside the tab, after its 1.33 border: 4, icon 16, 5, title, 1, close 16, 2
    /// (Windows probe, glyph extents: the selected tab is 93.3 wide around a 47.3
    /// px title, its close glyph 0.7 after the title's ink).
    pub const EDITOR_TAB_PAD_LEFT: f32 = 4.;
    pub const EDITOR_TAB_PAD_RIGHT: f32 = 2.;
    pub const EDITOR_TAB_GAP: f32 = 4.;
    pub const EDITOR_TAB_ICON_GAP: f32 = 5.;
    pub const EDITOR_TAB_CLOSE_GAP: f32 = 1.;
    /// A modified tab: a 4 px dot in an 8 px box between the title and the close box (batch 3).
    pub const EDITOR_TAB_DOT: f32 = 4.;
    pub const EDITOR_TAB_DOT_BOX: f32 = 8.;
    /// An overflowing tab row shows a 16 px chevron box at either end (batch 3).
    pub const EDITOR_TAB_SCROLL_BOX: f32 = 16.;
    /// A dragged tab's ghost: the tab inside a 2 px ring (batch 3).
    pub const EDITOR_TAB_DRAG_RING: f32 = 2.;
    /// The gutter: line numbers right-aligned at `GUTTER_NUMBER_RIGHT`, text from `GUTTER_WIDTH`.
    pub const GUTTER_NUMBER_RIGHT: f32 = 36.;
    pub const GUTTER_WIDTH: f32 = 62.;
    pub const EDITOR_FONT_SIZE: f32 = 13.;
    /// 33 device px at 1.5 (Windows probe); 60 at 2.76 (KDE probe) agrees with 22.
    pub const EDITOR_LINE_HEIGHT: f32 = 22.;
    /// Lines of empty space the editor scrolls past its end: Fleet's thumb for a
    /// 38-line file in a 696 px view was 523 px, which fits 4 lines (Windows probe).
    pub const EDITOR_OVERSCROLL_LINES: f32 = 4.;
    pub const CARET_WIDTH: f32 = 2.;
    /// Batch 5: each selected row is its own rounded rectangle; an empty
    /// selected row shows a short stub at the text start.
    pub const SELECTION_RADIUS: f32 = 3.33;
    pub const SELECTION_EMPTY_LINE_WIDTH: f32 = 3.33;
    /// The identifier-under-caret box is 0.67 in from the line's top and bottom
    /// with about 2 px of rounding (batch 5).
    pub const OCCURRENCE_INSET: f32 = 0.67;
    pub const OCCURRENCE_RADIUS: f32 = 2.;
    /// The caret-line and search marks on the scrollbar: 12 wide, 2 tall,
    /// against the island's right edge (batch 5).
    pub const SCROLLBAR_MARK_WIDTH: f32 = 12.;
    pub const SCROLLBAR_MARK_HEIGHT: f32 = 2.;
    /// The gutter's fold chevron box starts 40 from the island's left edge and
    /// the breakpoint hint circle (10 px) at 21.3 (batch 5).
    pub const FOLD_CHEVRON_X: f32 = 40.;
    pub const BREAKPOINT_HINT_X: f32 = 21.3;
    pub const BREAKPOINT_HINT_SIZE: f32 = 10.;
    /// The shadow under sticky lines (batch 5, estimate).
    pub const STICKY_SHADOW: f32 = 3.;
    /// The fold chevron's box is 16 (`ICON_SIZE`); the folded mark `{…}` is 3
    /// character cells wide with a radius of about 3 (batch 5).
    pub const FOLDED_MARK_CELLS: f32 = 3.;
    pub const FOLDED_MARK_RADIUS: f32 = 3.;
    /// The find bar (batch 5): a 416.67 × 28 field 16 from the island's right
    /// edge and 12 below the tab row; a 1.33 focus border inside a 2 px focus
    /// outline; the replace field 6 below it. Inside the field, from the left:
    /// text at 6; from the right: 8 pad, the 16 px close box, 8, a separator,
    /// 6, three 20 px toggles 2 apart, 7, a separator, 8, the ↓ and ↑ boxes 6
    /// apart, 8, the match counter. The chevron that opens the replace row sits
    /// in a 16 px box 5.33 left of the field (its glyph 10 from the field).
    pub const FIND_BAR_WIDTH: f32 = 416.67;
    pub const FIND_BAR_HEIGHT: f32 = 28.;
    pub const FIND_BAR_RIGHT: f32 = 16.;
    pub const FIND_BAR_TOP: f32 = 12.;
    pub const FIND_BAR_ROW_GAP: f32 = 6.;
    pub const FIND_BAR_PAD_LEFT: f32 = 6.;
    pub const FIND_BAR_PAD_RIGHT: f32 = 8.;
    pub const FIND_BAR_CLOSE_GAP: f32 = 8.;
    pub const FIND_BAR_SEPARATOR_GAP_LEFT: f32 = 6.;
    pub const FIND_BAR_SEPARATOR_GAP_RIGHT: f32 = 7.;
    pub const FIND_BAR_TOGGLE: f32 = 20.;
    pub const FIND_BAR_TOGGLE_GAP: f32 = 2.;
    pub const FIND_BAR_ARROW_GAP: f32 = 6.;
    pub const FIND_BAR_COUNTER_GAP: f32 = 8.;
    pub const FIND_BAR_CHEVRON_GAP: f32 = 5.33;
    /// Estimates: the field's corner radius and its separators' height, and a
    /// text button's horizontal padding ("Replace", "Replace All").
    pub const INPUT_RADIUS: f32 = 4.;
    pub const INPUT_FOCUS_BORDER: f32 = 4. / 3.;
    pub const FIND_BAR_SEPARATOR_HEIGHT: f32 = 16.;
    pub const TEXT_BUTTON_PAD_X: f32 = 8.;
    /// Soft-wrapped continuation rows start at the text origin (estimate: not
    /// probed; batch 5 left soft wrap, whitespace and the guides for a scene each).
    pub const WRAP_INDENT: f32 = 0.;
    /// A centred editor (`editor.centerEditorHorizontally`) keeps this many
    /// columns of text width (estimate).
    pub const CENTERED_COLUMNS: f32 = 120.;
    /// The caret blinks on a 500 ms half period (estimate; harness G).
    pub const CARET_BLINK_MS: u64 = 500;
    /// The thumb sits flush with the island's right edge (Windows probe).
    pub const SCROLLBAR_THUMB: f32 = 6.;
    pub const SCROLLBAR_THUMB_MIN: f32 = 24.;
    /// The popup's own scrollbar is narrower (batch 2).
    pub const POPUP_SCROLLBAR_THUMB: f32 = 4.67;
    pub const UI_FONT_SIZE: f32 = 13.;
    /// 42 device px at 1.5 (Windows probe).
    pub const STATUS_BAR_HEIGHT: f32 = 28.;
    pub const STATUS_PAD_X: f32 = 8.;
    /// The right group ends 12 px from the window edge: margin 8 + this.
    pub const STATUS_PAD_RIGHT: f32 = 4.;
    pub const STATUS_GAP: f32 = 12.;
    /// A popup (the `+` tool chooser, batch 2): 1 px border, 6 px padding, a 24 px
    /// search field, 6, a separator, 6, 28 px rows with an 8 px left pad, a 16 px
    /// icon and a 4 px gap. Width 166, height capped at 316.67 (estimate: fixed).
    pub const POPUP_RADIUS: f32 = 6.;
    pub const POPUP_PAD: f32 = 6.;
    pub const POPUP_FIELD_HEIGHT: f32 = 24.;
    pub const POPUP_ROW_HEIGHT: f32 = 28.;
    pub const POPUP_ROW_PAD_X: f32 = 8.;
    pub const POPUP_ROW_ICON_GAP: f32 = 4.;
    pub const POPUP_ROW_RADIUS: f32 = 4.;
    pub const POPUP_WIDTH: f32 = 166.;
    pub const POPUP_MAX_HEIGHT: f32 = 316.67;
    /// The popup opens 2 px below the tab row, its left edge on the `+` glyph's
    /// left edge, 8 px into the 28 px box (batch 2: popup at 233.3, glyph at 234).
    pub const POPUP_OFFSET_Y: f32 = 2.;
    pub const POPUP_OFFSET_X: f32 = 8.;
    /// Estimate for the popup's shadow (batch 2: about 8 px, heavier below).
    pub const POPUP_SHADOW_BLUR: f32 = 8.;
    pub const POPUP_SHADOW_OFFSET_Y: f32 = 2.;
    /// A tooltip (batch 3): padding 8, 13 px text on an 18 px line, radius 4
    /// (estimate), at most 600 wide (estimate), 2 px below the hovered tab.
    pub const TOOLTIP_PAD_X: f32 = 8.;
    pub const TOOLTIP_PAD_Y: f32 = 8.;
    /// The tooltip's top sits 16 px below the pointer (batch 3: pointer at 56, box at 72).
    pub const TOOLTIP_OFFSET_Y: f32 = 16.;
    pub const TOOLTIP_LINE: f32 = 18.;
    pub const TOOLTIP_RADIUS: f32 = 4.;
    /// Fleet's tab tooltip wrapped at 616 wide (batch 3).
    pub const TOOLTIP_MAX_WIDTH: f32 = 616.;
    /// The Goto panel (batch 6, `docs/fleet-probe.md` sixth pass): 680 wide,
    /// centred, its top at 64.67 and its bottom at 743.33 in an 800 window when
    /// it holds a preview (`GOTO_MAX_HEIGHT`), else as tall as its rows plus 2.
    /// Inside, 8 px from the edges: the tab chips (28 tall, 5.33 below the
    /// top), 10, the field (28 with the 2 px ring outside), 6, the filter
    /// buttons (24), 8, a 26 px header, then 26 px rows with a 24 px wash;
    /// without the buttons the rows start 10 below the ring. The preview box
    /// sits at the bottom, 6.67 above the popup's edge, taking half of what is
    /// left under the list's top once the 8 px gap is taken out (Fleet's
    /// `splitRatioVertical` 0.5; 274.67 tall in an 800 window).
    pub const GOTO_TOP: f32 = 64.67;
    pub const GOTO_MAX_HEIGHT: f32 = 678.67;
    pub const GOTO_WIDTH: f32 = 680.;
    pub const GOTO_PREVIEW_SHARE: f32 = 0.5;
    pub const GOTO_FIELD_HEIGHT: f32 = 28.;
    pub const GOTO_ROW_HEIGHT: f32 = 24.;
    pub const GOTO_TAB_HEIGHT: f32 = 28.;
    pub const GOTO_TAB_PAD_X: f32 = 8.;
    pub const GOTO_TAB_GAP: f32 = 6.;
    pub const GOTO_TAB_TOP: f32 = 5.33;
    pub const GOTO_TAB_FIELD_GAP: f32 = 10.;
    pub const GOTO_PAD: f32 = 8.;
    pub const GOTO_CHIP_HEIGHT: f32 = 24.;
    pub const GOTO_CHIP_GAP: f32 = 10.67;
    pub const GOTO_CHIP_TOP_GAP: f32 = 6.;
    pub const GOTO_BLOCK_GAP: f32 = 8.;
    pub const GOTO_LIST_FIELD_GAP: f32 = 10.;
    pub const GOTO_HEADER_HEIGHT: f32 = 26.;
    pub const GOTO_ROW_PITCH: f32 = 26.;
    pub const GOTO_PREVIEW_BOTTOM_PAD: f32 = 6.67;
    pub const GOTO_BOTTOM_PAD: f32 = 2.;
    pub const GOTO_ICON_BUTTON: f32 = 24.;
    /// The Tools list: 8 above its separator, 4 below it (batch 6).
    pub const GOTO_SEPARATOR_ABOVE: f32 = 8.;
    pub const GOTO_SEPARATOR_BELOW: f32 = 4.;
    /// The Text Search tab (batch 7): the folder and mask fields 6 below the
    /// ring, 28 tall, the mask 164 wide, 8 apart; a 28 px count row; the
    /// Replace and Replace All buttons 8.67 apart, 11.33 above the bottom.
    pub const SEARCH_MASK_WIDTH: f32 = 164.;
    pub const SEARCH_COUNT_ROW: f32 = 28.;
    pub const GOTO_BUTTONS_PAD: f32 = 11.33;
    pub const GOTO_BUTTON_GAP: f32 = 8.67;
    /// Estimates left in the Goto panel: the preview box's radius and its
    /// breadcrumb footer, the Symbols in File tree's indent per level.
    pub const GOTO_PREVIEW_RADIUS: f32 = 6.;
    pub const GOTO_PREVIEW_FOOTER: f32 = 24.;
    pub const GOTO_TREE_INDENT: f32 = 24.;
    /// A key-cap hint (`shortcut.*`): 20 tall, 4 px padding, radius 3 (estimate).
    pub const SHORTCUT_HEIGHT: f32 = 20.;
    pub const SHORTCUT_PAD_X: f32 = 4.;
    pub const SHORTCUT_RADIUS: f32 = 3.;
    /// Context menus (batch 7): the popup's top-left corner at the pointer,
    /// 6 px padding, 28 px rows with 8 px row padding, a separator block of
    /// 12 (the 1 px line with 5.5 above and below), the shortcut in
    /// `text.secondary` right-aligned; the width follows the longest row
    /// (the tree's menu 230.67, the editor's 232, a tab's 170.67, the main
    /// menu 139), with 24 between a label and its shortcut (estimate).
    pub const MENU_ROW_HEIGHT: f32 = 28.;
    pub const MENU_ROW_PAD_X: f32 = 8.;
    pub const MENU_MIN_WIDTH: f32 = 140.;
    pub const MENU_SHORTCUT_GAP: f32 = 24.;
    pub const MENU_SEPARATOR_PAD: f32 = 5.5;
    /// Confirm dialogs (batch 7, estimates): 420 wide, 16 px padding, 24 px
    /// buttons 8 apart, the title 13 px semibold on a 20 line, 8 to the message.
    pub const DIALOG_WIDTH: f32 = 420.;
    pub const DIALOG_PAD: f32 = 16.;
    pub const DIALOG_BUTTON_GAP: f32 = 8.;
    pub const DIALOG_TITLE_LINE: f32 = 20.;
    pub const DIALOG_TITLE_GAP: f32 = 8.;
    pub const DIALOG_BUTTONS_GAP: f32 = 16.;
    /// Toasts (batch 7, estimates): 360 wide in the window's bottom-right
    /// corner, 12 px padding, 8 apart, 16 from the edges, shown 5 s.
    pub const TOAST_WIDTH: f32 = 360.;
    pub const TOAST_PAD: f32 = 12.;
    pub const TOAST_GAP: f32 = 8.;
    pub const TOAST_MARGIN: f32 = 16.;
    pub const TOAST_RADIUS: f32 = 6.;
    pub const TOAST_MS: u64 = 5000;
    /// Notifications tool rows: a 16 px icon, 8 gap, title and message on 18 px
    /// lines, 10 px padding (estimates).
    pub const NOTIFICATION_PAD: f32 = 10.;
    pub const NOTIFICATION_ICON_GAP: f32 = 8.;
    pub const NOTIFICATION_LINE: f32 = 18.;
    /// The Search tool (batch 7 and 8 do not cover it; estimates): a 28 px
    /// field like the find bar's, 6 px gaps, 24 px result rows indented 16 per
    /// level, the match line in the editor face.
    pub const SEARCH_FIELD_HEIGHT: f32 = 28.;
    pub const SEARCH_GAP: f32 = 6.;
    pub const SEARCH_ROW_HEIGHT: f32 = 24.;
    pub const SEARCH_INDENT: f32 = 16.;
    /// The Files tree's keyboard-highlight rectangle (`files.keyboardNavigation`
    /// highlight): 1 px of `tree.focusBorder` (estimate).
    pub const TREE_HIGHLIGHT_BORDER: f32 = 1.;
}

macro_rules! scaled_metrics {
    ($($field:ident = $konst:ident),* $(,)?) => {
        /// [`metrics`] at the current zoom level, the one scale factor Fleet's
        /// `zoomLevel` applies to every panel. Kept as a global so a view can
        /// read it in `render`.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Metrics {
            pub zoom: f32,
            $(pub $field: f32,)*
        }

        impl Metrics {
            pub fn at(zoom: f32) -> Metrics {
                Metrics { zoom, $($field: metrics::$konst * zoom,)* }
            }
        }
    };
}

scaled_metrics!(
    toolbar_height = TOOLBAR_HEIGHT,
    toolbar_button = TOOLBAR_BUTTON,
    toolbar_gap = TOOLBAR_GAP,
    toolbar_pad_x = TOOLBAR_PAD_X,
    toolbar_separator_margin = TOOLBAR_SEPARATOR_MARGIN,
    logo_size = LOGO_SIZE,
    icon_size = ICON_SIZE,
    caption_button_width = CAPTION_BUTTON_WIDTH,
    caption_glyph = CAPTION_GLYPH,
    title_name_max_width = TITLE_NAME_MAX_WIDTH,
    title_branch_gap = TITLE_BRANCH_GAP,
    title_branch_icon_gap = TITLE_BRANCH_ICON_GAP,
    island_margin = ISLAND_MARGIN,
    island_radius = ISLAND_RADIUS,
    left_panel_width = LEFT_PANEL_WIDTH,
    right_panel_width = RIGHT_PANEL_WIDTH,
    bottom_panel_height = BOTTOM_PANEL_HEIGHT,
    panel_min_width = PANEL_MIN_WIDTH,
    panel_min_height = PANEL_MIN_HEIGHT,
    panel_tab_height = PANEL_TAB_HEIGHT,
    panel_tab_radius = PANEL_TAB_RADIUS,
    panel_tab_pad_x = PANEL_TAB_PAD_X,
    panel_tab_selected_pad_x = PANEL_TAB_SELECTED_PAD_X,
    panel_tab_gap = PANEL_TAB_GAP,
    panel_tab_plus_gap = PANEL_TAB_PLUS_GAP,
    panel_header_pad_top = PANEL_HEADER_PAD_TOP,
    panel_header_pad_bottom = PANEL_HEADER_PAD_BOTTOM,
    panel_pad_x = PANEL_PAD_X,
    empty_panel_icon = EMPTY_PANEL_ICON,
    empty_panel_gap_icon = EMPTY_PANEL_GAP_ICON,
    empty_panel_title_size = EMPTY_PANEL_TITLE_SIZE,
    empty_panel_title_line = EMPTY_PANEL_TITLE_LINE,
    empty_panel_gap_title = EMPTY_PANEL_GAP_TITLE,
    empty_panel_text_line = EMPTY_PANEL_TEXT_LINE,
    empty_panel_gap_text = EMPTY_PANEL_GAP_TEXT,
    button_height = BUTTON_HEIGHT,
    button_pad_x = BUTTON_PAD_X,
    button_icon_gap = BUTTON_ICON_GAP,
    button_chevron_box = BUTTON_CHEVRON_BOX,
    button_radius = BUTTON_RADIUS,
    focus_outline = FOCUS_OUTLINE,
    tree_row_height = TREE_ROW_HEIGHT,
    tree_row_inset = TREE_ROW_INSET,
    tree_row_radius = TREE_ROW_RADIUS,
    tree_root_pad_left = TREE_ROOT_PAD_LEFT,
    tree_pad_left = TREE_PAD_LEFT,
    tree_indent = TREE_INDENT,
    tree_icon_gap = TREE_ICON_GAP,
    editor_tab_bar_height = EDITOR_TAB_BAR_HEIGHT,
    editor_tab_height = EDITOR_TAB_HEIGHT,
    editor_tab_radius = EDITOR_TAB_RADIUS,
    editor_tab_border = EDITOR_TAB_BORDER,
    editor_tab_pad_left = EDITOR_TAB_PAD_LEFT,
    editor_tab_pad_right = EDITOR_TAB_PAD_RIGHT,
    editor_tab_gap = EDITOR_TAB_GAP,
    editor_tab_icon_gap = EDITOR_TAB_ICON_GAP,
    editor_tab_close_gap = EDITOR_TAB_CLOSE_GAP,
    editor_tab_dot = EDITOR_TAB_DOT,
    editor_tab_dot_box = EDITOR_TAB_DOT_BOX,
    editor_tab_scroll_box = EDITOR_TAB_SCROLL_BOX,
    editor_tab_drag_ring = EDITOR_TAB_DRAG_RING,
    gutter_number_right = GUTTER_NUMBER_RIGHT,
    gutter_width = GUTTER_WIDTH,
    editor_font_size = EDITOR_FONT_SIZE,
    editor_line_height = EDITOR_LINE_HEIGHT,
    caret_width = CARET_WIDTH,
    selection_radius = SELECTION_RADIUS,
    selection_empty_line_width = SELECTION_EMPTY_LINE_WIDTH,
    occurrence_inset = OCCURRENCE_INSET,
    occurrence_radius = OCCURRENCE_RADIUS,
    scrollbar_mark_width = SCROLLBAR_MARK_WIDTH,
    scrollbar_mark_height = SCROLLBAR_MARK_HEIGHT,
    fold_chevron_x = FOLD_CHEVRON_X,
    breakpoint_hint_x = BREAKPOINT_HINT_X,
    breakpoint_hint_size = BREAKPOINT_HINT_SIZE,
    sticky_shadow = STICKY_SHADOW,
    folded_mark_cells = FOLDED_MARK_CELLS,
    folded_mark_radius = FOLDED_MARK_RADIUS,
    find_bar_width = FIND_BAR_WIDTH,
    find_bar_height = FIND_BAR_HEIGHT,
    find_bar_right = FIND_BAR_RIGHT,
    find_bar_top = FIND_BAR_TOP,
    find_bar_row_gap = FIND_BAR_ROW_GAP,
    find_bar_pad_left = FIND_BAR_PAD_LEFT,
    find_bar_pad_right = FIND_BAR_PAD_RIGHT,
    find_bar_close_gap = FIND_BAR_CLOSE_GAP,
    find_bar_separator_gap_left = FIND_BAR_SEPARATOR_GAP_LEFT,
    find_bar_separator_gap_right = FIND_BAR_SEPARATOR_GAP_RIGHT,
    find_bar_toggle = FIND_BAR_TOGGLE,
    find_bar_toggle_gap = FIND_BAR_TOGGLE_GAP,
    find_bar_arrow_gap = FIND_BAR_ARROW_GAP,
    find_bar_counter_gap = FIND_BAR_COUNTER_GAP,
    find_bar_chevron_gap = FIND_BAR_CHEVRON_GAP,
    input_radius = INPUT_RADIUS,
    input_focus_border = INPUT_FOCUS_BORDER,
    find_bar_separator_height = FIND_BAR_SEPARATOR_HEIGHT,
    text_button_pad_x = TEXT_BUTTON_PAD_X,
    wrap_indent = WRAP_INDENT,
    centered_columns = CENTERED_COLUMNS,
    scrollbar_thumb = SCROLLBAR_THUMB,
    scrollbar_thumb_min = SCROLLBAR_THUMB_MIN,
    popup_scrollbar_thumb = POPUP_SCROLLBAR_THUMB,
    ui_font_size = UI_FONT_SIZE,
    status_bar_height = STATUS_BAR_HEIGHT,
    status_pad_x = STATUS_PAD_X,
    status_pad_right = STATUS_PAD_RIGHT,
    status_gap = STATUS_GAP,
    popup_radius = POPUP_RADIUS,
    popup_pad = POPUP_PAD,
    popup_field_height = POPUP_FIELD_HEIGHT,
    popup_row_height = POPUP_ROW_HEIGHT,
    popup_row_pad_x = POPUP_ROW_PAD_X,
    popup_row_icon_gap = POPUP_ROW_ICON_GAP,
    popup_row_radius = POPUP_ROW_RADIUS,
    popup_width = POPUP_WIDTH,
    popup_max_height = POPUP_MAX_HEIGHT,
    popup_offset_y = POPUP_OFFSET_Y,
    popup_offset_x = POPUP_OFFSET_X,
    popup_shadow_blur = POPUP_SHADOW_BLUR,
    popup_shadow_offset_y = POPUP_SHADOW_OFFSET_Y,
    tooltip_pad_x = TOOLTIP_PAD_X,
    tooltip_pad_y = TOOLTIP_PAD_Y,
    tooltip_offset_y = TOOLTIP_OFFSET_Y,
    tooltip_line = TOOLTIP_LINE,
    tooltip_radius = TOOLTIP_RADIUS,
    tooltip_max_width = TOOLTIP_MAX_WIDTH,
    goto_top = GOTO_TOP,
    goto_max_height = GOTO_MAX_HEIGHT,
    goto_width = GOTO_WIDTH,
    goto_field_height = GOTO_FIELD_HEIGHT,
    goto_row_height = GOTO_ROW_HEIGHT,
    goto_tab_height = GOTO_TAB_HEIGHT,
    goto_tab_pad_x = GOTO_TAB_PAD_X,
    goto_tab_gap = GOTO_TAB_GAP,
    goto_pad = GOTO_PAD,
    goto_tab_top = GOTO_TAB_TOP,
    goto_tab_field_gap = GOTO_TAB_FIELD_GAP,
    goto_chip_height = GOTO_CHIP_HEIGHT,
    goto_chip_gap = GOTO_CHIP_GAP,
    goto_chip_top_gap = GOTO_CHIP_TOP_GAP,
    goto_block_gap = GOTO_BLOCK_GAP,
    goto_list_field_gap = GOTO_LIST_FIELD_GAP,
    goto_header_height = GOTO_HEADER_HEIGHT,
    goto_row_pitch = GOTO_ROW_PITCH,
    goto_preview_bottom_pad = GOTO_PREVIEW_BOTTOM_PAD,
    goto_bottom_pad = GOTO_BOTTOM_PAD,
    goto_icon_button = GOTO_ICON_BUTTON,
    goto_separator_above = GOTO_SEPARATOR_ABOVE,
    goto_separator_below = GOTO_SEPARATOR_BELOW,
    search_mask_width = SEARCH_MASK_WIDTH,
    search_count_row = SEARCH_COUNT_ROW,
    goto_buttons_pad = GOTO_BUTTONS_PAD,
    goto_button_gap = GOTO_BUTTON_GAP,
    goto_preview_radius = GOTO_PREVIEW_RADIUS,
    goto_preview_footer = GOTO_PREVIEW_FOOTER,
    goto_tree_indent = GOTO_TREE_INDENT,
    shortcut_height = SHORTCUT_HEIGHT,
    shortcut_pad_x = SHORTCUT_PAD_X,
    shortcut_radius = SHORTCUT_RADIUS,
    menu_row_height = MENU_ROW_HEIGHT,
    menu_row_pad_x = MENU_ROW_PAD_X,
    menu_min_width = MENU_MIN_WIDTH,
    menu_shortcut_gap = MENU_SHORTCUT_GAP,
    menu_separator_pad = MENU_SEPARATOR_PAD,
    dialog_width = DIALOG_WIDTH,
    dialog_pad = DIALOG_PAD,
    dialog_button_gap = DIALOG_BUTTON_GAP,
    dialog_title_line = DIALOG_TITLE_LINE,
    dialog_title_gap = DIALOG_TITLE_GAP,
    dialog_buttons_gap = DIALOG_BUTTONS_GAP,
    toast_width = TOAST_WIDTH,
    toast_pad = TOAST_PAD,
    toast_gap = TOAST_GAP,
    toast_margin = TOAST_MARGIN,
    toast_radius = TOAST_RADIUS,
    notification_pad = NOTIFICATION_PAD,
    notification_icon_gap = NOTIFICATION_ICON_GAP,
    notification_line = NOTIFICATION_LINE,
    search_field_height = SEARCH_FIELD_HEIGHT,
    search_gap = SEARCH_GAP,
    search_row_height = SEARCH_ROW_HEIGHT,
    search_indent = SEARCH_INDENT,
    tree_highlight_border = TREE_HIGHLIGHT_BORDER,
);

impl Global for Metrics {}

impl Metrics {
    /// The editor's text metrics for the settings: `editor.fontSize` times the
    /// zoom, and `editor.lineSpacing` times that, else Fleet's measured 22 at 13.
    pub fn editor_text(&self, settings: &oak_settings::Settings) -> (f32, f32) {
        let font_size = settings.editor_font_size() * self.zoom;
        let line_height = match settings.editor_line_spacing() {
            Some(spacing) => (font_size * spacing).round(),
            None => (font_size * metrics::EDITOR_LINE_HEIGHT / metrics::EDITOR_FONT_SIZE).round(),
        };
        (font_size, line_height)
    }
}

pub fn to_hsla(c: Color) -> Hsla {
    let (r, g, b, a) = c.to_f32();
    Rgba { r, g, b, a }.into()
}

/// A syntax attribute ready to paint: colour, weight and slant.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // the registry of what a theme offers; surfaces pick from it as they appear
pub struct Syntax {
    pub color: Option<Hsla>,
    pub background: Option<Hsla>,
    pub weight: FontWeight,
    pub italic: bool,
    pub underline: Option<Hsla>,
    /// `scrollbarMarkColor`: the mark this attribute leaves on the scrollbar.
    pub scrollbar_mark: Option<Hsla>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // the registry of what a theme offers; surfaces pick from it as they appear
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub ui_font: SharedString,
    pub mono_font: SharedString,

    // text.*
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub text_disabled: Hsla,
    pub text_positive: Hsla,
    pub text_dangerous: Hsla,
    pub text_bright: Hsla,
    /// `button.warning.background.default`: the warning hue (no `text.warning` key exists).
    pub warning: Hsla,
    // background.*, island.background, border, separator.default
    pub background_primary: Hsla,
    pub background_secondary: Hsla,
    pub island_background: Hsla,
    pub border: Hsla,
    pub border_focused: Hsla,
    pub focus_outline: Hsla,
    pub separator: Hsla,
    // icon.*
    pub icon_primary: Hsla,
    pub icon_secondary: Hsla,
    // ghostButton.off.background.* (the toolbar's icon buttons)
    pub ghost_hovered: Hsla,
    pub ghost_pressed: Hsla,
    // tab.*
    pub tab_bg_hovered: Hsla,
    pub tab_bg_selected: Hsla,
    pub tab_bg_selected_focused: Hsla,
    pub tab_border_selected: Hsla,
    pub tab_border_selected_focused: Hsla,
    pub tab_text: Hsla,
    pub tab_text_hovered: Hsla,
    pub tab_text_selected: Hsla,
    pub tab_close_icon: Hsla,
    pub tab_close_icon_hovered: Hsla,
    pub tab_drag_outline: Hsla,
    // listItem.* and tree.*
    pub list_bg_hovered: Hsla,
    pub list_bg_selected: Hsla,
    pub list_bg_focused: Hsla,
    pub list_text: Hsla,
    pub list_text_focused: Hsla,
    pub list_text_secondary: Hsla,
    pub tree_focus_border: Hsla,
    // editor.*
    pub editor_text: Hsla,
    pub editor_caret: Hsla,
    pub current_line: Hsla,
    pub current_line_focused: Hsla,
    pub line_number: Hsla,
    pub line_number_current: Hsla,
    pub whitespace_indicator: Hsla,
    /// `editor.foldIndicator.*`: the gutter chevron and its hover wash.
    pub fold_icon: Hsla,
    pub fold_icon_hovered: Hsla,
    pub fold_bg_hovered: Hsla,
    /// `editor.foldedMark.*`: the `{…}` box on a folded line.
    pub folded_mark_bg: Hsla,
    pub folded_mark_text: Hsla,
    /// `editor.currentLine.scrollbarMark`.
    pub current_line_mark: Hsla,
    pub selection: Hsla,
    pub selection_focused: Hsla,
    pub search_match_bg: Hsla,
    pub brace_match_bg: Hsla,
    /// `identifier.underCaret`: the box on the identifier at the caret and its
    /// other occurrences, and its scrollbar mark.
    pub occurrence_bg: Hsla,
    pub occurrence_mark: Hsla,
    /// `editor.indentGuide` and `.current`.
    pub indent_guide: Hsla,
    pub indent_guide_current: Hsla,
    /// `editor.search.results`: the box on a find match and its scrollbar mark.
    pub search_result_bg: Hsla,
    pub search_result_mark: Hsla,
    // scrollbar.*
    pub scrollbar_thumb: Hsla,
    pub scrollbar_thumb_hovered: Hsla,
    pub scrollbar_thumb_pressed: Hsla,
    pub scrollbar_track_hovered: Hsla,
    // statusBar.*, popup.*, tooltip.*, shadow.*
    pub status_text: Hsla,
    pub popup_bg: Hsla,
    pub popup_border: Hsla,
    pub popup_separator: Hsla,
    pub popup_text: Hsla,
    pub tooltip_bg: Hsla,
    pub tooltip_border: Hsla,
    pub tooltip_text: Hsla,
    pub tooltip_text_secondary: Hsla,
    pub shadow_medium: Hsla,
    // button.primary.*, button.secondary.*, toggleButton.on.*
    pub button_primary_bg: Hsla,
    pub button_primary_bg_hovered: Hsla,
    pub button_primary_text: Hsla,
    pub button_secondary_bg: Hsla,
    pub button_secondary_bg_hovered: Hsla,
    pub button_secondary_bg_pressed: Hsla,
    pub button_secondary_border: Hsla,
    pub button_secondary_text: Hsla,
    pub toggle_on_bg: Hsla,
    pub toggle_on_bg_hovered: Hsla,
    pub toggle_on_border: Hsla,
    pub toggle_on_text: Hsla,
    // toggleButton.off.*
    pub toggle_off_bg: Hsla,
    pub toggle_off_bg_hovered: Hsla,
    pub toggle_off_bg_pressed: Hsla,
    pub toggle_off_border: Hsla,
    pub toggle_off_text: Hsla,
    pub toggle_off_text_hovered: Hsla,
    // ghostButton.off.text.*
    pub ghost_text: Hsla,
    pub ghost_text_hovered: Hsla,
    // inputField.*
    pub input_bg: Hsla,
    pub input_bg_hovered: Hsla,
    pub input_border: Hsla,
    pub input_border_hovered: Hsla,
    pub input_focus_border: Hsla,
    pub input_focus_outline: Hsla,
    pub input_text: Hsla,
    pub input_hint: Hsla,
    pub input_caret: Hsla,
    pub input_selection: Hsla,
    // search.*
    pub search_counter_bg: Hsla,
    /// `search.match.text`: the text of a match in a Search tool result row
    /// (`search_match_bg` above is its background).
    pub search_match_text: Hsla,
    pub smart_mode_enabled: Hsla,
    pub link: Hsla,
    // shortcut.*: the key-cap hints in the Goto panel and menus
    pub shortcut_text: Hsla,
    pub shortcut_bg: Hsla,
    // notification.*, toast.*
    pub notification_bg: Hsla,
    pub notification_bg_unread: Hsla,
    pub notification_text: Hsla,
    pub notification_text_secondary: Hsla,
    pub notification_separator: Hsla,
    pub toast_bg: Hsla,
    pub toast_border: Hsla,
    pub toast_text: Hsla,
    // dragAndDrop.*: the target a dragged file is over
    pub drag_bg: Hsla,
    pub drag_border: Hsla,
    pub drag_border_highlight: Hsla,
    // button.dangerous.*
    pub button_dangerous_bg: Hsla,
    pub button_dangerous_bg_hovered: Hsla,
    pub button_dangerous_text: Hsla,
    // popup.goto.background, popup.footer.*
    pub popup_goto_bg: Hsla,
    pub popup_footer_bg: Hsla,
    pub popup_footer_text: Hsla,

    syntax: HashMap<String, Syntax>,
}

impl Global for Theme {}

/// Where a theme comes from: a bundled name or a Fleet-format file.
#[derive(Clone, Debug)]
pub enum ThemeSource {
    Bundled(String),
    File(PathBuf),
}

impl ThemeSource {
    /// `dark`, `dark_purple`, `light`, or a path to a JSON file. A bare name
    /// that is not bundled is looked for in Fleet's own themes folder.
    pub fn parse(spec: &str) -> ThemeSource {
        if assets::THEMES.iter().any(|(n, _)| *n == spec) {
            return ThemeSource::Bundled(spec.to_string());
        }
        let path = Path::new(spec);
        if path.exists() {
            return ThemeSource::File(path.to_path_buf());
        }
        for dir in theme_dirs() {
            let candidate = dir.join(format!("{spec}.json"));
            if candidate.exists() {
                return ThemeSource::File(candidate);
            }
        }
        ThemeSource::Bundled(spec.to_string())
    }
}

/// The folders a theme name is looked up in after the bundled set: Oak's own
/// `themes/` and then Fleet's, so a theme installed for Fleet is usable by name.
fn theme_dirs() -> Vec<PathBuf> {
    [
        oak_settings::oak_config_dir(),
        oak_settings::fleet_config_dir(),
    ]
    .into_iter()
    .flatten()
    .map(|d| d.join("themes"))
    .collect()
}

impl Theme {
    pub fn load(source: &ThemeSource) -> Result<Theme> {
        let json = match source {
            ThemeSource::Bundled(name) => assets::THEMES
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, json)| json.to_string())
                .ok_or_else(|| anyhow!("no bundled theme named {name:?}"))?,
            ThemeSource::File(path) => {
                std::fs::read_to_string(path).with_context(|| path.display().to_string())?
            }
        };
        let parsed =
            oak_theme::Theme::parse(&json).with_context(|| format!("loading theme {source:?}"))?;
        for warning in &parsed.warnings {
            eprintln!("oak: theme {source:?}: skipped {warning}");
        }
        Ok(Theme::from_parsed(parsed))
    }

    pub fn from_parsed(t: oak_theme::Theme) -> Theme {
        let dark = t.is_dark();
        let fallback_text = if dark {
            Color::rgb(0xE0, 0xE1, 0xE4)
        } else {
            Color::rgb(0x18, 0x19, 0x1B)
        };
        let fallback_bg = if dark {
            Color::rgb(0x09, 0x09, 0x09)
        } else {
            Color::rgb(0xFF, 0xFF, 0xFF)
        };
        let c = |key: &str, fallback: Color| to_hsla(t.color_or(key, fallback));
        let first = |keys: &[&str], fallback: Color| {
            to_hsla(keys.iter().find_map(|k| t.color(k)).unwrap_or(fallback))
        };
        let text_primary = t.color_or("text.primary", fallback_text);
        let bg_primary = t.color_or("background.primary", fallback_bg);
        let bg_secondary = t.color_or("background.secondary", bg_primary);
        let transparent = Color::TRANSPARENT;

        let attr = |key: &str| -> Option<Syntax> {
            let a = t.attributes_cascading(key)?;
            Some(Syntax {
                color: a.foreground.map(to_hsla),
                background: a.background.map(to_hsla),
                weight: match a.weight {
                    Some(oak_theme::FontWeight::Bold) => FontWeight::BOLD,
                    Some(oak_theme::FontWeight::SemiBold) => FontWeight::SEMIBOLD,
                    Some(oak_theme::FontWeight::Medium) => FontWeight::MEDIUM,
                    _ => FontWeight::NORMAL,
                },
                italic: a.italic,
                underline: a.decoration.and_then(|d| d.color.map(to_hsla)),
                scrollbar_mark: a.scrollbar_mark.map(to_hsla),
            })
        };
        let syntax = t
            .attribute_keys()
            .filter_map(|k| Some((k.to_string(), attr(k)?)))
            .collect();
        let selection_attr = |key: &str| t.attributes(key).and_then(|a| a.background).map(to_hsla);

        Theme {
            name: t.name.clone(),
            dark,
            ui_font: UI_FONT.into(),
            mono_font: MONO_FONT.into(),
            text_primary: to_hsla(text_primary),
            text_secondary: c("text.secondary", text_primary.with_alpha(0x85)),
            text_tertiary: c("text.tertiary", text_primary.with_alpha(0x59)),
            text_disabled: c("text.disabled", text_primary.with_alpha(0x59)),
            text_positive: c("text.positive", Color::rgb(0x40, 0x9D, 0x78)),
            text_dangerous: c("text.dangerous", Color::rgb(0xEC, 0x5D, 0x6F)),
            text_bright: c("text.bright", Color::rgb(0xFF, 0xFF, 0xFF)),
            warning: c(
                "button.warning.background.default",
                Color::rgb(0xD9, 0x8B, 0x2D),
            ),
            background_primary: to_hsla(bg_primary),
            background_secondary: to_hsla(bg_secondary),
            island_background: c("island.background", bg_secondary),
            border: c("border", text_primary.with_alpha(0x21)),
            border_focused: c("border.focused", Color::rgb(0x2A, 0x7D, 0xEB)),
            focus_outline: c("focusOutline", Color::rgb(0x22, 0x42, 0x71)),
            separator: c("separator.default", text_primary.with_alpha(0x26)),
            icon_primary: c("icon.primary", text_primary),
            icon_secondary: c("icon.secondary", Color::rgb(0x7A, 0x7F, 0x86)),
            ghost_hovered: c(
                "ghostButton.off.background.hovered",
                text_primary.with_alpha(0x16),
            ),
            ghost_pressed: c(
                "ghostButton.off.background.pressed",
                text_primary.with_alpha(0x21),
            ),
            tab_bg_hovered: c("tab.background.hovered", text_primary.with_alpha(0x14)),
            tab_bg_selected: c("tab.background.selected", text_primary.with_alpha(0x14)),
            tab_bg_selected_focused: c(
                "tab.background.selectedFocused",
                Color::rgb(0x08, 0x70, 0xE4).with_alpha(0x40),
            ),
            tab_border_selected: c("tab.border.selected", text_primary.with_alpha(0x14)),
            tab_border_selected_focused: c(
                "tab.border.selectedFocused",
                Color::rgb(0x08, 0x70, 0xE4).with_alpha(0x80),
            ),
            tab_text: c("tab.text.default", text_primary),
            tab_text_hovered: c("tab.text.hovered", text_primary),
            tab_text_selected: c("tab.text.selectedFocused", Color::rgb(0xFF, 0xFF, 0xFF)),
            tab_close_icon: c("tab.closeIcon.default", text_primary.with_alpha(0x4D)),
            tab_close_icon_hovered: c("tab.closeIcon.hovered", text_primary.with_alpha(0xB2)),
            tab_drag_outline: c("tab.focusOutline.dragAndDrop", Color::rgb(0x2A, 0x7D, 0xEB)),
            list_bg_hovered: c("listItem.background.hovered", text_primary.with_alpha(0x0B)),
            list_bg_selected: c(
                "listItem.background.selected",
                text_primary.with_alpha(0x21),
            ),
            list_bg_focused: c(
                "listItem.background.focused",
                Color::rgb(0x08, 0x70, 0xE4).with_alpha(0x66),
            ),
            list_text: c("listItem.text.default", text_primary),
            list_text_focused: c("listItem.text.focused", text_primary),
            list_text_secondary: c("listItem.text.secondary", text_primary.with_alpha(0x85)),
            tree_focus_border: c("tree.focusBorder", Color::rgb(0x2A, 0x7D, 0xEB)),
            editor_text: c("editor.text", text_primary),
            editor_caret: c("editor.caret.background", text_primary),
            current_line: c(
                "editor.currentLine.background.default",
                text_primary.with_alpha(0x1A),
            ),
            current_line_focused: c(
                "editor.currentLine.background.focused",
                Color::rgb(0x08, 0x70, 0xE4).with_alpha(0x33),
            ),
            line_number: c("editor.lineNumber.default", text_primary.with_alpha(0x4D)),
            line_number_current: c("editor.lineNumber.current", text_primary),
            whitespace_indicator: c("editor.whitespaceIndicator", text_primary.with_alpha(0x4D)),
            fold_icon: c(
                "editor.foldIndicator.icon.default",
                Color::rgb(0x7A, 0x7F, 0x86),
            ),
            fold_icon_hovered: c(
                "editor.foldIndicator.icon.hovered",
                Color::rgb(0x7A, 0x7F, 0x86),
            ),
            fold_bg_hovered: c(
                "editor.foldIndicator.background.hovered",
                text_primary.with_alpha(0x16),
            ),
            folded_mark_bg: c(
                "editor.foldedMark.background",
                text_primary.with_alpha(0x1B),
            ),
            folded_mark_text: c("editor.foldedMark.text", Color::rgb(0x89, 0x8E, 0x94)),
            current_line_mark: c("editor.currentLine.scrollbarMark", text_primary),
            selection: selection_attr("editor.selection")
                .unwrap_or_else(|| to_hsla(Color::rgb(0x4C, 0x51, 0x57))),
            selection_focused: selection_attr("editor.selection.focused")
                .unwrap_or_else(|| to_hsla(Color::rgb(0x22, 0x50, 0x90))),
            search_match_bg: c("search.match.background", Color::rgb(0x6C, 0x4B, 0x18)),
            occurrence_bg: selection_attr("identifier.underCaret")
                .unwrap_or_else(|| to_hsla(text_primary.with_alpha(0x21))),
            occurrence_mark: attr("identifier.underCaret")
                .and_then(|a| a.scrollbar_mark)
                .unwrap_or_else(|| to_hsla(text_primary.with_alpha(0x43))),
            indent_guide: attr("editor.indentGuide")
                .and_then(|a| a.color)
                .unwrap_or_else(|| to_hsla(text_primary.with_alpha(0x21))),
            indent_guide_current: attr("editor.indentGuide.current")
                .and_then(|a| a.color)
                .unwrap_or_else(|| to_hsla(text_primary.with_alpha(0x43))),
            search_result_bg: selection_attr("editor.search.results")
                .unwrap_or_else(|| to_hsla(Color::rgb(0x9F, 0x68, 0x0C).with_alpha(0x80))),
            search_result_mark: attr("editor.search.results")
                .and_then(|a| a.scrollbar_mark)
                .unwrap_or_else(|| to_hsla(Color::rgb(0xD4, 0x95, 0x50))),
            brace_match_bg: selection_attr("editor.brace.match")
                .unwrap_or_else(|| to_hsla(Color::rgb(0x2A, 0x52, 0x4F))),
            scrollbar_thumb: c("scrollbar.thumb.default", text_primary.with_alpha(0x26)),
            scrollbar_thumb_hovered: c("scrollbar.thumb.hovered", text_primary.with_alpha(0x33)),
            scrollbar_thumb_pressed: c("scrollbar.thumb.pressed", text_primary.with_alpha(0x4D)),
            scrollbar_track_hovered: c("scrollbar.track.hovered", text_primary.with_alpha(0x1A)),
            status_text: c("statusBar.text.default", text_primary.with_alpha(0x80)),
            popup_bg: c("popup.background", bg_secondary),
            popup_border: c("popup.border", text_primary.with_alpha(0x21)),
            popup_separator: c("popup.separator", text_primary.with_alpha(0x16)),
            popup_text: c("popup.text", text_primary),
            tooltip_bg: c("tooltip.background", Color::rgb(0x32, 0x34, 0x38)),
            tooltip_border: c("tooltip.border", transparent),
            tooltip_text: c("tooltip.text.primary", text_primary),
            tooltip_text_secondary: c("tooltip.text.secondary", text_primary.with_alpha(0x85)),
            shadow_medium: c(
                "shadow.background.medium",
                Color::rgb(0, 0, 0).with_alpha(0xD9),
            ),
            button_primary_bg: c(
                "button.primary.background.default",
                Color::rgb(0x08, 0x70, 0xE4),
            ),
            button_primary_bg_hovered: c(
                "button.primary.background.hovered",
                Color::rgb(0x18, 0x68, 0xCB),
            ),
            button_primary_text: c("button.primary.text.default", Color::rgb(0xFF, 0xFF, 0xFF)),
            button_secondary_bg: c(
                "button.secondary.background.default",
                text_primary.with_alpha(0x21),
            ),
            button_secondary_bg_hovered: c(
                "button.secondary.background.hovered",
                text_primary.with_alpha(0x1C),
            ),
            button_secondary_bg_pressed: c(
                "button.secondary.background.pressed",
                text_primary.with_alpha(0x26),
            ),
            button_secondary_border: c("button.secondary.border.default", transparent),
            button_secondary_text: c("button.secondary.text.default", text_primary),
            toggle_on_bg: c(
                "toggleButton.on.background.default",
                Color::rgb(0x08, 0x70, 0xE4),
            ),
            toggle_on_bg_hovered: c(
                "toggleButton.on.background.hovered",
                Color::rgb(0x18, 0x68, 0xCB),
            ),
            toggle_on_border: c("toggleButton.on.border.default", transparent),
            toggle_on_text: c("toggleButton.on.text.default", Color::rgb(0xFF, 0xFF, 0xFF)),
            toggle_off_bg: c("toggleButton.off.background.default", transparent),
            toggle_off_bg_hovered: c(
                "toggleButton.off.background.hovered",
                text_primary.with_alpha(0x12),
            ),
            toggle_off_bg_pressed: c(
                "toggleButton.off.background.pressed",
                text_primary.with_alpha(0x16),
            ),
            toggle_off_border: c(
                "toggleButton.off.border.default",
                text_primary.with_alpha(0x21),
            ),
            toggle_off_text: c("toggleButton.off.text.default", text_primary),
            toggle_off_text_hovered: c("toggleButton.off.text.hovered", text_primary),
            ghost_text: c("ghostButton.off.text.default", text_primary),
            ghost_text_hovered: c("ghostButton.off.text.hovered", text_primary),
            input_bg: c("inputField.background.default", transparent),
            input_bg_hovered: c("inputField.background.hovered", transparent),
            input_border: c("inputField.border.default", text_primary.with_alpha(0x21)),
            input_border_hovered: c("inputField.border.hovered", text_primary.with_alpha(0x43)),
            input_focus_border: c(
                "inputField.focusBorder.default",
                Color::rgb(0x2A, 0x7D, 0xEB),
            ),
            input_focus_outline: c(
                "inputField.focusOutline.default",
                Color::rgb(0x22, 0x42, 0x71),
            ),
            input_text: c("inputField.text.default", text_primary),
            input_hint: c("inputField.hint.default", Color::rgb(0x89, 0x8E, 0x94)),
            input_caret: c("inputField.caret.default", text_primary),
            input_selection: c(
                "inputField.selectionBackground.default",
                Color::rgb(0x22, 0x42, 0x71),
            ),
            search_counter_bg: c("search.counter.background", text_primary.with_alpha(0x2C)),
            smart_mode_enabled: c("smartMode.background.enabled", Color::rgb(0x2A, 0x7D, 0xEB)),
            search_match_text: c("search.match.text", text_primary),
            link: first(&["link.text.default"], Color::rgb(0x4B, 0x8D, 0xEC)),
            shortcut_text: c("shortcut.text", text_primary),
            shortcut_bg: c("shortcut.background", text_primary.with_alpha(0x21)),
            notification_bg: c("notification.background.default", bg_secondary),
            notification_bg_unread: c(
                "notification.background.unread",
                text_primary.with_alpha(0x0B),
            ),
            notification_text: c("notification.text.primary", text_primary),
            notification_text_secondary: c(
                "notification.text.secondary",
                text_primary.with_alpha(0x85),
            ),
            notification_separator: c("notification.separator", Color::rgb(0x3E, 0x41, 0x47)),
            toast_bg: c("toast.background", Color::rgb(0x32, 0x34, 0x38)),
            toast_border: c("toast.border", transparent),
            toast_text: c("toast.text", text_primary),
            drag_bg: c(
                "dragAndDrop.background",
                Color::rgb(0x08, 0x70, 0xE4).with_alpha(0x26),
            ),
            drag_border: c("dragAndDrop.border.default", Color::rgb(0x2A, 0x7D, 0xEB)),
            drag_border_highlight: c("dragAndDrop.border.highlight", Color::rgb(0x2A, 0x7D, 0xEB)),
            button_dangerous_bg: c(
                "button.dangerous.background.default",
                Color::rgb(0xC9, 0x4F, 0x4F),
            ),
            button_dangerous_bg_hovered: c(
                "button.dangerous.background.hovered",
                Color::rgb(0xB3, 0x45, 0x45),
            ),
            button_dangerous_text: c(
                "button.dangerous.text.default",
                Color::rgb(0xFF, 0xFF, 0xFF),
            ),
            popup_goto_bg: first(&["popup.goto.background", "popup.background"], bg_secondary),
            popup_footer_bg: first(
                &["popup.footer.background", "popup.background"],
                bg_secondary,
            ),
            popup_footer_text: c("popup.footer.text", text_primary.with_alpha(0x85)),
            syntax,
        }
        .with_transparent_fallbacks(transparent)
    }

    fn with_transparent_fallbacks(self, _t: Color) -> Theme {
        self
    }

    /// The attribute a highlighter capture paints as, by Fleet key, with the
    /// dotted-name cascade already applied when the theme was loaded.
    pub fn syntax(&self, key: &str) -> Option<&Syntax> {
        if let Some(s) = self.syntax.get(key) {
            return Some(s);
        }
        let mut key = key;
        while let Some((parent, _)) = key.rsplit_once('.') {
            if let Some(s) = self.syntax.get(parent) {
                return Some(s);
            }
            key = parent;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_theme_loads() {
        for (name, _) in assets::THEMES {
            let theme = Theme::load(&ThemeSource::Bundled(name.to_string()))
                .unwrap_or_else(|e| panic!("{name}: {e:#}"));
            assert_eq!(theme.dark, *name != "light", "{name}");
        }
        let light = Theme::load(&ThemeSource::Bundled("light".into())).unwrap();
        assert_eq!(
            light.island_background,
            to_hsla(Color::rgb(0xFF, 0xFF, 0xFF))
        );
        assert_eq!(
            light.background_primary,
            to_hsla(Color::rgb(0xEE, 0xEF, 0xF0))
        );
    }
}
