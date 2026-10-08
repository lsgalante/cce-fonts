mod pages;

use wayland_client::QueueHandle;
use cce_ui::widget::Owned;
use cce_ui::cosmic_text::FontSystem;
use cce_ui::engine::{Application, EngineState, LogicalPosition, LogicalSize, WindowSettings};
use cce_ui::widget::{
    MouseButton, ElementState, MouseScrollDelta, KeyEvent, WidgetHost, Event,
    TextBox, Button, Key, NamedKey, Dropdown, Spinbox, ScrollRegion,
};


/// How long after a click on a family row a second click on the same row
/// still counts as a double-click (and, in `--select` mode, confirms).
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(350);

/// The family list pane's width and the select-mode bar's height — sizes, not
/// spacing; every inset and gap around them comes from the ladder.
const LEFT_PANEL_W: f32 = 270.0;
const SELECT_BAR_H: f32 = 48.0;
/// The family list's row height (its rows are `list_item_h` tall, but
/// `scroll_to_index` keeps the legacy 24 it always scrolled by).
const LIST_ROW_H: f32 = 24.0;

/// The preview box's text until the user types their own.
const DEFAULT_PREVIEW: &str = "The quick brown fox jumps over the lazy dog";
/// The alphabet box's lines for a face that covers Latin.
const LATIN_SAMPLE: [&str; 4] = [
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "abcdefghijklmnopqrstuvwxyz",
    "0123456789",
    "!@#$%^&*()_+-=[]{}|;':\",./<>",
];
/// The sample lines drawn from a non-Latin face's own coverage: how many, and
/// how many characters on each.
const COVERAGE_LINES: usize = 4;
const COVERAGE_LINE_CHARS: usize = 16;

fn latin_sample() -> Vec<String> {
    LATIN_SAMPLE.iter().map(|l| l.to_string()).collect()
}

/// The face fontdb loaded from `file`#`index`, if it loaded one: the faces it
/// could not parse (WOFF2, bitmap-only .otb) are absent, and so are a variable
/// font's named instances, which fc-list reports at index `n << 16` but
/// fontdb (and the cosmic-text 0.12 renderer, which cannot set variation
/// coordinates) knows only as the one default-instance face at index 0.
fn find_face<'a>(
    db: &'a cce_ui::cosmic_text::fontdb::Database,
    file: &str,
    index: u32,
) -> Option<&'a cce_ui::cosmic_text::fontdb::FaceInfo> {
    use cce_ui::cosmic_text::fontdb::Source;
    db.faces().find(|f| {
        f.index == index && matches!(&f.source, Source::File(p) | Source::SharedFile(p, _) if p.as_os_str() == file)
    })
}

/// fc-list's faces that the renderer can actually draw (see `find_face`), one
/// per family + style. An unloadable face is dropped BEFORE the dedup, so a
/// family shipping both a .woff2 and a .ttf of a style keeps the .ttf: the
/// .woff2 used to win when fc-list listed it first, and the style then
/// previewed as the family's default face.
fn previewable_fonts(db: &cce_ui::cosmic_text::fontdb::Database) -> Vec<pages::FontEntry> {
    let mut seen = std::collections::HashSet::new();
    pages::fetch_fonts()
        .into_iter()
        .filter(|f| find_face(db, &f.file, f.index).is_some())
        .filter(|f| seen.insert((f.family.clone(), f.style.clone())))
        .collect()
}

/// The alphabet box's lines for `face`: the Latin sample when the face covers
/// Latin letters, else characters from its own coverage — an Arabic, Hebrew,
/// music or icon font has no Latin, so the Latin sample drew entirely in the
/// fallback sans and the preview showed nothing of the font. The characters
/// come from the face's most populated 256-codepoint blocks first (its script,
/// or its private-use icons), letters first, each in codepoint order.
fn sample_lines(db: &cce_ui::cosmic_text::fontdb::Database, face: &cce_ui::cosmic_text::fontdb::FaceInfo) -> Vec<String> {
    let mapped = db.with_face_data(face.id, |data, index| {
        let Ok(f) = ttf_parser::Face::parse(data, index) else { return Vec::new() };
        let mut cps = std::collections::BTreeSet::new();
        for sub in f.tables().cmap.into_iter().flat_map(|c| c.subtables) {
            if sub.is_unicode() {
                sub.codepoints(|cp| {
                    // A mark or a zero-advance glyph (a format character) shows
                    // nothing alone but a dotted circle or a blank, and Arabic and
                    // Hebrew open their blocks with dozens of them.
                    let shows = sub.glyph_index(cp).is_some_and(|g| {
                        g.0 != 0
                            && f.tables().gdef.and_then(|t| t.glyph_class(g)) != Some(ttf_parser::gdef::GlyphClass::Mark)
                            && f.glyph_hor_advance(g) != Some(0)
                    });
                    if shows {
                        cps.insert(cp);
                    }
                });
            }
        }
        cps.into_iter()
            .filter_map(char::from_u32)
            .filter(|c| !c.is_control() && !c.is_whitespace())
            .collect::<Vec<char>>()
    });
    let Some(chars) = mapped.filter(|c| !c.is_empty()) else { return latin_sample() };
    if chars.iter().filter(|c| c.is_ascii_alphabetic()).count() >= 26 {
        return latin_sample();
    }
    let mut blocks: Vec<Vec<char>> = Vec::new();
    for c in chars {
        match blocks.last_mut() {
            Some(b) if b[0] as u32 >> 8 == c as u32 >> 8 => b.push(c),
            _ => blocks.push(vec![c]),
        }
    }
    // The presentation-form blocks (Latin / Hebrew / Arabic ligatures and
    // contextual forms) last: an Arabic font maps hundreds of them, more than
    // its base letters, which shaping turns into those forms anyway.
    let presentation = |c: char| matches!(c as u32, 0xFB00..=0xFDFF | 0xFE70..=0xFEFF);
    blocks.sort_by_key(|b| (presentation(b[0]), std::cmp::Reverse(b.len())));
    // Letters ahead of the script's punctuation and signs; icons and emoji
    // are no letters, and keep their order.
    let (mut picked, rest): (Vec<char>, Vec<char>) = blocks.into_iter().flatten().partition(|c| c.is_alphabetic());
    picked.extend(rest);
    picked.truncate(COVERAGE_LINES * COVERAGE_LINE_CHARS);
    picked
        .chunks(COVERAGE_LINE_CHARS)
        .map(|line| line.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" "))
        .collect()
}

/// The family list's row gap, doubling as its inner inset.
/// style: deliberate — the rows are 24px scan lines and the pane gap would
/// triple their pitch; this is the toolkit `List::new(24, 4)` metric the
/// list dissolved from, kept tight so a long family list stays scannable.
fn list_row_gap() -> f32 {
    4.0
}

