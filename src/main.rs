mod pages;

use wayland_client::QueueHandle;
use glyphon::FontSystem;
use cce_ui::engine::{Application, EngineState, LogicalPosition, LogicalSize, WindowSettings};
use cce_ui::widget::{
    MouseButton, ElementState, MouseScrollDelta, KeyEvent, Element,
    TextBox, Button, Key, NamedKey, List, ScrollBox, Dropdown, Spinbox,
    Backplate, Plate
};
use cce_ui::widget::focus::link_parent_child;



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

struct TypefaceApp {
    // Browse panel
    search_box: cce_ui::widget::Adapted<TextBox>,
    font_list: List,
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

    // Containers
    root_window: Backplate,
    left_panel: Plate,
    mid_panel: Plate,
    mid_scroll: ScrollBox,
    bottom_bar: Plate,
    ui_context: cce_ui::context::UiContext,
}

impl TypefaceApp {
    fn rebuild_hierarchy(&mut self) {
        let ctx = &mut self.ui_context;
        self.root_window.clear_children(ctx);
        self.left_panel.clear_children(ctx);
        self.mid_panel.clear_children(ctx);
        self.bottom_bar.clear_children(ctx);
        self.mid_scroll.clear_children(ctx);

        // 1. Link active page-level containers to root
        link_parent_child(&mut self.root_window, &mut self.left_panel, ctx);
        link_parent_child(&mut self.root_window, &mut self.mid_panel, ctx);
        
        if self.select_mode {
            link_parent_child(&mut self.root_window, &mut self.bottom_bar, ctx);
        }

        // Left Panel (Browse list)
        link_parent_child(&mut self.left_panel, &mut self.search_box, ctx);
        link_parent_child(&mut self.left_panel, &mut self.font_list, ctx);
        
        // Scrolling list buttons
        for btn in &mut self.font_buttons {
            if btn.rect().0 > -9000.0 {
                link_parent_child(&mut self.left_panel, btn, ctx);
            }
        }

        // Middle Panel (Preview)
        if self.selected_family.is_some() {
            // Link mid_scroll to mid_panel
            link_parent_child(&mut self.mid_panel, &mut self.mid_scroll, ctx);

            // Link interactive elements to mid_panel (Plate) so it routes events to them
            link_parent_child(&mut self.mid_panel, &mut self.btn_open_folder, ctx);
            link_parent_child(&mut self.mid_panel, &mut self.btn_remove_font, ctx);
            link_parent_child(&mut self.mid_panel, &mut self.style_dropdown, ctx);
            link_parent_child(&mut self.mid_panel, &mut self.size_spinbox, ctx);
            link_parent_child(&mut self.mid_panel, &mut self.preview_box, ctx);
        }

        // Bottom bar
        if self.select_mode {
            link_parent_child(&mut self.bottom_bar, &mut self.select_cancel_btn, ctx);
            link_parent_child(&mut self.bottom_bar, &mut self.select_confirm_btn, ctx);
        }
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
        let item_height_full = 24.0 + 4.0; // item_height + item_gap
        let top = index as f32 * item_height_full;
        let bottom = top + 24.0;
        let viewport_top = self.font_list.scroll_box.scroll_y;
        let viewport_bottom = viewport_top + self.font_list.scroll_box.viewport_h;

        if bottom > viewport_bottom {
            let max_scroll = (self.font_list.scroll_box.content_h - self.font_list.scroll_box.viewport_h).max(0.0);
            self.font_list.scroll_box.scroll_y = (bottom - self.font_list.scroll_box.viewport_h).clamp(0.0, max_scroll);
        } else if top < viewport_top {
            self.font_list.scroll_box.scroll_y = top;
        }
    }

