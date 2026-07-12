mod pages;

use wayland_client::QueueHandle;
use glyphon::FontSystem;
use cce_ui::engine::{Application, EngineState, LogicalPosition, LogicalSize, WindowSettings};
use cce_ui::widget::{
    MouseButton, ElementState, MouseScrollDelta, KeyEvent, Element,
    TextBox, Button, Key, NamedKey, Dropdown, Spinbox,
};



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

/// App-owned scroll region replacing the dissolved ScrollBox / List (fonts used List with
/// columns=None — a pure scroll frame; the rows here are standalone Button widgets). State,
/// wheel/scrollbar-drag/keyboard behavior, and the bg/track/thumb prims replicate the
/// legacy widgets verbatim.
struct ScrollRegion {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    scroll_y: f32,
    content_h: f32,
    viewport_y: f32,
    viewport_h: f32,
    dragging: bool,
    drag_offset_y: f32,
    hovered: bool,
}

impl ScrollRegion {
    fn new() -> Self {
        Self {
            x: 0.0, y: 0.0, w: 0.0, h: 0.0,
            scroll_y: 0.0, content_h: 0.0, viewport_y: 0.0, viewport_h: 0.0,
            dragging: false, drag_offset_y: 0.0, hovered: false,
        }
    }

    fn set_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.x = x; self.y = y; self.w = w; self.h = h;
    }

    fn update_bounds(&mut self, content_h: f32, viewport_y: f32, viewport_h: f32) {
        self.content_h = content_h;
        self.viewport_y = viewport_y;
        self.viewport_h = viewport_h;
        self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll());
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h - self.viewport_h).max(0.0)
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    fn get_item_draw_y(&self, virtual_y: f32, item_h: f32) -> Option<f32> {
        let draw_y = self.viewport_y + virtual_y - self.scroll_y;
        if draw_y >= self.viewport_y - 1.0 && draw_y + item_h <= self.viewport_y + self.viewport_h + 1.0 {
            Some(draw_y)
        } else {
            None
        }
    }

    /// Scrollbar geometry: (sb_x, track_y, sb_w, track_h, thumb_y, thumb_h).
    fn scrollbar_geom(&self) -> (f32, f32, f32, f32, f32, f32) {
        let sb_w = cce_ui::layout::scrollbar_width();
        let sb_x = self.x + self.w - sb_w - 4.0;
        let track_h = self.viewport_h - 8.0;
        let track_y = self.viewport_y + 4.0;
        let visible_ratio = self.viewport_h / self.content_h.max(1.0);
        let thumb_h = if track_h <= 20.0 { track_h } else { (track_h * visible_ratio).clamp(20.0, track_h) };
        let scroll_ratio = if self.max_scroll() > 0.0 { self.scroll_y / self.max_scroll() } else { 0.0 };
        let thumb_y = track_y + scroll_ratio * (track_h - thumb_h);
        (sb_x, track_y, sb_w, track_h, thumb_y, thumb_h)
    }

    fn hit_scrollbar(&self, px: f32, py: f32) -> bool {
        if self.content_h <= self.viewport_h {
            return false;
        }
        let (sb_x, track_y, sb_w, track_h, _, _) = self.scrollbar_geom();
        px >= sb_x - 4.0 && px <= sb_x + sb_w + 4.0 && py >= track_y && py <= track_y + track_h
    }

    /// Left press: scrollbar thumb grab or track jump (the legacy ScrollBox::mouse_input).
    fn press(&mut self, px: f32, py: f32) -> bool {
        if !self.hit_scrollbar(px, py) {
            self.dragging = false;
            return false;
        }
        self.dragging = true;
        let (_, track_y, _, track_h, thumb_y, thumb_h) = self.scrollbar_geom();
        let click_offset = py - thumb_y;
        if click_offset >= 0.0 && click_offset <= thumb_h {
            self.drag_offset_y = click_offset;
        } else {
            self.drag_offset_y = thumb_h / 2.0;
            let target = py - self.drag_offset_y;
            let ratio = if track_h - thumb_h > 0.0 {
                ((target - track_y) / (track_h - thumb_h)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            self.scroll_y = ratio * self.max_scroll();
        }
        true
    }

    fn release(&mut self) {
        self.dragging = false;
    }

    fn drag_move(&mut self, py: f32) -> bool {
        if !self.dragging {
            return false;
        }
        let (_, track_y, _, track_h, _, thumb_h) = self.scrollbar_geom();
        let target = py - self.drag_offset_y;
        let ratio = if track_h - thumb_h > 0.0 {
            ((target - track_y) / (track_h - thumb_h)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let old = self.scroll_y;
        self.scroll_y = ratio * self.max_scroll();
        (self.scroll_y - old).abs() > 0.01
    }

    fn wheel(&mut self, delta: &MouseScrollDelta, px: f32, py: f32) -> bool {
        if !self.hit(px, py) {
            return false;
        }
        let dy = match delta {
            MouseScrollDelta::LineDelta(_, y) => -y * 24.0,
            MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
        };
        let old = self.scroll_y;
        self.scroll_y = (self.scroll_y + dy).clamp(0.0, self.max_scroll());
        (self.scroll_y - old).abs() > 0.01
    }

    /// Hover-scoped keyboard scrolling (the legacy ScrollBox handled these when focused or
    /// hovered; the dissolved region keeps the hover half, which is how it was reached here).
    fn keyboard(&mut self, event: &KeyEvent) -> bool {
        if !self.hovered || event.state != ElementState::Pressed {
            return false;
        }
        let max = self.max_scroll();
        let old = self.scroll_y;
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.scroll_y = (self.scroll_y + 24.0).clamp(0.0, max),
            Key::Named(NamedKey::ArrowUp) => self.scroll_y = (self.scroll_y - 24.0).clamp(0.0, max),
            Key::Named(NamedKey::PageDown) => self.scroll_y = (self.scroll_y + self.viewport_h).clamp(0.0, max),
            Key::Named(NamedKey::PageUp) => self.scroll_y = (self.scroll_y - self.viewport_h).clamp(0.0, max),
            Key::Named(NamedKey::Home) => self.scroll_y = 0.0,
            Key::Named(NamedKey::End) => self.scroll_y = max,
            _ => return false,
        }
        (self.scroll_y - old).abs() > 0.01
    }

    /// The legacy paint, verbatim: plain bg quad (ScrollBox::extra_quads) — plus, for the
    /// List flavor, the rounded bg the walk's leaf branch emitted from all_rounded_quads —
    /// then the scrollbar track and thumb.
    fn push_prims(&self, rounded_list_frame: bool, pc: &mut cce_ui::scene::paint::PaintCtx) {
        use cce_ui::scene::layout::Rect;
        let rect = Rect { x: self.x, y: self.y, width: self.w, height: self.h };
        let _ = rounded_list_frame;
        pc.quad(rect, cce_ui::color::list_bg_color());
        if self.content_h > self.viewport_h {
            let (sb_x, track_y, sb_w, track_h, thumb_y, thumb_h) = self.scrollbar_geom();
            pc.quad(Rect { x: sb_x, y: track_y, width: sb_w, height: track_h }, cce_ui::color::scrollbar_track_color());
            pc.quad(Rect { x: sb_x, y: thumb_y, width: sb_w, height: thumb_h }, cce_ui::color::scrollbar_thumb_color());
        }
    }
}

struct TypefaceApp {
    // Browse panel
    search_box: cce_ui::widget::Adapted<TextBox>,
    list_region: ScrollRegion,
    list_item_h: f32,
    font_buttons: Vec<cce_ui::widget::Adapted<cce_ui::widget::Button>>,

    // Preview panel
    style_dropdown: cce_ui::widget::Adapted<Dropdown>,
    size_spinbox: cce_ui::widget::Adapted<cce_ui::widget::Spinbox>,
    preview_box: cce_ui::widget::Adapted<TextBox>,

    // Details panel
    btn_open_folder: cce_ui::widget::Adapted<cce_ui::widget::Button>,
    btn_remove_font: cce_ui::widget::Adapted<cce_ui::widget::Button>,

    // Selection mode buttons
    select_cancel_btn: cce_ui::widget::Adapted<cce_ui::widget::Button>,
    select_confirm_btn: cce_ui::widget::Adapted<cce_ui::widget::Button>,
    select_mode: bool,
    last_click_idx: Option<usize>,
    click_timer: f32,

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

    // Containers (root Backplate, the three Plates, and the ScrollBox/List are DISSOLVED:
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
        let self_ptr = self as *mut Self;
        unsafe {
            if !self.widgets_registered {
                self.widgets_registered = true;
                self.ui_context.register_widget(self.search_box.base().unwrap().id(), (*self_ptr).search_box.as_ptr_mut());
                self.ui_context.register_widget(self.btn_open_folder.base().unwrap().id(), (*self_ptr).btn_open_folder.as_ptr_mut());
                self.ui_context.register_widget(self.btn_remove_font.base().unwrap().id(), (*self_ptr).btn_remove_font.as_ptr_mut());
                self.ui_context.register_widget(self.style_dropdown.base().unwrap().id(), (*self_ptr).style_dropdown.as_ptr_mut());
                self.ui_context.register_widget(self.size_spinbox.base().unwrap().id(), (*self_ptr).size_spinbox.as_ptr_mut());
                self.ui_context.register_widget(self.preview_box.base().unwrap().id(), (*self_ptr).preview_box.as_ptr_mut());
                self.ui_context.register_widget(self.select_cancel_btn.base().unwrap().id(), (*self_ptr).select_cancel_btn.as_ptr_mut());
                self.ui_context.register_widget(self.select_confirm_btn.base().unwrap().id(), (*self_ptr).select_confirm_btn.as_ptr_mut());
            }
            for btn in (*self_ptr).font_buttons.iter_mut() {
                if btn.rect().0 > -9000.0 {
                    self.ui_context.register_widget(btn.base().unwrap().id(), btn.as_ptr_mut());
                }
            }
        }
    }

    fn content_h(&self) -> f32 {
        let h = self.height as f32;
        if self.select_mode {
            (h - 30.0 - 48.0).max(100.0)
        } else {
            (h - 20.0).max(100.0)
        }
    }

    /// The dissolved mid panel's rect (was `mid_panel.rect()`).
    fn mid_panel_rect(&self) -> (f32, f32, f32, f32) {
        let left_panel_w = 270.0;
        let mid_x = 10.0 + left_panel_w + 12.0;
        (mid_x, 10.0, self.width as f32 - 10.0 - mid_x, self.content_h())
    }

    /// The dissolved Plates' visual (blur off, non-draggable): config plate color else
    /// page-low, at plate opacity, with the config border and corner radius.
    fn plate_prims(&self, rect: cce_ui::scene::layout::Rect, pc: &mut cce_ui::scene::paint::PaintCtx) {
        let mut fill = cce_ui::colors::plate_color().unwrap_or_else(cce_ui::colors::page_low_color);
        fill[3] *= cce_ui::layout::plate_opacity();
        let radius = cce_ui::layout::plate_corner_radius();
        let radii = (radius, radius, radius, radius);
        if let Some(bc) = cce_ui::colors::plate_border_color() {
            pc.border(rect, radii, fill, bc, cce_ui::colors::plate_border_thickness());
        } else if fill[3].abs() > 0.001 {
            if radius > 0.1 {
                pc.rounded_rect(rect, radius, (true, true, true, true), fill);
            } else {
                pc.quad(rect, fill);
            }
        }
    }

    /// Event dispatch order of the dissolved panels: the flat child list, panel-grouped
    /// (left: search/list/buttons; mid, when a family is selected; bottom bar in select
    /// mode) — the same sets the Plates forwarded to.
    fn dispatch_widgets(&mut self, forward: bool) -> Vec<*mut (dyn Element + 'static)> {
        let self_ptr = self as *mut Self;
        let mut v: Vec<*mut (dyn Element + 'static)> = Vec::new();
        unsafe {
            v.push((*self_ptr).search_box.as_ptr_mut());
            for btn in (*self_ptr).font_buttons.iter_mut() {
                if btn.rect().0 > -9000.0 {
                    v.push(btn.as_ptr_mut());
                }
            }
            if self.selected_family.is_some() {
                v.push((*self_ptr).btn_open_folder.as_ptr_mut());
                v.push((*self_ptr).btn_remove_font.as_ptr_mut());
                v.push((*self_ptr).style_dropdown.as_ptr_mut());
                v.push((*self_ptr).size_spinbox.as_ptr_mut());
                v.push((*self_ptr).preview_box.as_ptr_mut());
            }
            if self.select_mode {
                v.push((*self_ptr).select_cancel_btn.as_ptr_mut());
                v.push((*self_ptr).select_confirm_btn.as_ptr_mut());
            }
        }
        if !forward {
            v.reverse();
        }
        v
    }

    fn reload_fonts(&mut self) {
        self.all_fonts = pages::fetch_fonts();
        self.families = self.extract_families(&self.all_fonts);
        let query = if self.search_box.editing { &self.search_box.edit_buffer } else { &self.search_box.text };
        self.filtered = self.filter_families(&self.families, query);
        self.font_buttons = self.filtered.iter().map(|f| Button::new_list_row(0.0, 0.0, 0.0, 0.0).with_label(f)).collect();
        
        if let Some(sel) = self.selected_idx {
            if sel >= self.filtered.len() {
                self.selected_idx = None;
                self.selected_family = None;
                self.selected_style = None;
                self.selected_file = None;
                self.family_styles.clear();
                self.family_files.clear();
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
        self.preview_box.font_family = family.clone();
        
        // Find styles and files
        let entries: Vec<&pages::FontEntry> = self.all_fonts.iter().filter(|f| f.family == family).collect();
        
        let mut styles = Vec::new();
        let mut files = Vec::new();
        for e in entries {
            styles.push(e.style.clone());
            files.push(e.file.clone());
        }
        
        self.family_styles = styles.clone();
        self.family_files = files.clone();
        
        // Pick default style: "Regular" if available, else first
        let default_idx = styles.iter().position(|s| s == "Regular").unwrap_or(0);
        
        self.style_dropdown.options = styles;
        self.style_dropdown.selected = default_idx;
        
        if !self.family_styles.is_empty() {
            let style = self.family_styles[default_idx].clone();
            let file = self.family_files[default_idx].clone();
            self.selected_style = Some(style);
            self.selected_file = Some(file.clone());
            
            self.charset_count = pages::count_chars(&file);
            self.charset_str = self.charset_count.to_string();
            self.is_user_font = pages::is_user_font(&file);
        } else {
            self.selected_style = None;
            self.selected_file = None;
            self.charset_count = 0;
            self.charset_str = String::from("0");
            self.is_user_font = false;
        }

        // Highlight selected button
        if let Some(sel) = self.selected_idx {
            for (idx, btn) in self.font_buttons.iter_mut().enumerate() {
                btn.selected = idx == sel;
            }
        }
        
        self.needs_rebuild = true;
    }

    fn scroll_to_index(&mut self, index: usize) {
        let item_height_full = 24.0 + 4.0; // legacy hardcoded item_height + item_gap
        let top = index as f32 * item_height_full;
        let bottom = top + 24.0;
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
        let Some(ref family) = self.selected_family else { return };

        let font_size = self.size_spinbox.value as f32;
        let mut attrs = cce_ui::scene::paint::TextAttrs::default();
        if let Some(ref style) = self.selected_style {
            let sl = style.to_lowercase();
            if sl.contains("italic") || sl.contains("oblique") {
                attrs.italic = true;
            }
            if sl.contains("bold") {
                attrs.weight = Some(700);
            } else if sl.contains("light") {
                attrs.weight = Some(300);
            } else if sl.contains("medium") {
                attrs.weight = Some(500);
            }
        }

        let (mid_panel_x, _, mid_panel_w, mid_panel_h) = self.mid_panel_rect();
        let preview_box_x = mid_panel_x + 10.0;
        let preview_box_w = mid_panel_w - 20.0;

        let alphabet_virtual_y = if self.select_mode { 320.0 } else { 380.0 };
        let alphabet_box_h = if self.select_mode { 102.0 } else { 120.0 };
        let scroll_y = self.mid_region.scroll_y;
        let alphabet_draw_y = 10.0 + alphabet_virtual_y - scroll_y;

        let viewport_top = 10.0;
        let viewport_bottom = 10.0 + mid_panel_h;
        if alphabet_draw_y + alphabet_box_h < viewport_top || alphabet_draw_y > viewport_bottom {
            return;
        }
        let bounds_y_start = alphabet_draw_y.max(viewport_top);
        let bounds_y_end = (alphabet_draw_y + alphabet_box_h).min(viewport_bottom);
        let bounds = Some([preview_box_x, bounds_y_start, preview_box_x + preview_box_w, bounds_y_end]);

        let size_used = (font_size * 0.55).clamp(8.0, 36.0);
        let lines = [
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            "abcdefghijklmnopqrstuvwxyz",
            "0123456789",
            "!@#$%^&*()_+-=[]{}|;':\",./<>",
        ];
        for (i, line) in lines.iter().enumerate() {
            pc.text_attrs(
                *line,
                mid_panel_x + 20.0,
                alphabet_draw_y + 12.0 + i as f32 * size_used * 1.4,
                size_used,
                [0x88, 0x88, 0x99],
                Some(family.clone()),
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

    /// The font picker previews arbitrary installed families: the engine's render FontSystem
    /// must contain the system fonts, or preview text asking for a system-only family is
    /// silently invisible.
    fn load_system_fonts(&self) -> bool {
        true
    }

    fn display_list_text(&self) -> bool {
        true
    }

    fn is_movable_backplate_at(&self, px: f32, py: f32) -> bool {
        // Root Backplate dissolved: the surface itself is the movable plate.
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

        let mut preview_box = TextBox::new(String::from("The quick brown fox jumps over the lazy dog")).with_multiline(true).with_draw_bg_border(true).with_max_width(None);
        preview_box.font_size = 32.0;

        let btn_open_folder = Button::new(914.0, 200.0, 110.0, 28.0).with_label("Open Folder");
        let btn_remove_font = Button::new_reset(1034.0, 200.0, 110.0, 28.0).with_label("Remove Font");

        let select_cancel_btn = Button::new_reset(0.0, 0.0, 80.0, 28.0).with_label("Cancel");
        let select_confirm_btn = Button::new(0.0, 0.0, 80.0, 28.0).with_label("Select");

        let all_fonts = pages::fetch_fonts();
        let mut app = Self {
            search_box,
            list_region: ScrollRegion::new(),
            list_item_h,
            font_buttons: Vec::new(),
            style_dropdown,
            size_spinbox,
            preview_box,
            btn_open_folder,
            btn_remove_font,
            select_cancel_btn,
            select_confirm_btn,
            select_mode,
            last_click_idx: None,
            click_timer: 0.0,
            all_fonts: all_fonts.clone(),
            families: Vec::new(),
            filtered: Vec::new(),
            selected_idx: None,
            selected_family: None,
            selected_style: None,
            selected_file: None,
            family_styles: Vec::new(),
            family_files: Vec::new(),
            charset_count: 0,
            charset_str: String::from("0"),
            is_user_font: false,
            width: if select_mode { 900 } else { 1200 },
            height: if select_mode { 500 } else { 720 },
            scale_factor: 1.0,
            font_system: cce_ui::create_font_system_with_system_fonts(),
            needs_rebuild: true,
            mid_region: ScrollRegion::new(),
            widgets_registered: false,
            ui_context: cce_ui::context::UiContext::new(),
        };
        
        app.families = app.extract_families(&app.all_fonts);
        app.filtered = app.filter_families(&app.families, "");
        app.font_buttons = app.filtered.iter().map(|f| Button::new_list_row(0.0, 0.0, 0.0, 0.0).with_label(f)).collect();
        
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
                title: "CCE Fonts".to_string(),
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
                    self.style_dropdown.selected = idx;
                    let style = self.family_styles[idx].clone();
                    let file = self.family_files[idx].clone();
                    self.selected_style = Some(style);
                    self.selected_file = Some(file.clone());
                    self.charset_count = pages::count_chars(&file);
                    self.charset_str = self.charset_count.to_string();
                    self.is_user_font = pages::is_user_font(&file);
                    *needs_rebuild = true;
                    self.needs_rebuild = true;
                }
            }
            AppMessage::FontSizeChanged => {
                self.preview_box.font_size = self.size_spinbox.value as f32;
                *needs_rebuild = true;
                self.needs_rebuild = true;
            }
            AppMessage::SearchChanged => {
                let query = if self.search_box.editing { &self.search_box.edit_buffer } else { &self.search_box.text };
                self.filtered = self.filter_families(&self.families, query);
                self.font_buttons = self.filtered.iter().map(|f| Button::new_list_row(0.0, 0.0, 0.0, 0.0).with_label(f)).collect();
                
                // reset or re-evaluate selection
                self.selected_idx = None;
                self.selected_family = None;
                self.selected_style = None;
                self.selected_file = None;
                self.family_styles.clear();
                self.family_files.clear();
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

    fn tick(&mut self, dt: f32, _needs_rebuild: &mut bool) {
        if self.click_timer > 0.0 {
            self.click_timer -= dt;
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

        let left_panel_x = 10.0;
        let left_panel_w = 270.0;
        let mid_panel_x = left_panel_x + left_panel_w + 12.0;
        let mid_panel_w = w_f32 - 10.0 - mid_panel_x;

        let select_bar_h = 48.0;
        let content_h = if self.select_mode {
            (h_f32 - 30.0 - select_bar_h).max(100.0)
        } else {
            (h_f32 - 20.0).max(100.0)
        };

        if self.needs_rebuild || size_changed {
            cce_ui::scale::set_scale_factor(scale as f32);
            self.register_widgets();

            self.mid_region.set_rect(mid_panel_x, 10.0, mid_panel_w, content_h);

            let mid_content_h = if self.select_mode { 440.0 } else { 520.0 };
            self.mid_region.update_bounds(mid_content_h, 10.0, content_h);

            let bar_y = h_f32 - select_bar_h - 10.0;

            // Search box
            self.search_box.set_rect(left_panel_x + 10.0, 10.0, left_panel_w - 20.0, 26.0);

            // Scrolling List
            let list_x = left_panel_x + 10.0;
            let list_y = 46.0;
            let list_w = left_panel_w - 20.0;
            let list_h = (content_h - 36.0).max(100.0);
            self.list_region.set_rect(list_x, list_y, list_w, list_h);
            // The dissolved List's content math: count * (adjusted item height + gap) + 4.
            let item_full = self.list_item_h + 4.0;
            self.list_region.update_bounds(self.filtered.len() as f32 * item_full + 4.0, list_y, list_h);

            // Layout list buttons
            for (idx, btn) in self.font_buttons.iter_mut().enumerate() {
                let item_full = self.list_item_h + 4.0;
                if let Some(draw_y) = self.list_region.get_item_draw_y(idx as f32 * item_full, self.list_item_h) {
                    btn.set_rect(list_x + 4.0, draw_y, list_w - 8.0, 24.0);
                } else {
                    btn.set_rect(-9999.0, -9999.0, 0.0, 0.0);
                }
            }

            // Preview panel widgets layout
            let style_dropdown_h = cce_ui::layout::dropdown_height() + cce_ui::widget::label_offset(&self.style_dropdown);
            let size_spinbox_h = cce_ui::layout::spinbox_height() + cce_ui::widget::label_offset(&self.size_spinbox);
            let preview_box_h = if self.select_mode { 120.0 } else { 180.0 };

            let scroll_y = self.mid_region.scroll_y;
            let viewport_top = 10.0;
            let viewport_bottom = 10.0 + content_h;

            // Open Folder & Remove Font buttons (virtual_y = 20.0, h = 28.0)
            let btn_draw_y = 10.0 + 20.0 - scroll_y;
            if btn_draw_y + 28.0 >= viewport_top && btn_draw_y <= viewport_bottom {
                self.btn_open_folder.set_rect(mid_panel_x + 10.0, btn_draw_y, 110.0, 28.0);
                self.btn_remove_font.set_rect(mid_panel_x + 130.0, btn_draw_y, 110.0, 28.0);
            } else {
                self.btn_open_folder.set_rect(-9999.0, -9999.0, 0.0, 0.0);
                self.btn_remove_font.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            // Style dropdown (virtual_y = 65.0, h = style_dropdown_h)
            let dropdown_draw_y = 10.0 + 65.0 - scroll_y;
            if dropdown_draw_y + style_dropdown_h >= viewport_top && dropdown_draw_y <= viewport_bottom {
                self.style_dropdown.set_rect(mid_panel_x + 10.0, dropdown_draw_y, mid_panel_w - 20.0, style_dropdown_h);
            } else {
                self.style_dropdown.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            // Size spinbox (virtual_y = 120.0, h = size_spinbox_h)
            let spinbox_draw_y = 10.0 + 120.0 - scroll_y;
            if spinbox_draw_y + size_spinbox_h >= viewport_top && spinbox_draw_y <= viewport_bottom {
                self.size_spinbox.set_rect(mid_panel_x + 10.0, spinbox_draw_y, mid_panel_w - 20.0, size_spinbox_h);
            } else {
                self.size_spinbox.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            // Preview box (virtual_y = 190.0, h = preview_box_h)
            let preview_draw_y = 10.0 + 190.0 - scroll_y;
            if preview_draw_y + preview_box_h >= viewport_top && preview_draw_y <= viewport_bottom {
                self.preview_box.set_rect(mid_panel_x + 10.0, preview_draw_y, mid_panel_w - 20.0, preview_box_h);
            } else {
                self.preview_box.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            if self.select_mode {
                // The bottom bar's buttons, laid out directly (the layout Plate is
                // DISSOLVED): text-sized via the bridge's Element::intrinsic_size, packed
                // right with a 10px gap and inset, vertically centered — the old row style.
                let cancel_sz = cce_ui::widget::Element::intrinsic_size(&self.select_cancel_btn)
                    .unwrap_or(cce_ui::scene::layout::Size::new(80.0, 28.0));
                let confirm_sz = cce_ui::widget::Element::intrinsic_size(&self.select_confirm_btn)
                    .unwrap_or(cce_ui::scene::layout::Size::new(80.0, 28.0));
                let bar_w = w_f32 - 20.0;
                let mut x = left_panel_x + bar_w - 10.0 - confirm_sz.width;
                self.select_confirm_btn.set_rect(x, bar_y + (select_bar_h - confirm_sz.height) / 2.0, confirm_sz.width, confirm_sz.height);
                x -= 10.0 + cancel_sz.width;
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

        let base_low = cce_ui::colors::page_low_color();
        let border_col = cce_ui::colors::color_borders_color();

        // 1. General window background
        let bg_color = [
            (base_low[0] * 1.17).min(1.0),
            (base_low[1] * 1.17).min(1.0),
            (base_low[2] * 1.17).min(1.0),
            base_low[3],
        ];
        // 2. The dissolved root Backplate's plate (bg at backplate opacity, config border
        // and radius), then the dissolved panels' plates and their children walked in the
        // legacy panel order.
        use cce_ui::scene::layout::Rect;
        let mut pc = cce_ui::scene::paint::PaintCtx::new();
        {
            let mut fill = bg_color;
            if fill[3] > 0.001 {
                fill[3] = cce_ui::color::active_backplate_opacity();
            }
            let radius = cce_ui::colors::backplate_corner_radius();
            let radii = (radius, radius, radius, radius);
            let rect = Rect { x: 0.0, y: 0.0, width: w_f32, height: h_f32 };
            pc.border(rect, radii, fill, [0.22, 0.22, 0.28, 1.0], 1.5);
        }
        {
            let self_ptr = self as *mut Self;
            // Left panel plate + children.
            self.plate_prims(Rect { x: left_panel_x, y: 10.0, width: left_panel_w, height: content_h }, &mut pc);
            unsafe {
                cce_ui::scene::painter::paint_root_into(&self.ui_context, (*self_ptr).search_box.as_ptr_mut(), &mut pc);
                (*self_ptr).list_region.push_prims(true, &mut pc);
                for btn in (*self_ptr).font_buttons.iter_mut() {
                    if btn.rect().0 > -9000.0 {
                        cce_ui::scene::painter::paint_root_into(&self.ui_context, btn.as_ptr_mut(), &mut pc);
                    }
                }
            }
            // Mid panel plate + children (when a family is selected, like the old links).
            self.plate_prims(Rect { x: mid_panel_x, y: 10.0, width: mid_panel_w, height: content_h }, &mut pc);
            if self.selected_family.is_some() {
                unsafe {
                    (*self_ptr).mid_region.push_prims(false, &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, (*self_ptr).btn_open_folder.as_ptr_mut(), &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, (*self_ptr).btn_remove_font.as_ptr_mut(), &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, (*self_ptr).style_dropdown.as_ptr_mut(), &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, (*self_ptr).size_spinbox.as_ptr_mut(), &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, (*self_ptr).preview_box.as_ptr_mut(), &mut pc);
                }
            }
            // Bottom bar plate + children (select mode).
            if self.select_mode {
                let bar_y = h_f32 - select_bar_h - 10.0;
                self.plate_prims(Rect { x: left_panel_x, y: bar_y, width: w_f32 - 20.0, height: select_bar_h }, &mut pc);
                unsafe {
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, (*self_ptr).select_cancel_btn.as_ptr_mut(), &mut pc);
                    cce_ui::scene::painter::paint_root_into(&self.ui_context, (*self_ptr).select_confirm_btn.as_ptr_mut(), &mut pc);
                }
            }
        }

        let quad = |x: f32, y: f32, w: f32, h: f32, c: [f32; 4], pc: &mut cce_ui::scene::paint::PaintCtx| {
            pc.quad(Rect { x, y, width: w, height: h }, c);
        };

        // 3. Page Content Outline Borders
        // Left Panel Borders
        quad(left_panel_x, 10.0, left_panel_w, 1.0, border_col, &mut pc);
        quad(left_panel_x, 10.0 + content_h, left_panel_w, 1.0, border_col, &mut pc);
        quad(left_panel_x, 10.0, 1.0, content_h, border_col, &mut pc);
        quad(left_panel_x + left_panel_w, 10.0, 1.0, content_h, border_col, &mut pc);

        // Middle Panel Borders
        quad(mid_panel_x, 10.0, mid_panel_w, 1.0, border_col, &mut pc);
        quad(mid_panel_x, 10.0 + content_h, mid_panel_w, 1.0, border_col, &mut pc);
        quad(mid_panel_x, 10.0, 1.0, content_h, border_col, &mut pc);
        quad(mid_panel_x + mid_panel_w, 10.0, 1.0, content_h, border_col, &mut pc);

        // The ScrollBox now automatically draws its own borders and scrollbar.

        // 4. Alphabet preview box + its text prims (family + style/weight attrs).
        if self.selected_family.is_some() {
            let mid_panel_h = self.content_h();
            let alphabet_virtual_y = if self.select_mode { 320.0 } else { 380.0 };
            let alphabet_box_h = if self.select_mode { 102.0 } else { 120.0 };
            let scroll_y = self.mid_region.scroll_y;
            let alphabet_draw_y = 10.0 + alphabet_virtual_y - scroll_y;

            let viewport_top = 10.0;
            let viewport_bottom = 10.0 + mid_panel_h;

            if alphabet_draw_y + alphabet_box_h >= viewport_top && alphabet_draw_y <= viewport_bottom {
                let draw_y_start = alphabet_draw_y.max(viewport_top);
                let draw_y_end = (alphabet_draw_y + alphabet_box_h).min(viewport_bottom);
                let draw_h = draw_y_end - draw_y_start;

                if draw_h > 0.0 {
                    let preview_box_x = mid_panel_x + 10.0;
                    let preview_box_w = mid_panel_w - 20.0;
                    let alphabet_bg = [
                        base_low[0] * 0.9,
                        base_low[1] * 0.9,
                        base_low[2] * 0.9,
                        1.0,
                    ];
                    let alphabet_border = [
                        border_col[0] * 0.85,
                        border_col[1] * 0.85,
                        border_col[2] * 0.85,
                        1.0,
                    ];

                    quad(preview_box_x, draw_y_start, preview_box_w, draw_h, alphabet_bg, &mut pc); // alphabet box bg

                    if alphabet_draw_y >= viewport_top {
                        quad(preview_box_x, alphabet_draw_y, preview_box_w, 1.0, alphabet_border, &mut pc); // Top border
                    }
                    if alphabet_draw_y + alphabet_box_h <= viewport_bottom {
                        quad(preview_box_x, alphabet_draw_y + alphabet_box_h, preview_box_w, 1.0, alphabet_border, &mut pc); // Bottom border
                    }
                    quad(preview_box_x, draw_y_start, 1.0, draw_h, alphabet_border, &mut pc); // Left border
                    quad(preview_box_x + preview_box_w, draw_y_start, 1.0, draw_h, alphabet_border, &mut pc); // Right border
                }
            }

            self.push_alphabet_preview(&mut pc);
        }

        if self.select_mode {
            let bar_y = h_f32 - select_bar_h - 10.0;
            // Draw bottom bar separator, side borders, and bottom border
            quad(left_panel_x, bar_y, w_f32 - 20.0, 1.0, border_col, &mut pc);
            quad(left_panel_x, bar_y, 1.0, select_bar_h, border_col, &mut pc);
            quad(w_f32 - 10.0, bar_y, 1.0, select_bar_h, border_col, &mut pc);
            quad(left_panel_x, bar_y + select_bar_h, w_f32 - 20.0, 1.0, border_col, &mut pc);

            // Selected-font readout in the bottom bar
            pc.text_with(
                "Selected Font:",
                left_panel_x + 10.0,
                bar_y + 18.0,
                12.0,
                [0x5c, 0x90, 0x60],
                Some("monospace".to_string()),
                None,
            );
            let font_name = self.selected_family.clone().unwrap_or_else(|| "None".to_string());
            pc.text_with(
                font_name,
                left_panel_x + 110.0,
                bar_y + 18.0,
                12.0,
                [0xdd, 0xdd, 0xe2],
                Some("monospace".to_string()),
                None,
            );
        }

        // 5. Style-dropdown popover — geometry and labels last, on top of everything. Its
        // labels carry bounds equal to the popover rect, which both clips them to the plate
        // and exempts them from the occlusion clamp (the is-overlay-text convention).
        if self.selected_family.is_some() && self.style_dropdown.open {
            let mut coll = cce_ui::layout::PopoverCollector::new();
            self.style_dropdown.render_popover(&mut coll);
            for &(c, x, y, w, h) in &coll.rects {
                quad(x, y, w, h, c, &mut pc);
            }
            let pop_bounds = self
                .style_dropdown
                .popover_rect()
                .map(|(x, y, w, h)| [x, y, x + w, y + h]);
            for (content, size, tx, ty, color, _font, _bounds) in coll.texts {
                let color_u8 = [
                    (color[0] * 255.0).clamp(0.0, 255.0) as u8,
                    (color[1] * 255.0).clamp(0.0, 255.0) as u8,
                    (color[2] * 255.0).clamp(0.0, 255.0) as u8,
                ];
                pc.text_with(content, tx, ty, size, color_u8, Some("monospace".to_string()), pop_bounds);
            }
        }

        Some(pc.finish())
    }

    fn handle_pointer_move(&mut self, pos: LogicalPosition, needs_rebuild: &mut bool) {
        let mut changed = false;
        let px = pos.x as f32;
        let py = pos.y as f32;

        let old_size = self.preview_box.font_size;

        // The dissolved scroll regions: scrollbar-thumb drags and hover tracking.
        if self.list_region.drag_move(py) {
            changed = true;
        }
        if self.mid_region.drag_move(py) {
            changed = true;
        }
        self.list_region.hovered = self.list_region.hit(px, py);
        self.mid_region.hovered = self.mid_region.hit(px, py);

        // The dissolved panels' cursor forwarding: dragging children get drag_update,
        // everyone else cursor_moved (Plate::on_cursor_moved's inner loop, flattened).
        for w_ptr in self.dispatch_widgets(true) {
            unsafe {
                let w = &mut *w_ptr;
                if w.is_dragging() {
                    if w.drag_update(px, py) {
                        changed = true;
                    }
                } else if w.cursor_moved(px, py, &mut self.ui_context) {
                    changed = true;
                }
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

        // The dissolved panels' press routing, flattened: popover-first (an open dropdown
        // must see the click before anything beneath), then reverse order with the
        // Plates' unfocus-on-missed-press rule, stopping at the first handler.
        let widgets = self.dispatch_widgets(false);
        let mut input_handled = region_handled;
        for &w_ptr in &widgets {
            if input_handled {
                break;
            }
            unsafe {
                let w = &mut *w_ptr;
                if w.popover_rect().is_some() {
                    if w.mouse_input(button, state, px, py, &mut self.ui_context) {
                        changed = true;
                        input_handled = true;
                        break;
                    }
                }
            }
        }
        if !input_handled {
            for &w_ptr in &widgets {
                unsafe {
                    let w = &mut *w_ptr;
                    if w.mouse_input(button, state, px, py, &mut self.ui_context) {
                        changed = true;
                        break;
                    }
                    if state == ElementState::Pressed && !w.hit_test(px, py, &self.ui_context) {
                        w.unfocus();
                    }
                }
            }
        }

        if self.selected_family.is_some() && self.style_dropdown.take_change() {
            msg_out = Some(AppMessage::SelectStyle(self.style_dropdown.selected));
        }

        for (idx, btn) in self.font_buttons.iter_mut().enumerate() {
            if btn.rect().0 > -9000.0 && btn.take_click() {
                if self.select_mode && self.last_click_idx == Some(idx) && self.click_timer > 0.0 {
                    let selected = self.filtered[idx].clone();
                    print!("{} {}", selected, self.size_spinbox.value);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    std::process::exit(0);
                }
                self.last_click_idx = Some(idx);
                self.click_timer = 0.35;
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
        for w_ptr in self.dispatch_widgets(true) {
            unsafe {
                if (*w_ptr).mouse_wheel(delta, px, py, &mut self.ui_context) {
                    changed = true;
                }
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

        // Custom keyboard shortcuts
        if event.ctrl && event.state == ElementState::Pressed {
            if let Key::Character(ref ch) = event.logical_key {
                match ch.to_lowercase().as_str() {
                    "r" => {
                        msg_out = Some(AppMessage::RefreshFonts);
                        handled = true;
                    }
                    "f" => {
                        self.search_box.focus();
                        self.ui_context.set_focused(&mut self.search_box);
                        self.preview_box.unfocus();
                        handled = true;
                    }
                    "e" => {
                        self.preview_box.focus();
                        self.ui_context.set_focused(&mut self.preview_box);
                        self.search_box.unfocus();
                        handled = true;
                    }
                    "o" => {
                        msg_out = Some(AppMessage::OpenFolder);
                        handled = true;
                    }
                    _ => {}
                }
            }
        }

        if !handled && event.state == ElementState::Pressed {
            match event.logical_key {
                Key::Named(NamedKey::Delete) => {
                    msg_out = Some(AppMessage::RemoveFont);
                    handled = true;
                }
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

        if !handled {
            // Arrow navigation
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
            
            for w_ptr in self.dispatch_widgets(true) {
                unsafe {
                    if (*w_ptr).keyboard_input(event, &mut self.ui_context) {
                        handled = true;
                        break;
                    }
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
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _guard = rt.enter();
    
    cce_ui::engine::run::<TypefaceApp>();
}