/// The mid pane's preview form, stacked by the ladder inside the pane's
/// scroll content: each control's virtual y (from the pane's top rim) and
/// height, and the content height that stack adds up to.
struct PreviewForm {
    buttons_y: f32,
    dropdown_y: f32,
    dropdown_h: f32,
    spinbox_y: f32,
    spinbox_h: f32,
    preview_y: f32,
    preview_h: f32,
    alphabet_y: f32,
    alphabet_h: f32,
    content_h: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseNavigation {
    Up,
    Down,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum AppMessage {
    Exit,
    SelectFamily(usize),
    SelectStyle(usize),
    FontSizeChanged,
    SearchChanged,
    OpenFolder,
    RemoveFont,
    RefreshFonts,
}

/// The scrollbar's idle copy: the regions are sink-behind, so the bar rides
/// the region's centre line and idles behind the panel plate, every frame —
/// emitted BEFORE that plate. (No bg fill of its own: the region shows the
/// plate it sits on.) Its fore copy is emitted after the rows, by
/// [`emit_region_bar`].
fn emit_region_idle_bar(r: &ScrollRegion, pc: &mut cce_ui::scene::paint::PaintCtx) {
    r.push_scrollbar_prims(pc);
}

/// The scrollbar's fore copy, at the raise's fade (nothing while sunk):
/// emitted after the panel content so the bar rides over the rows.
fn emit_region_bar(r: &ScrollRegion, pc: &mut cce_ui::scene::paint::PaintCtx) {
    r.push_scrollbar_fore(pc);
}

/// App shortcuts, resolved once at startup from input.kdl
/// (`cce-fonts` domain → `cce-ui` domain), defaulting to the historical keys.
struct FontsKeys {
    refresh: String,
    open_search: String,
    focus_preview: String,
    open_folder: String,
    remove_font: String,
}

impl FontsKeys {
    fn load() -> Self {
        let get = cce_ui::input::app_chord;
        Self {
            refresh: get("refresh", "ctrl+r"),
            open_search: get("open_search", "ctrl+f"),
            focus_preview: get("focus_preview", "ctrl+e"),
            open_folder: get("open_folder", "ctrl+o"),
            remove_font: get("remove_font", "delete"),
        }
    }
}

struct TypefaceApp {
    keys: FontsKeys,

    // Browse panel
    search_box: Owned<cce_ui::widget::Adapted<TextBox>>,
    list_region: ScrollRegion,
    list_item_h: f32,
    font_buttons: Vec<Owned<cce_ui::widget::Adapted<cce_ui::widget::Button>>>,

    // Preview panel
    style_dropdown: Owned<cce_ui::widget::Adapted<Dropdown>>,
    size_spinbox: Owned<cce_ui::widget::Adapted<cce_ui::widget::Spinbox>>,
    preview_box: Owned<cce_ui::widget::Adapted<TextBox>>,

    // Details panel
    btn_open_folder: Owned<cce_ui::widget::Adapted<cce_ui::widget::Button>>,
    btn_remove_font: Owned<cce_ui::widget::Adapted<cce_ui::widget::Button>>,

    // Selection mode buttons
    select_cancel_btn: Owned<cce_ui::widget::Adapted<cce_ui::widget::Button>>,
    select_confirm_btn: Owned<cce_ui::widget::Adapted<cce_ui::widget::Button>>,
    select_mode: bool,
    last_click_idx: Option<usize>,
    /// When that click landed. A wall clock, not a `dt` countdown: `dt` is
    /// animation time, clamped to one frame after an idle sleep, and the app
    /// idles between clicks — so the 0.35 s window stayed open for seconds
    /// and a leisurely second click confirmed as a double-click.
    last_click_at: Option<std::time::Instant>,

    // State
    all_fonts: Vec<pages::FontEntry>,
    families: Vec<String>,
    filtered: Vec<String>,
    selected_idx: Option<usize>,
    selected_family: Option<String>,
    selected_style: Option<String>,
    selected_file: Option<String>,
    family_styles: Vec<String>,
    family_files: Vec<String>,
    family_indices: Vec<u32>,
    /// The selected face's own italic / weight (see `face_attrs`): the preview
    /// box and the alphabet are shaped with it.
    selected_attrs: cce_ui::scene::paint::TextAttrs,
    /// The family that names exactly the selected face (cce-ui's `face_family`
    /// alias), which the preview box and alphabet are shaped in. The family's
    /// own name will not do: BodonianScript's seven cuts, FormP Color Six's
    /// colourways and Old Timey Mono's Condensed and Compressed files share one
    /// family AND one style, width and weight, so by name every one of them
    /// previewed as the first.
    preview_family: Option<String>,
    /// The alphabet box's lines for the selected face (see `sample_lines`).
    sample: Vec<String>,
    charset_count: usize,
    charset_str: String,
    is_user_font: bool,

    // UI metrics
    width: u32,
    height: u32,
    scale_factor: f64,
    // Shapes glyph advances (prepare_text) — load-bearing for cursor↔pixel mapping and label
    // measurement; all rendered text is display-list prims shaped by the engine. Must stay
    // create_font_system_with_system_fonts() so measurement sees the same faces the engine
    // renders (load_system_fonts).
    font_system: FontSystem,
    needs_rebuild: bool,

    // Containers (root plate container, the three Plates, and the ScrollBox/List are DISSOLVED:
    // plates and scroll frames are prims, scroll state lives in the ScrollRegions,
    // children are dispatched/walked directly, panel rects are computed)
    mid_region: ScrollRegion,
    widgets_registered: bool,
    ui_context: cce_ui::context::UiContext,
}

impl TypefaceApp {
    /// Register the widgets (parentless — the panel Plates are dissolved). Static widgets
    /// once; the font-list buttons every rebuild (they are recreated on search changes,
    /// same cadence the old per-rebuild link_parent_child re-registration had).
    fn register_widgets(&mut self) {
        if !self.widgets_registered {
            self.widgets_registered = true;
            self.ui_context.register_host(&mut self.search_box);
            self.ui_context.register_host(&mut self.btn_open_folder);
            self.ui_context.register_host(&mut self.btn_remove_font);
            self.ui_context.register_host(&mut self.style_dropdown);
            self.ui_context.register_host(&mut self.size_spinbox);
            self.ui_context.register_host(&mut self.preview_box);
            self.ui_context.register_host(&mut self.select_cancel_btn);
            self.ui_context.register_host(&mut self.select_confirm_btn);
        }
        for btn in self.font_buttons.iter_mut() {
            if btn.rect().0 > -9000.0 {
                self.ui_context.register_host(btn);
            }
        }
    }

    /// The panes' height: the window less the root inset at top and bottom,
    /// and in select mode the bar and the root gap that separates it.
    fn content_h(&self) -> f32 {
        let h = self.height as f32;
        let inset = cce_ui::layout::root_plate_inset();
        if self.select_mode {
            (h - 2.0 * inset - cce_ui::layout::root_plate_gap() - SELECT_BAR_H).max(100.0)
        } else {
            (h - 2.0 * inset).max(100.0)
        }
    }

    /// The dissolved mid panel's rect (was `mid_panel.rect()`): the root gap
    /// right of the family list pane, the root inset from the other edges.
    fn mid_panel_rect(&self) -> (f32, f32, f32, f32) {
        let inset = cce_ui::layout::root_plate_inset();
        let mid_x = inset + LEFT_PANEL_W + cce_ui::layout::root_plate_gap();
        (mid_x, inset, self.width as f32 - inset - mid_x, self.content_h())
    }

    /// The preview form's stack (see [`PreviewForm`]): the pane padding at
    /// the top, the control gap between the form's controls, the pane gap
    /// before the alphabet box, and the pane padding below it.
    fn preview_form(&self) -> PreviewForm {
        let pad = cce_ui::layout::plate_padding();
        let gap = cce_ui::layout::plate_gap();
        let control_gap = cce_ui::layout::control_gap();
        let dropdown_h = cce_ui::layout::dropdown_height() + self.style_dropdown.label_strip();
        let spinbox_h = cce_ui::layout::spinbox_height() + self.size_spinbox.label_strip();
        let preview_h = if self.select_mode { 120.0 } else { 180.0 };
        let alphabet_h = if self.select_mode { 102.0 } else { 120.0 };
        let buttons_y = pad;
        let dropdown_y = buttons_y + cce_ui::layout::button_height() + control_gap;
        let spinbox_y = dropdown_y + dropdown_h + control_gap;
        let preview_y = spinbox_y + spinbox_h + control_gap;
        let alphabet_y = preview_y + preview_h + gap;
        PreviewForm {
            buttons_y,
            dropdown_y,
            dropdown_h,
            spinbox_y,
            spinbox_h,
            preview_y,
            preview_h,
            alphabet_y,
            alphabet_h,
            content_h: alphabet_y + alphabet_h + pad,
        }
    }

    /// The dissolved Plates' visual: a pane plate made of the root plate's own
    /// material (`Material::root()`), so a panel inherits the window base's colour
    /// rather than wearing a tint of its own, set off by the DE roll.
    fn plate_prims(&self, rect: cce_ui::scene::layout::Rect, pc: &mut cce_ui::scene::paint::PaintCtx) {
        let radius = cce_ui::layout::plate_corner_radius();
        let depth = cce_ui::layout::bevel_width().min(rect.height * 0.2);
        pc.plate(rect, (radius, radius, radius, radius), &cce_ui::scene::Material::root(), depth);
    }

    /// Event dispatch order of the dissolved panels: the flat child list, panel-grouped
    /// (left: search/list/buttons; mid, when a family is selected; bottom bar in select
    /// mode) — the same sets the Plates forwarded to.
    fn dispatch_widgets(&mut self, forward: bool) -> Vec<cce_ui::widget::WidgetId> {
        let mut v: Vec<cce_ui::widget::WidgetId> = Vec::new();
        v.push(self.search_box.id());
        for btn in self.font_buttons.iter() {
            if btn.rect().0 > -9000.0 {
                v.push(btn.id());
            }
        }
        if self.selected_family.is_some() {
            v.push(self.btn_open_folder.id());
            v.push(self.btn_remove_font.id());
            v.push(self.style_dropdown.id());
            v.push(self.size_spinbox.id());
            v.push(self.preview_box.id());
        }
        if self.select_mode {
            v.push(self.select_cancel_btn.id());
            v.push(self.select_confirm_btn.id());
        }
        if !forward {
            v.reverse();
        }
        v
    }

    fn reload_fonts(&mut self) {
        self.all_fonts = previewable_fonts(self.font_system.db());
        self.families = self.extract_families(&self.all_fonts);
        let query = if self.search_box.editing { &self.search_box.edit_buffer } else { &self.search_box.text };
        self.filtered = self.filter_families(&self.families, query);
        // Ids are globally monotonic and never reused, so the fresh buttons register under
        // NEW ids; the outgoing ones would keep pointing into this Vec's freed buffer, and
        // the engine derefs the whole registry on every left press. Drop them first.
        let stale: Vec<_> = self.font_buttons.iter().map(|b| b.id()).collect();
        for id in stale {
            self.ui_context.unregister_widget(id);
        }
        self.font_buttons = self.filtered.iter().map(|f| Owned::new(Button::new_list_row(0.0, 0.0, 0.0, 0.0).with_label(f))).collect();
        // ...and register the replacements now. `dispatch_widgets()` reports every row whose
        // rect passes the parked-sentinel gate, and fresh rows are (0,0,0,0) so they pass it
        // immediately — but `register_widgets()` only runs in the next layout pass, so the
        // ids were reported unregistered until then and the router dropped those events.
        for btn in self.font_buttons.iter_mut() {
            self.ui_context.register_host(btn);
        }

        if let Some(sel) = self.selected_idx {
            if sel >= self.filtered.len() {
                self.selected_idx = None;
                self.selected_family = None;
                self.selected_style = None;
                self.selected_file = None;
                self.family_styles.clear();
                self.family_files.clear();
                self.family_indices.clear();
                self.selected_attrs = cce_ui::scene::paint::TextAttrs::default();
                self.preview_family = None;
                self.set_sample(latin_sample());
                self.style_dropdown.options.clear();
                self.style_dropdown.selected = 0;
                self.charset_count = 0;
                self.charset_str = String::from("0");
                self.is_user_font = false;
            } else {
                let family = self.filtered[sel].clone();
                self.select_family(family);
            }
        }
        self.needs_rebuild = true;
    }

    fn extract_families(&self, fonts: &[pages::FontEntry]) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        for f in fonts {
            seen.insert(f.family.clone());
        }
        seen.into_iter().collect()
    }

    fn filter_families(&self, families: &[String], query: &str) -> Vec<String> {
        if query.is_empty() {
            return families.to_vec();
        }
        let q = query.to_lowercase();
        families
            .iter()
            .filter(|f| f.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    fn select_family(&mut self, family: String) {
        self.selected_family = Some(family.clone());

        let entries: Vec<&pages::FontEntry> = self.all_fonts.iter().filter(|f| f.family == family).collect();
        self.family_styles = entries.iter().map(|e| e.style.clone()).collect();
        self.family_files = entries.iter().map(|e| e.file.clone()).collect();
        self.family_indices = entries.iter().map(|e| e.index).collect();

        // Pick default style: "Regular" if available, else first
        let default_idx = self.family_styles.iter().position(|s| s == "Regular").unwrap_or(0);
        self.style_dropdown.options = self.family_styles.clone();
        self.style_dropdown.selected = default_idx;

        if self.family_styles.is_empty() {
            self.selected_style = None;
            self.selected_file = None;
            self.selected_attrs = cce_ui::scene::paint::TextAttrs::default();
            self.preview_family = None;
            self.set_sample(latin_sample());
            self.charset_count = 0;
            self.charset_str = String::from("0");
            self.is_user_font = false;
            self.sync_preview_font();
        } else {
            self.select_style(default_idx);
        }

        // Highlight selected button
        if let Some(sel) = self.selected_idx {
            for (idx, btn) in self.font_buttons.iter_mut().enumerate() {
                btn.selected = idx == sel;
            }
        }
        
        self.needs_rebuild = true;
    }

    /// Make the family's `idx`th style the shown face: its file's charset and
    /// home, and the attrs the preview text is shaped with.
    fn select_style(&mut self, idx: usize) {
        let style = self.family_styles[idx].clone();
        let file = self.family_files[idx].clone();
        self.style_dropdown.selected = idx;
        self.selected_style = Some(style);
        self.selected_attrs = self.face_attrs(&file, self.family_indices[idx]);
        self.preview_family = Some(cce_ui::backend::text::face_family(&file, self.family_indices[idx]));
        let sample = match find_face(self.font_system.db(), &file, self.family_indices[idx]) {
            Some(face) => sample_lines(self.font_system.db(), face),
            None => latin_sample(),
        };
        self.set_sample(sample);
        self.charset_count = pages::count_chars(&file);
        self.charset_str = self.charset_count.to_string();
        self.is_user_font = pages::is_user_font(&file);
        self.selected_file = Some(file);
        self.sync_preview_font();
    }

    /// The italic / weight / width that select exactly the face at `file`#`index`,
    /// as fontdb read them from the font itself. cosmic-text takes a family's face
    /// only at its exact weight, style and stretch, so the style NAME is no guide:
    /// Circe Slab's Light is weight 350, Bodoni Egyptian Mono Thin 280, Roboto's
    /// Thin 250, and a guessed 300 or 400 matched none of them; Circe Slab A Narrow
    /// differs from its sibling only in width.
    fn face_attrs(&self, file: &str, index: u32) -> cce_ui::scene::paint::TextAttrs {
        use cce_ui::cosmic_text::fontdb::{Stretch, Style};
        match find_face(self.font_system.db(), file, index) {
            Some(f) => cce_ui::scene::paint::TextAttrs {
                italic: f.style != Style::Normal,
                weight: Some(f.weight.0),
                stretch: (f.stretch != Stretch::Normal).then(|| f.stretch.to_number()),
            },
            None => cce_ui::scene::paint::TextAttrs::default(),
        }
    }

    /// Show `sample` in the alphabet box, and its first line in the preview box
    /// while that still holds the app's own text — never over what the user typed.
    fn set_sample(&mut self, sample: Vec<String>) {
        let auto = |s: &[String]| {
            if s.iter().eq(LATIN_SAMPLE.iter()) {
                DEFAULT_PREVIEW.to_string()
            } else {
                s.first().cloned().unwrap_or_else(|| DEFAULT_PREVIEW.to_string())
            }
        };
        let (old, new) = (auto(&self.sample), auto(&sample));
        if !self.preview_box.editing && self.preview_box.text == old {
            self.preview_box.text = new;
        }
        self.sample = sample;
    }

    /// Point the preview box at the selected face. The family goes in WITH the
    /// size, as a "family size" font string: a bare family whose name ends in a
    /// number ("Noto Sans Symbols 2") would otherwise have that number read as
    /// its size by the toolkit's font-string split.
    fn sync_preview_font(&mut self) {
        if let Some(family) = self.preview_family.as_ref().or(self.selected_family.as_ref()) {
            self.preview_box.font_family = format!("{} {}", family, self.preview_box.font_size);
        }
        self.preview_box.font_attrs = self.selected_attrs;
    }

    fn scroll_to_index(&mut self, index: usize) {
        let item_height_full = LIST_ROW_H + list_row_gap();
        let top = index as f32 * item_height_full;
        let bottom = top + LIST_ROW_H;
        let viewport_top = self.list_region.scroll_y;
        let viewport_bottom = viewport_top + self.list_region.viewport_h;

        if bottom > viewport_bottom {
            self.list_region.scroll_y = (bottom - self.list_region.viewport_h).clamp(0.0, self.list_region.max_scroll());
        } else if top < viewport_top {
            self.list_region.scroll_y = top;
        }
    }

    /// Glyph-advance shaping for the interactive widgets — load-bearing for cursor↔pixel
    /// mapping and label measurement; all rendered text is display-list prims shaped by the
    /// engine (which also carries the system fonts via `load_system_fonts`).
    fn refresh_widget_text(&mut self) {
        let font_system = &mut self.font_system;
        self.search_box.prepare_text(font_system);
        for btn in &mut self.font_buttons {
            btn.prepare_text(font_system);
        }
        self.style_dropdown.prepare_text(font_system);
        self.size_spinbox.prepare_text(font_system);
        self.preview_box.prepare_text(font_system);
        self.btn_open_folder.prepare_text(font_system);
        self.btn_remove_font.prepare_text(font_system);
        self.select_cancel_btn.prepare_text(font_system);
        self.select_confirm_btn.prepare_text(font_system);
    }

    /// The alphabet preview as display-list text: one prim per line (the legacy single
    /// multi-line buffer becomes per-line prims at the same 1.4 line spacing), in the selected
    /// family with the style variant's italic/weight attrs (`TextAttrs` — the Phase 6 prim
    /// extension this app motivated), clipped to the preview box.
    fn push_alphabet_preview(&self, pc: &mut cce_ui::scene::paint::PaintCtx) {
        let Some(family) = self.preview_family.as_ref().or(self.selected_family.as_ref()) else { return };

        let font_size = self.size_spinbox.value as f32;
        let attrs = self.selected_attrs;

        let (mid_panel_x, panel_y, mid_panel_w, mid_panel_h) = self.mid_panel_rect();
        let pad = cce_ui::layout::plate_padding();
        let preview_box_x = mid_panel_x + pad;
        let preview_box_w = mid_panel_w - 2.0 * pad;

        let form = self.preview_form();
        let alphabet_box_h = form.alphabet_h;
        let scroll_y = self.mid_region.scroll_y;
        let alphabet_draw_y = panel_y + form.alphabet_y - scroll_y;

        let viewport_top = panel_y;
        let viewport_bottom = panel_y + mid_panel_h;
        if alphabet_draw_y + alphabet_box_h < viewport_top || alphabet_draw_y > viewport_bottom {
            return;
        }
        let bounds_y_start = alphabet_draw_y.max(viewport_top);
        let bounds_y_end = (alphabet_draw_y + alphabet_box_h).min(viewport_bottom);
        let bounds = Some([preview_box_x, bounds_y_start, preview_box_x + preview_box_w, bounds_y_end]);

        let size_used = (font_size * 0.55).clamp(8.0, 36.0);
        for (i, line) in self.sample.iter().enumerate() {
            pc.text_attrs(
                line.as_str(),
                preview_box_x + pad,
                alphabet_draw_y + pad + i as f32 * size_used * 1.4,
                size_used,
                [0x88, 0x88, 0x99],
                // With the size, like `sync_preview_font`, so a trailing number in the
                // family name stays part of it.
                Some(format!("{family} {size_used}")),
                bounds,
                attrs,
            );
        }
    }
}

impl Application for TypefaceApp {
    type Message = AppMessage;

    fn ui_context(&self) -> Option<&cce_ui::context::UiContext> {
        Some(&self.ui_context)
    }

    // The engine ticks the exposed context each loop — this is what drives the
    // dropdown expand/contract animation frames.
    fn ui_context_mut(&mut self) -> Option<&mut cce_ui::context::UiContext> {
        Some(&mut self.ui_context)
    }

    /// The font picker previews arbitrary installed families: the engine's render FontSystem
    /// must contain the system fonts, or preview text asking for a system-only family is
    /// silently invisible.
    fn load_system_fonts(&self) -> bool {
        true
    }

    fn display_list_text(&self) -> bool {
        true
    }

    fn is_movable_root_plate_at(&self, px: f32, py: f32) -> bool {
        // root plate container dissolved: the surface itself is the movable plate.
        self.ui_context.drag_allowed_at(px, py)
    }

    fn new(_qh: &QueueHandle<EngineState<Self>>, _sender: calloop::channel::Sender<Self::Message>) -> Self {
        cce_ui::scale::set_scale_factor(1.0);
        // Parse command line arguments
        let args: Vec<String> = std::env::args().collect();
        let select_mode = args.iter().any(|arg| arg == "--select");

        let mut search_box = TextBox::new(String::new()).with_multiline(false).with_draw_bg_border(true);
        search_box.font_size = 12.0;

        // The dissolved List's item metrics (List::new(24, 4) adjusted item height to
        // max(24, list font size + 14)); scroll_to_index keeps its legacy hardcoded 24s.
        let list_item_h = 24.0f32.max(cce_ui::layout::list_font_parsed().1 + 14.0);

        let style_dropdown = Dropdown::new(Vec::new(), 0).with_label("Style:");

        let size_spinbox = Spinbox::new(32, 8, 120, 1).with_label("Size:");

        // Wrapped whatever the DE's `textbox_line_wrap`: a sample is read whole,
        // not scrolled sideways, and a 32px one outruns the box.
        let mut preview_box = TextBox::new(String::from(DEFAULT_PREVIEW)).with_multiline(true).with_line_wrap(true).with_draw_bg_border(true).with_max_width(None);
        preview_box.font_size = 32.0;

        let btn_h = cce_ui::layout::button_height();
        let btn_open_folder = Button::new(914.0, 200.0, 110.0, btn_h).with_label("Open Folder");
        let btn_remove_font = Button::new_reset(1034.0, 200.0, 110.0, btn_h).with_label("Remove Font");

        let select_cancel_btn = Button::new_reset(0.0, 0.0, 80.0, btn_h).with_label("Cancel");
        let select_confirm_btn = Button::new(0.0, 0.0, 80.0, btn_h).with_label("Select");

        let font_system = cce_ui::create_font_system_with_system_fonts();
        let all_fonts = previewable_fonts(font_system.db());
        let mut app = Self {
            keys: FontsKeys::load(),
            search_box: Owned::new(search_box),
            list_region: ScrollRegion::new(0.0, 4.0)
                .with_sink_behind(true),
            list_item_h,
            font_buttons: Vec::new(),
            style_dropdown: Owned::new(style_dropdown),
            size_spinbox: Owned::new(size_spinbox),
            preview_box: Owned::new(preview_box),
            btn_open_folder: Owned::new(btn_open_folder),
            btn_remove_font: Owned::new(btn_remove_font),
            select_cancel_btn: Owned::new(select_cancel_btn),
            select_confirm_btn: Owned::new(select_confirm_btn),
            select_mode,
            last_click_idx: None,
            last_click_at: None,
            all_fonts,
            families: Vec::new(),
            filtered: Vec::new(),
            selected_idx: None,
            selected_family: None,
            selected_style: None,
            selected_file: None,
            family_styles: Vec::new(),
            family_files: Vec::new(),
            family_indices: Vec::new(),
            selected_attrs: cce_ui::scene::paint::TextAttrs::default(),
            preview_family: None,
            sample: latin_sample(),
            charset_count: 0,
            charset_str: String::from("0"),
            is_user_font: false,
            width: if select_mode { 900 } else { 1200 },
            height: if select_mode { 500 } else { 720 },
            scale_factor: 1.0,
            font_system,
            needs_rebuild: true,
            mid_region: ScrollRegion::new(0.0, 4.0)
                .with_sink_behind(true),
            widgets_registered: false,
            ui_context: cce_ui::context::UiContext::new(),
        };
        
        app.families = app.extract_families(&app.all_fonts);
        app.filtered = app.filter_families(&app.families, "");
        app.font_buttons = app.filtered.iter().map(|f| Owned::new(Button::new_list_row(0.0, 0.0, 0.0, 0.0).with_label(f))).collect();
        
        // Parse preselected family and size from CLI args (passed by FontSelector)
        let mut preselected_family = None;
        let mut preselected_size = None;
        if select_mode {
            let other_args: Vec<&String> = args.iter().filter(|arg| *arg != "--select" && !arg.ends_with("cce-fonts")).collect();
            if let Some(arg) = other_args.first() {
                let (fam, sz) = cce_ui::layout::parse_font_string(arg);
                preselected_family = Some(fam);
                preselected_size = sz;
            }
        }

        // Auto-select first font or preselected font at start if available
        if !app.filtered.is_empty() {
            let mut select_idx = 0;
            if let Some(ref pre_fam) = preselected_family {
                if let Some(pos) = app.filtered.iter().position(|f| f.to_lowercase() == pre_fam.to_lowercase()) {
                    select_idx = pos;
                }
            }
            app.selected_idx = Some(select_idx);
            let family = app.filtered[select_idx].clone();
            app.select_family(family);
        }

        if let Some(sz) = preselected_size {
            app.size_spinbox.value = sz as i32;
            if app.size_spinbox.editing {
                app.size_spinbox.edit_buffer = app.size_spinbox.value.to_string();
            }
            app.preview_box.font_size = sz;
            app.sync_preview_font();
        }

        app
    }

    fn settings(&self) -> WindowSettings {
        if self.select_mode {
            WindowSettings {
                title: "Select Font".to_string(),
                app_id: "cce-fonts-select".to_string(),
                width: 900,
                height: 500,
                fullscreen: false,
                min_size: Some((800, 400)),
            }
        } else {
            WindowSettings {
                title: "Fonts".to_string(),
                app_id: "cce-fonts".to_string(),
                width: 1200,
                height: 720,
                fullscreen: false,
                min_size: Some((800, 500)),
            }
        }
    }

    fn update(&mut self, msg: Self::Message, needs_rebuild: &mut bool, exit: &mut bool) {
        match msg {
            AppMessage::Exit => {
                *exit = true;
            }
            AppMessage::SelectFamily(idx) => {
                if idx < self.filtered.len() {
                    self.selected_idx = Some(idx);
                    let family = self.filtered[idx].clone();
                    self.select_family(family);
                    self.scroll_to_index(idx);
                    *needs_rebuild = true;
                    self.needs_rebuild = true;
                }
            }
            AppMessage::SelectStyle(idx) => {
                if idx < self.family_styles.len() {
                    self.select_style(idx);
                    *needs_rebuild = true;
                    self.needs_rebuild = true;
                }
            }
            AppMessage::FontSizeChanged => {
                self.preview_box.font_size = self.size_spinbox.value as f32;
                self.sync_preview_font();
                *needs_rebuild = true;
                self.needs_rebuild = true;
            }
            AppMessage::SearchChanged => {
                let query = if self.search_box.editing { &self.search_box.edit_buffer } else { &self.search_box.text };
                self.filtered = self.filter_families(&self.families, query);
                // See the note in refresh(): monotonic ids mean the outgoing buttons must be
                // unregistered or the registry keeps dangling pointers. This path runs on
                // every keystroke in the filter box, so it is the hottest producer of them.
                let stale: Vec<_> = self.font_buttons.iter().map(|b| b.id()).collect();
                for id in stale {
                    self.ui_context.unregister_widget(id);
                }
                self.font_buttons = self.filtered.iter().map(|f| Owned::new(Button::new_list_row(0.0, 0.0, 0.0, 0.0).with_label(f))).collect();
                // Register the replacements now rather than waiting for the next layout pass —
                // see refresh(). This is the path that produced the stale-root spam.
                for btn in self.font_buttons.iter_mut() {
                    self.ui_context.register_host(btn);
                }

                // reset or re-evaluate selection
                self.selected_idx = None;
                self.selected_family = None;
                self.selected_style = None;
                self.selected_file = None;
                self.family_styles.clear();
                self.family_files.clear();
                self.family_indices.clear();
                self.selected_attrs = cce_ui::scene::paint::TextAttrs::default();
                self.preview_family = None;
                self.set_sample(latin_sample());
                self.style_dropdown.options.clear();
                self.style_dropdown.selected = 0;
                self.charset_count = 0;
                self.charset_str = String::from("0");
                self.is_user_font = false;

                // Auto-select first match if available
                if !self.filtered.is_empty() {
                    self.selected_idx = Some(0);
                    let family = self.filtered[0].clone();
                    self.select_family(family);
                }

                *needs_rebuild = true;
                self.needs_rebuild = true;
            }
            AppMessage::OpenFolder => {
                if let Some(ref file) = self.selected_file {
                    if let Some(parent) = std::path::Path::new(file).parent() {
                        let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
                    }
                }
            }
            AppMessage::RemoveFont => {
                if let Some(ref file) = self.selected_file {
                    if self.is_user_font {
                        let _ = std::process::Command::new("gio").args(["trash", file]).spawn();
                        let _ = std::process::Command::new("fc-cache").arg("-f").spawn();
                        self.reload_fonts();
                        *needs_rebuild = true;
                        self.needs_rebuild = true;
                    }
                }
            }
            AppMessage::RefreshFonts => {
                self.reload_fonts();
                *needs_rebuild = true;
                self.needs_rebuild = true;
            }
        }
    }

    fn tick(&mut self, dt: f32, needs_rebuild: &mut bool) {
        // Pump the widget tick walk: animating widgets (the style dropdown's
        // expand/contract menu) register as tick receivers and report changed
        // until their transition lands — without this a closing menu freezes
        // fully open.
        if self.ui_context.tick(dt) {
            *needs_rebuild = true;
            self.needs_rebuild = true;
        }

        // Raise/sink upkeep for the panel scrollbars: true while the
        // post-scroll hold runs or on the depth flip, keeping frames coming
        // so the sink actually renders.
        if self.list_region.tick(dt) {
            *needs_rebuild = true;
            self.needs_rebuild = true;
        }
        if self.mid_region.tick(dt) {
            *needs_rebuild = true;
            self.needs_rebuild = true;
        }

    }

    fn display_list(&mut self, size: LogicalSize, scale: f64) -> Option<cce_ui::scene::paint::DisplayList> {
        // Phase 6 single paint path: setup/relayout (the old view() body), then the whole
        // frame — the widget tree walked into one list, the panel borders and alphabet box
        // (lost since the Phase 3 adoption discarded view()'s quads), the alphabet preview
        // prims, and the style-dropdown popover on top — is built here.
        let size_changed = self.width != size.width as u32 || self.height != size.height as u32 || self.scale_factor != scale;
        if self.needs_rebuild || size_changed {
            self.width = size.width as u32;
            self.height = size.height as u32;
            self.scale_factor = scale;
        }

        let w_f32 = self.width as f32;
        let h_f32 = self.height as f32;

        // The ladder: the window edge to a pane is the root inset; the two
        // panes (and the select bar below them) are siblings on the root plate,
        // a root gap apart; inside a pane, content stands off the rim by the
        // pane padding, and rows / grouped buttons sit a pane gap apart.
        let inset = cce_ui::layout::root_plate_inset();
        let pad = cce_ui::layout::plate_padding();
        let gap = cce_ui::layout::plate_gap();
        let row_gap = list_row_gap();

        let panel_y = inset;
        let left_panel_x = inset;
        let left_panel_w = LEFT_PANEL_W;
        let (mid_panel_x, _, mid_panel_w, _) = self.mid_panel_rect();

        let select_bar_h = SELECT_BAR_H;
        let content_h = self.content_h();
        let bar_y = h_f32 - select_bar_h - inset;

        if self.needs_rebuild || size_changed {
            cce_ui::scale::set_scale_factor(scale as f32);
            self.register_widgets();

            self.mid_region.set_rect(mid_panel_x, panel_y, mid_panel_w, content_h);

            let form = self.preview_form();
            self.mid_region.update_bounds_raw(form.content_h, panel_y, content_h);

            // Search box, inset from the pane's rim.
            let search_h = cce_ui::layout::textbox_height();
            self.search_box.set_rect(left_panel_x + pad, panel_y + pad, left_panel_w - 2.0 * pad, search_h);

            // Scrolling list: a pane gap below the search box, down to the
            // pane's padded bottom rim.
            let list_x = left_panel_x + pad;
            let list_y = panel_y + pad + search_h + gap;
            let list_w = left_panel_w - 2.0 * pad;
            let list_h = (panel_y + content_h - pad - list_y).max(100.0);
            self.list_region.set_rect(list_x, list_y, list_w, list_h);
            // The dissolved List's content math: count * (adjusted item height + row gap) + row gap.
            let item_full = self.list_item_h + row_gap;
            self.list_region.update_bounds_raw(self.filtered.len() as f32 * item_full + row_gap, list_y, list_h);

            // Layout list buttons, inset from the list by its row gap.
            for (idx, btn) in self.font_buttons.iter_mut().enumerate() {
                if let Some(draw_y) = self.list_region.get_draw_y(idx as f32 * item_full, self.list_item_h) {
                    btn.set_rect(list_x + row_gap, draw_y, list_w - 2.0 * row_gap, LIST_ROW_H);
                } else {
                    btn.set_rect(-9999.0, -9999.0, 0.0, 0.0);
                }
            }

            // Preview panel widgets layout: the form's stack, scrolled.
            let scroll_y = self.mid_region.scroll_y;
            let viewport_top = panel_y;
            let viewport_bottom = panel_y + content_h;
            let control_x = mid_panel_x + pad;
            let control_w = mid_panel_w - 2.0 * pad;

            // Open Folder & Remove Font buttons, a pane gap apart.
            let btn_h = cce_ui::layout::button_height();
            let btn_draw_y = panel_y + form.buttons_y - scroll_y;
            if btn_draw_y + btn_h >= viewport_top && btn_draw_y <= viewport_bottom {
                self.btn_open_folder.set_rect(control_x, btn_draw_y, 110.0, btn_h);
                self.btn_remove_font.set_rect(control_x + 110.0 + gap, btn_draw_y, 110.0, btn_h);
            } else {
                self.btn_open_folder.set_rect(-9999.0, -9999.0, 0.0, 0.0);
                self.btn_remove_font.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            // Style dropdown
            let dropdown_draw_y = panel_y + form.dropdown_y - scroll_y;
            if dropdown_draw_y + form.dropdown_h >= viewport_top && dropdown_draw_y <= viewport_bottom {
                self.style_dropdown.set_rect(control_x, dropdown_draw_y, control_w, form.dropdown_h);
            } else {
                self.style_dropdown.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            // Size spinbox
            let spinbox_draw_y = panel_y + form.spinbox_y - scroll_y;
            if spinbox_draw_y + form.spinbox_h >= viewport_top && spinbox_draw_y <= viewport_bottom {
                self.size_spinbox.set_rect(control_x, spinbox_draw_y, control_w, form.spinbox_h);
            } else {
                self.size_spinbox.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            // Preview box
            let preview_draw_y = panel_y + form.preview_y - scroll_y;
            if preview_draw_y + form.preview_h >= viewport_top && preview_draw_y <= viewport_bottom {
                self.preview_box.set_rect(control_x, preview_draw_y, control_w, form.preview_h);
            } else {
                self.preview_box.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            if self.select_mode {
                // The bottom bar's buttons, laid out directly (the layout Plate is
                // DISSOLVED): text-sized via the widgets' intrinsic size, packed
                // right — the pane padding off the bar's rim, a pane gap apart —
                // and vertically centered: the old row style.
                let btn_h = cce_ui::layout::button_height();
                let cancel_sz = self.select_cancel_btn.intrinsic_size()
                    .unwrap_or(cce_ui::scene::layout::Size::new(80.0, btn_h));
                let confirm_sz = self.select_confirm_btn.intrinsic_size()
                    .unwrap_or(cce_ui::scene::layout::Size::new(80.0, btn_h));
                let bar_w = w_f32 - 2.0 * inset;
                let mut x = left_panel_x + bar_w - pad - confirm_sz.width;
                self.select_confirm_btn.set_rect(x, bar_y + (select_bar_h - confirm_sz.height) / 2.0, confirm_sz.width, confirm_sz.height);
                x -= gap + cancel_sz.width;
                self.select_cancel_btn.set_rect(x, bar_y + (select_bar_h - cancel_sz.height) / 2.0, cancel_sz.width, cancel_sz.height);
            }

            self.refresh_widget_text();
            self.needs_rebuild = false;
            self.ui_context.rebuild_spatial_grid();
        }

        // Popover registration for the display-list text occlusion clamp. ui_context ONLY —
        // deliberately not the global popovers registry: this app draws its popover in the
        // display list below (not on an engine xdg popup), and a global registration would
        // spawn an empty popup surface (no render_popovers override here).
        self.ui_context.clear_popovers();
        if self.selected_family.is_some() && self.style_dropdown.popover_rect().is_some() {
            self.ui_context.register_popover(&mut self.style_dropdown);
        }

        // 1. The window base, then the dissolved panels' plates and their children
        // walked in the legacy panel order.
        use cce_ui::scene::layout::Rect;
        let mut pc = cce_ui::scene::paint::PaintCtx::new();
        // The standard root plate (cce-ui PlateSpec::window).
        pc.root_plate(w_f32, h_f32);
        {
            let self_ptr = self as *mut Self;
            // Left panel plate + children (the list's idle scrollbar under the plate).
            emit_region_idle_bar(&self.list_region, &mut pc);
            self.plate_prims(Rect { x: left_panel_x, y: panel_y, width: left_panel_w, height: content_h }, &mut pc);
            unsafe {
                cce_ui::scene::painter::paint_root_into(&self.ui_context, &(*self_ptr).search_box, &mut pc);
                // Rows under the list-viewport clip: `get_draw_y` returns
                // PARTIALLY visible rows (toolkit ScrollRegion intersection
                // contract), so an edge row renders cut instead of vanishing.
                // (Its full rect can overlap the search box's hit area above;
                // the router dispatches the search box first, so it wins.)
                let lr = &(*self_ptr).list_region;
                pc.push_clip(Rect { x: lr.x, y: lr.viewport_y, width: lr.w, height: lr.viewport_h });
                for btn in (*self_ptr).font_buttons.iter_mut() {
                    if btn.rect().0 > -9000.0 {
                        cce_ui::scene::painter::paint_root_into(&self.ui_context, &*btn, &mut pc);
                    }
                }
                pc.pop_clip();
            }
            // Mid panel plate + children (when a family is selected, like the old links).
            if self.selected_family.is_some() {
                emit_region_idle_bar(&self.mid_region, &mut pc);
            }
            self.plate_prims(Rect { x: mid_panel_x, y: panel_y, width: mid_panel_w, height: content_h }, &mut pc);
            if self.selected_family.is_some() {
                unsafe {
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, &(*self_ptr).btn_open_folder, &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, &(*self_ptr).btn_remove_font, &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, &(*self_ptr).style_dropdown, &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, &(*self_ptr).size_spinbox, &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, &(*self_ptr).preview_box, &mut pc);
                }
            }
            // Bottom bar plate + children (select mode).
            if self.select_mode {
                self.plate_prims(Rect { x: left_panel_x, y: bar_y, width: w_f32 - 2.0 * inset, height: select_bar_h }, &mut pc);
                unsafe {
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, &(*self_ptr).select_cancel_btn, &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, &(*self_ptr).select_confirm_btn, &mut pc);
                }
            }
        }

        // 4. Alphabet preview text prims (family + style/weight attrs), straight on the
        // mid plate: no box fill or outline of its own, so it wears the plate's colour.
        if self.selected_family.is_some() {
            self.push_alphabet_preview(&mut pc);
        }

        // The scrollbars' fore copies ride over the panel content (the idle
        // copies went under the panel plates);
        // the style dropdown's popover still stacks above them.
        emit_region_bar(&self.list_region, &mut pc);
        if self.selected_family.is_some() {
            emit_region_bar(&self.mid_region, &mut pc);
        }

        if self.select_mode {
            // Selected-font readout in the bottom bar: the pane padding off
            // its rim, the 12px text centered in the bar's height, the name
            // in a 100px column after the label.
            let readout_size = 12.0;
            let readout_y = bar_y + (select_bar_h - readout_size) / 2.0;
            pc.text_with(
                "Selected Font:",
                left_panel_x + pad,
                readout_y,
                readout_size,
                [0x5c, 0x90, 0x60],
                Some("monospace".to_string()),
                None,
            );
            let font_name = self.selected_family.clone().unwrap_or_else(|| "None".to_string());
            pc.text_with(
                font_name,
                left_panel_x + pad + 100.0,
                readout_y,
                readout_size,
                [0xdd, 0xdd, 0xe2],
                Some("monospace".to_string()),
                None,
            );
        }

        // 5. Style-dropdown popover — geometry and labels last, on top of everything. Its
        // labels carry bounds equal to the popover rect, which both clips them to the plate
        // and exempts them from the occlusion clamp (the is-overlay-text convention).
        if self.selected_family.is_some() && self.style_dropdown.open {
            // PaintCtx is a RenderTarget: the popover draws its real prims (the
            // expanded inset-plate surface) with its own per-label bounds.
            self.style_dropdown.render_popover(&mut pc);
        }

        Some(pc.finish())
    }

    fn handle_pointer_move(&mut self, pos: LogicalPosition, needs_rebuild: &mut bool) {
        let mut changed = false;
        let px = pos.x as f32;
        let py = pos.y as f32;

        // The shared context menu gets the pointer to itself while open (its
        // row highlight), as it gets the clicks (`handle_mouse_input`).
        if cce_ui::widget::context_menu::is_visible() {
            if cce_ui::widget::context_menu::cursor_moved(px, py) {
                *needs_rebuild = true;
                self.needs_rebuild = true;
            }
            return;
        }

        let old_size = self.preview_box.font_size;

        // The dissolved scroll regions: scrollbar-thumb drags and hover tracking.
        // Toolkit cursor_moved = the old drag_move + manual hover tracking.
        if self.list_region.cursor_moved(px, py) {
            changed = true;
        }
        if self.mid_region.cursor_moved(px, py) {
            changed = true;
        }

        // Routed dispatch (6bd shrink): one PointerMove through the router per roster
        // root — hover bookkeeping plus the router's drag forwarding (replaces the
        // is_dragging -> drag_update pass; DragUpdate reaches the drag target off-rect).
        let ev = Event::PointerMove { x: px, y: py, local_x: px, local_y: py };
        for root in self.dispatch_widgets(true) {
            if self.ui_context.propagate_event(&ev, root) {
                changed = true;
            }
        }

        let new_size = self.size_spinbox.value as f32;
        if (new_size - old_size).abs() > 0.001 {
            self.preview_box.font_size = new_size;
            changed = true;
        }

        if changed {
            *needs_rebuild = true;
            self.needs_rebuild = true;
        }
    }

    fn handle_mouse_input(&mut self, button: MouseButton, state: ElementState, pos: LogicalPosition, needs_rebuild: &mut bool) -> Option<Self::Message> {
        let mut changed = false;
        let mut msg_out = None;
        let px = pos.x as f32;
        let py = pos.y as f32;

        // The toolkit's shared context menu (the size spinbox's, the text boxes')
        // takes every click while it is open: a row runs its action, a press
        // anywhere else dismisses it. The toolkit leaves this routing to the app,
        // and without it the menu could not be closed by clicking outside it.
        if cce_ui::widget::context_menu::is_visible() {
            if cce_ui::widget::context_menu::mouse_input(button, state, px, py, Some(&mut self.ui_context)) {
                *needs_rebuild = true;
                self.needs_rebuild = true;
            }
            // A size pasted into the spinbox from the menu.
            let new_size = self.size_spinbox.value as f32;
            if (new_size - self.preview_box.font_size).abs() > 0.001 {
                self.preview_box.font_size = new_size;
                return Some(AppMessage::FontSizeChanged);
            }
            return None;
        }

        if state == ElementState::Pressed && button == MouseButton::Left {
            if !self.search_box.hit_test(px, py, &self.ui_context) {
                self.search_box.unfocus();
                changed = true;
            } else {
                self.ui_context.set_focused(&mut self.search_box);
            }
            if !self.preview_box.hit_test(px, py, &self.ui_context) {
                self.preview_box.unfocus();
                changed = true;
            } else {
                self.ui_context.set_focused(&mut self.preview_box);
            }
        }

        // The dissolved scroll regions' scrollbars (thumb grab / track jump / release).
        let mut region_handled = false;
        if button == MouseButton::Left {
            match state {
                ElementState::Pressed => {
                    if self.list_region.press(px, py) || (self.selected_family.is_some() && self.mid_region.press(px, py)) {
                        changed = true;
                        region_handled = true;
                    }
                }
                ElementState::Released => {
                    self.list_region.release();
                    self.mid_region.release();
                }
            }
        }

        // The dissolved panels' press routing, flattened and ROUTED (6bd shrink):
        // popover-first (an open dropdown must see the click before anything beneath),
        // then reverse order with the Plates' unfocus-on-missed-press rule, stopping at
        // the first handler. The router hit-gates presses and records the drag target.
        let ev = Event::MouseButton { button, state, x: px, y: py, local_x: px, local_y: py };
        let widgets = self.dispatch_widgets(false);
        let mut input_handled = region_handled;
        for &root in &widgets {
            if input_handled {
                break;
            }
            let has_popover = self.ui_context.get_widget(root).map_or(false, |w| w.popover_rect().is_some());
            if has_popover {
                if self.ui_context.propagate_event(&ev, root) {
                    changed = true;
                    input_handled = true;
                    break;
                }
            }
        }
        if !input_handled {
            for &root in &widgets {
                if self.ui_context.propagate_event(&ev, root) {
                    changed = true;
                    break;
                }
                if state == ElementState::Pressed {
                    let missed = self.ui_context.get_widget(root).map_or(false, |w| !w.hit_test(px, py, &self.ui_context));
                    if missed {
                        if let Some(w) = self.ui_context.get_widget_mut(root) {
                            w.unfocus();
                        }
                    }
                }
            }
        }

        if self.selected_family.is_some() && self.style_dropdown.take_change() {
            msg_out = Some(AppMessage::SelectStyle(self.style_dropdown.selected));
        }

        for (idx, btn) in self.font_buttons.iter_mut().enumerate() {
            if btn.rect().0 > -9000.0 && btn.take_click() {
                let double = self.last_click_at.is_some_and(|t| t.elapsed() < DOUBLE_CLICK);
                if self.select_mode && self.last_click_idx == Some(idx) && double {
                    let selected = self.filtered[idx].clone();
                    print!("{} {}", selected, self.size_spinbox.value);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    std::process::exit(0);
                }
                self.last_click_idx = Some(idx);
                self.last_click_at = Some(std::time::Instant::now());
                msg_out = Some(AppMessage::SelectFamily(idx));
                break;
            }
        }

        let old_size = self.preview_box.font_size;
        let new_size = self.size_spinbox.value as f32;
        if (new_size - old_size).abs() > 0.001 {
            self.preview_box.font_size = new_size;
            msg_out = Some(AppMessage::FontSizeChanged);
        }

        if self.btn_open_folder.take_click() {
            msg_out = Some(AppMessage::OpenFolder);
        }
        if self.btn_remove_font.take_click() {
            msg_out = Some(AppMessage::RemoveFont);
        }

        if self.select_mode {
            if self.select_cancel_btn.take_click() {
                std::process::exit(1);
            }
            if self.select_confirm_btn.take_click() {
                let selected = self.selected_family.clone().unwrap_or_default();
                print!("{} {}", selected, self.size_spinbox.value);
                use std::io::Write;
                let _ = std::io::stdout().flush();
                std::process::exit(0);
            }
        }

        if changed {
            *needs_rebuild = true;
            self.needs_rebuild = true;
        }

        msg_out
    }

    fn handle_mouse_wheel(&mut self, delta: &MouseScrollDelta, pos: LogicalPosition, needs_rebuild: &mut bool) {
        let mut changed = false;
        let px = pos.x as f32;
        let py = pos.y as f32;

        if self.list_region.wheel(delta, px, py) {
            changed = true;
        }
        if self.selected_family.is_some() && self.mid_region.wheel(delta, px, py) {
            changed = true;
        }
        // Routed (6bd shrink): every roster root sees the wheel, as the legacy
        // no-break loop did; each widget hit-gates internally.
        let ev = Event::MouseWheel { delta: *delta, x: px, y: py, local_x: px, local_y: py };
        for root in self.dispatch_widgets(true) {
            if self.ui_context.propagate_event(&ev, root) {
                changed = true;
            }
        }

        if changed {
            *needs_rebuild = true;
            self.needs_rebuild = true;
        }
    }

    fn handle_key_input(&mut self, event: &KeyEvent, needs_rebuild: &mut bool) -> Option<Self::Message> {
        let mut handled = false;
        let mut msg_out = None;

        // Custom keyboard shortcuts (input.kdl `cce-fonts` domain)
        if event.state == ElementState::Pressed {
            let m = |chord: &str| cce_ui::widget::match_key_shortcut(event, chord);
            if m(&self.keys.refresh) {
                msg_out = Some(AppMessage::RefreshFonts);
                handled = true;
            } else if m(&self.keys.open_search) {
                self.search_box.focus();
                self.ui_context.set_focused(&mut self.search_box);
                self.preview_box.unfocus();
                handled = true;
            } else if m(&self.keys.focus_preview) {
                self.preview_box.focus();
                self.ui_context.set_focused(&mut self.preview_box);
                self.search_box.unfocus();
                handled = true;
            } else if m(&self.keys.open_folder) {
                msg_out = Some(AppMessage::OpenFolder);
                handled = true;
            } else if m(&self.keys.remove_font) {
                msg_out = Some(AppMessage::RemoveFont);
                handled = true;
            }
        }

        if !handled && event.state == ElementState::Pressed {
            match event.logical_key {
                Key::Character(ref ch) if ch == "+" || ch == "=" => {
                    if self.selected_family.is_some() {
                        let (_, max) = self.size_spinbox.range();
                        let old_val = self.size_spinbox.value;
                        self.size_spinbox.value = (self.size_spinbox.value + 2).min(max);
                        if self.size_spinbox.value != old_val && self.size_spinbox.editing {
                            self.size_spinbox.edit_buffer = self.size_spinbox.value.to_string();
                        }
                        msg_out = Some(AppMessage::FontSizeChanged);
                        handled = true;
                    }
                }
                Key::Character(ref ch) if ch == "-" => {
                    if self.selected_family.is_some() {
                        let (min, _) = self.size_spinbox.range();
                        let old_val = self.size_spinbox.value;
                        self.size_spinbox.value = (self.size_spinbox.value - 2).max(min);
                        if self.size_spinbox.value != old_val && self.size_spinbox.editing {
                            self.size_spinbox.edit_buffer = self.size_spinbox.value.to_string();
                        }
                        msg_out = Some(AppMessage::FontSizeChanged);
                        handled = true;
                    }
                }
                _ => {}
            }
        }

        if !handled && !self.style_dropdown.open {
            // Arrow navigation — family list only while no dropdown is open:
            // an open style dropdown takes the arrows for its own hover (via
            // the propagate loop below), and consuming them here left it
            // keyboard-navigable in every way except the one that matters.
            let direction = match event.logical_key {
                Key::Named(NamedKey::ArrowUp) => Some(BrowseNavigation::Up),
                Key::Named(NamedKey::ArrowDown) => Some(BrowseNavigation::Down),
                _ => None,
            };

            if let Some(dir) = direction {
                if event.state == ElementState::Pressed && !self.filtered.is_empty() {
                    let len = self.filtered.len();
                    let next_idx = match (self.selected_idx, dir) {
                        (Some(idx), BrowseNavigation::Up) => idx.saturating_sub(1),
                        (Some(idx), BrowseNavigation::Down) => (idx + 1).min(len - 1),
                        (None, BrowseNavigation::Up) => len - 1,
                        (None, BrowseNavigation::Down) => 0,
                    };
                    msg_out = Some(AppMessage::SelectFamily(next_idx));
                    handled = true;
                }
            }
        }

        // TextBox and other focused widgets input propagation
        if !handled {
            let old_search_text = if self.search_box.editing { self.search_box.edit_buffer.clone() } else { self.search_box.text.clone() };
            
            // Routed (6bd shrink); short-circuits on the first handler — the router
            // delivers KeyInput to the focused widget first on EVERY call (the 6ac trap).
            let ev = Event::KeyInput(event.clone());
            for root in self.dispatch_widgets(true) {
                if self.ui_context.propagate_event(&ev, root) {
                    handled = true;
                    break;
                }
            }
            if !handled && self.list_region.keyboard(event) {
                handled = true;
            }
            if !handled && self.selected_family.is_some() && self.mid_region.keyboard(event) {
                handled = true;
            }

            if handled {
                let new_search_text = if self.search_box.editing { &self.search_box.edit_buffer } else { &self.search_box.text };
                if old_search_text != *new_search_text {
                    msg_out = Some(AppMessage::SearchChanged);
                }
            }
        }

        if handled {
            *needs_rebuild = true;
            self.needs_rebuild = true;
        }

        msg_out
    }
}

fn main() {
    cce_ui::engine::run::<TypefaceApp>();
}