    /// Glyph-advance shaping for the interactive widgets — load-bearing for cursor↔pixel
    /// mapping and label measurement; all rendered text is display-list prims shaped by the
    /// engine (which also carries the system fonts via `load_system_fonts`).
    fn refresh_widget_text(&mut self) {
        let font_system = &mut self.font_system;
        self.search_box.prepare_text(font_system);
        self.font_list.prepare_text(font_system);
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

        let (mid_panel_x, _, mid_panel_w, mid_panel_h) = self.mid_panel.rect();
        let preview_box_x = mid_panel_x + 10.0;
        let preview_box_w = mid_panel_w - 20.0;

        let alphabet_virtual_y = if self.select_mode { 320.0 } else { 380.0 };
        let alphabet_box_h = if self.select_mode { 102.0 } else { 120.0 };
        let scroll_y = self.mid_scroll.scroll_y;
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

    fn new(_qh: &QueueHandle<EngineState<Self>>, _sender: calloop::channel::Sender<Self::Message>) -> Self {
        cce_ui::scale::set_scale_factor(1.0);
        // Parse command line arguments
        let args: Vec<String> = std::env::args().collect();
        let select_mode = args.iter().any(|arg| arg == "--select");

        let mut search_box = TextBox::new(String::new()).with_multiline(false).with_draw_bg_border(true);
        search_box.font_size = 12.0;

        let font_list = List::new(24.0, 4.0);

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
            font_list,
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
            root_window: {
                let win_color = cce_ui::colors::page_low_color();
                let win_radius = cce_ui::colors::backplate_corner_radius();
                Backplate::new(0.0, 0.0, if select_mode { 900.0 } else { 1200.0 }, if select_mode { 500.0 } else { 720.0 })
                    .with_background(win_color)
                    .with_border([0.22, 0.22, 0.28, 1.0], 1.5)
                    .with_radius(win_radius)
            },
            left_panel: Plate::new(0.0, 0.0, 0.0, 0.0).with_blur(false).with_draggable(false),
            mid_panel: Plate::new(0.0, 0.0, 0.0, 0.0).with_blur(false).with_draggable(false),
            mid_scroll: ScrollBox::new(),
            bottom_bar: Plate::new(0.0, 0.0, 0.0, 0.0).with_blur(false).with_draggable(false).with_engine_layout({
                // Right-aligned row of buttons, 10px gap, vertically centered, 10px right inset.
                // The buttons size to their text (Button::intrinsic_size) instead of a fixed 80px.
                let mut s = cce_ui::scene::layout::Style::row()
                    .gap(10.0)
                    .main_align(cce_ui::scene::layout::MainAlign::End)
                    .cross_align(cce_ui::scene::layout::CrossAlign::Center);
                s.padding = cce_ui::scene::layout::Edges { left: 0.0, right: 10.0, top: 0.0, bottom: 0.0 };
                s
            }),
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
            self.root_window.set_rect(0.0, 0.0, w_f32, h_f32);

            // Position panel Plates
            self.left_panel.set_rect(left_panel_x, 10.0, left_panel_w, content_h);
            self.mid_panel.set_rect(mid_panel_x, 10.0, mid_panel_w, content_h);
            self.mid_scroll.set_rect(mid_panel_x, 10.0, mid_panel_w, content_h);

            let mid_content_h = if self.select_mode { 440.0 } else { 520.0 };
            self.mid_scroll.update_bounds(mid_content_h, 10.0, content_h);

            let bar_y = h_f32 - select_bar_h - 10.0;
            self.bottom_bar.set_rect(left_panel_x, bar_y, w_f32 - 20.0, select_bar_h);

            // Configure Plate visibility
            self.left_panel.visible = true;
            self.mid_panel.visible = true;
            self.bottom_bar.visible = self.select_mode;

            // Search box
            self.search_box.set_rect(left_panel_x + 10.0, 10.0, left_panel_w - 20.0, 26.0);

            // Scrolling List
            let list_x = left_panel_x + 10.0;
            let list_y = 46.0;
            let list_w = left_panel_w - 20.0;
            let list_h = (content_h - 36.0).max(100.0);
            self.font_list.set_rect(list_x, list_y, list_w, list_h);
            self.font_list.update_bounds(self.filtered.len(), list_y, list_h);

            // Layout list buttons
            for (idx, btn) in self.font_buttons.iter_mut().enumerate() {
                if let Some(draw_y) = self.font_list.get_item_draw_y(idx, 0.0) {
                    btn.set_rect(list_x + 4.0, draw_y, list_w - 8.0, 24.0);
                } else {
                    btn.set_rect(-9999.0, -9999.0, 0.0, 0.0);
                }
            }

            // Preview panel widgets layout
            let style_dropdown_h = cce_ui::layout::dropdown_height() + cce_ui::widget::label_offset(&self.style_dropdown);
            let size_spinbox_h = cce_ui::layout::spinbox_height() + cce_ui::widget::label_offset(&self.size_spinbox);
            let preview_box_h = if self.select_mode { 120.0 } else { 180.0 };

            let scroll_y = self.mid_scroll.scroll_y;
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
                // Phase 2b: lay out the bottom bar's buttons via the scene layout engine. They
                // size to their text and right-align in the bar, instead of fixed 80px slots.
                let bar_ptr: *mut (dyn cce_ui::widget::Element + 'static) =
                    &mut self.bottom_bar as *mut _;
                cce_ui::scene::bridge::layout_subtree(
                    &self.ui_context,
                    bar_ptr,
                    cce_ui::scene::layout::Rect {
                        x: left_panel_x,
                        y: bar_y,
                        width: w_f32 - 20.0,
                        height: select_bar_h,
                    },
                );
            }

            self.rebuild_hierarchy();
            self.refresh_widget_text();
            self.needs_rebuild = false;
        }

        // Popover registration for the display-list text occlusion clamp. ui_context ONLY —
        // deliberately not the global popovers registry: this app draws its popover in the
        // display list below (not on an engine xdg popup), and a global registration would
        // spawn an empty popup surface (no render_popovers override here).
        self.ui_context.clear_popovers();
        if self.selected_family.is_some() && self.style_dropdown.popover_rect().is_some() {
            self.ui_context.register_popover(&self.style_dropdown);
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
        self.root_window.background_color = Some(bg_color);

        // 2. The widget tree walked into the list.
        use cce_ui::scene::layout::Rect;
        let mut pc = cce_ui::scene::paint::PaintCtx::new();
        let root: *mut (dyn cce_ui::widget::Element + 'static) = self.root_window.as_ptr_mut();
        cce_ui::scene::painter::paint_root_into(&self.ui_context, root, &mut pc);

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
            let mid_panel_h = self.mid_panel.base.base.h;
            let alphabet_virtual_y = if self.select_mode { 320.0 } else { 380.0 };
            let alphabet_box_h = if self.select_mode { 102.0 } else { 120.0 };
            let scroll_y = self.mid_scroll.scroll_y;
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

        for &child_ptr in &self.root_window.children {
            unsafe {
                if let Some(child) = child_ptr.as_mut() {
                    if child.visible() {
                        if child.cursor_moved(px, py, &mut self.ui_context) {
                            changed = true;
                        }
                    }
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

        // Route input to root_window children in reverse order
        for &child_ptr in self.root_window.children.iter().rev() {
            unsafe {
                if let Some(child) = child_ptr.as_mut() {
                    if child.visible() {
                        if child.mouse_input(button, state, px, py, &mut self.ui_context) {
                            changed = true;
                            break;
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

        for &child_ptr in &self.root_window.children {
            unsafe {
                if let Some(child) = child_ptr.as_mut() {
                    if child.visible() {
                        if child.mouse_wheel(delta, px, py, &mut self.ui_context) {
                            changed = true;
                        }
                    }
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
            
            for &child_ptr in &self.root_window.children {
                unsafe {
                    if let Some(child) = child_ptr.as_mut() {
                        if child.visible() {
                            if child.keyboard_input(event, &mut self.ui_context) {
                                handled = true;
                                break;
                            }
                        }
                    }
                }
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
