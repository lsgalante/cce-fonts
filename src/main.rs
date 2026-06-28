mod pages;

use wayland_client::QueueHandle;
use glyphon::{FontSystem, Buffer, Metrics, Attrs};
use cce_ui::engine::{Application, EngineState, LogicalPosition, LogicalSize, WindowSettings};
use cce_ui::widget::{
    MouseButton, ElementState, MouseScrollDelta, KeyEvent, TextItem, Element,
    TextBox, Button, TextLabel, Key, NamedKey, ScrollingList, ScrollBox, Dropdown, Slider,
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
    search_box: TextBox,
    font_list: ScrollingList,
    font_buttons: Vec<Button>,

    // Preview panel
    style_dropdown: Dropdown,
    size_slider: Slider,
    preview_box: TextBox,

    // Details panel
    btn_open_folder: Button,
    btn_remove_font: Button,

    // Selection mode buttons
    select_cancel_btn: Button,
    select_confirm_btn: Button,
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
    text_items: Vec<TextItem>,
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

fn make_text_buffer_with_font(
    fs: &mut FontSystem,
    text: &str,
    size: f32,
    font: Option<&str>,
    style: Option<glyphon::Style>,
    weight: Option<glyphon::Weight>,
) -> Buffer {
    let scale = cce_ui::scale::scale_factor();
    let mut font_size = size;
    let mut family_name = None;

    if let Some(font_str) = font {
        let (parsed_family, parsed_size) = cce_ui::layout::parse_font_string(font_str);
        if let Some(ps) = parsed_size {
            font_size = ps;
        }
        family_name = Some(parsed_family);
    }

    let physical_size = font_size * scale;
    let metrics = Metrics::new(physical_size, physical_size * 1.4);
    let mut buf = Buffer::new(fs, metrics);
    let mut attrs = Attrs::new();
    if let Some(ref font_family) = family_name {
        let family = match font_family.as_str() {
            "monospace" => glyphon::Family::Name(cce_ui::layout::get_system_monospace_font()),
            "sans-serif" => glyphon::Family::SansSerif,
            "serif" => glyphon::Family::Serif,
            name => glyphon::Family::Name(name),
        };
        attrs = attrs.family(family);
    }
    if let Some(s) = style {
        attrs = attrs.style(s);
    }
    if let Some(w) = weight {
        attrs = attrs.weight(w);
    }
    buf.set_text(fs, text, attrs, glyphon::Shaping::Advanced);
    buf.shape_until_scroll(fs, true);
    buf
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
            link_parent_child(&mut self.mid_panel, &mut self.size_slider, ctx);
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

    fn rebuild_text_items(&mut self) {
        self.text_items.clear();
        let font_system = &mut self.font_system;

        // Ensure all widgets have their text prepared/shaped
        self.search_box.prepare_text(font_system);
        self.font_list.prepare_text(font_system);
        for btn in &mut self.font_buttons {
            btn.prepare_text(font_system);
        }
        self.style_dropdown.prepare_text(font_system);
        self.size_slider.prepare_text(font_system);
        self.preview_box.prepare_text(font_system);
        self.btn_open_folder.prepare_text(font_system);
        self.btn_remove_font.prepare_text(font_system);
        self.select_cancel_btn.prepare_text(font_system);
        self.select_confirm_btn.prepare_text(font_system);

        let mut labels = Vec::new();

        // Page Content
        labels.extend(self.left_panel.text_labels_with_bounds(&self.ui_context));

        // Clamped middle panel labels
        let (mid_panel_x, _, mid_panel_w, mid_panel_h) = self.mid_panel.rect();
        let mid_viewport = [mid_panel_x, 10.0, mid_panel_x + mid_panel_w, 10.0 + mid_panel_h];
        let mid_labels = self.mid_panel.text_labels_with_bounds(&self.ui_context);
        for (label, bounds) in mid_labels {
            let clamped_bounds = match bounds {
                Some(b) => {
                    let x0 = b[0].max(mid_viewport[0]);
                    let y0 = b[1].max(mid_viewport[1]);
                    let x1 = b[2].min(mid_viewport[2]);
                    let y1 = b[3].min(mid_viewport[3]);
                    if x0 < x1 && y0 < y1 {
                        Some([x0, y0, x1, y1])
                    } else {
                        continue; // Completely clipped
                    }
                }
                None => Some(mid_viewport),
            };
            labels.push((label, clamped_bounds));
        }

        if self.select_mode {
            labels.extend(self.bottom_bar.text_labels_with_bounds(&self.ui_context));
        }

        // Render Dropdown popover labels if open
        if self.style_dropdown.open && self.selected_family.is_some() {
            let mut pc = cce_ui::layout::PopoverCollector::new();
            self.style_dropdown.render_popover(&mut pc);
            for (content, size, tx, ty, color, _font, _bounds) in pc.texts {
                let color_u8 = [
                    (color[0] * 255.0).clamp(0.0, 255.0) as u8,
                    (color[1] * 255.0).clamp(0.0, 255.0) as u8,
                    (color[2] * 255.0).clamp(0.0, 255.0) as u8,
                ];
                let pop_x = tx;
                let pop_y = ty;
                let popover_bounds = Some(mid_viewport);
                labels.push((TextLabel {
                    text: content,
                    x: pop_x,
                    y: pop_y,
                    font_size: size,
                    color: color_u8,
                }, popover_bounds));
            }
        }

        // Alphabet preview
        if let Some(ref family) = self.selected_family {
            let font_size = self.size_slider.get_scaled_value();
            let mut style_val = None;
            let mut weight_val = None;
            if let Some(ref style) = self.selected_style {
                let sl = style.to_lowercase();
                if sl.contains("italic") || sl.contains("oblique") {
                    style_val = Some(glyphon::Style::Italic);
                }
                if sl.contains("bold") {
                    weight_val = Some(glyphon::Weight::BOLD);
                } else if sl.contains("light") {
                    weight_val = Some(glyphon::Weight::LIGHT);
                } else if sl.contains("medium") {
                    weight_val = Some(glyphon::Weight::MEDIUM);
                }
            }

            let alphabet_text = "ABCDEFGHIJKLMNOPQRSTUVWXYZ\nabcdefghijklmnopqrstuvwxyz\n0123456789\n!@#$%^&*()_+-=[]{}|;':\",./<>";
            let alph_buf = make_text_buffer_with_font(
                font_system,
                alphabet_text,
                (font_size * 0.55).clamp(8.0, 36.0),
                Some(family),
                style_val,
                weight_val,
            );
            
            let preview_box_x = mid_panel_x + 10.0;
            let preview_box_w = mid_panel_w - 20.0;
            
            let alphabet_virtual_y = if self.select_mode { 320.0 } else { 380.0 };
            let alphabet_box_h = if self.select_mode { 102.0 } else { 120.0 };
            let scroll_y = self.mid_scroll.scroll_y;
            let alphabet_draw_y = 10.0 + alphabet_virtual_y - scroll_y;

            let viewport_top = 10.0;
            let viewport_bottom = 10.0 + mid_panel_h;
            if alphabet_draw_y + alphabet_box_h >= viewport_top && alphabet_draw_y <= viewport_bottom {
                let bounds_y_start = alphabet_draw_y.max(viewport_top);
                let bounds_y_end = (alphabet_draw_y + alphabet_box_h).min(viewport_bottom);
                self.text_items.push(TextItem {
                    buffer: preview_text_buffer_clamped(alph_buf, font_system, mid_panel_w - 40.0),
                    x: mid_panel_x + 20.0,
                    y: alphabet_draw_y + 12.0,
                    color: glyphon::Color::rgb(0x88, 0x88, 0x99),
                    bounds: Some([preview_box_x, bounds_y_start, preview_box_x + preview_box_w, bounds_y_end]),
                });
            }
        }

        if self.select_mode {
            let left_panel_x = self.left_panel.base.base.x;
            let bar_y = self.height as f32 - 48.0 - 10.0;
            labels.push((TextLabel {
                text: "Selected Font:".to_string(),
                x: left_panel_x + 10.0,
                y: bar_y + 18.0,
                font_size: 12.0,
                color: [0x5c, 0x90, 0x60],
            }, None));

            let font_name = self.selected_family.clone().unwrap_or_else(|| "None".to_string());
            labels.push((TextLabel {
                text: font_name,
                x: left_panel_x + 110.0,
                y: bar_y + 18.0,
                font_size: 12.0,
                color: [0xdd, 0xdd, 0xe2],
            }, None));
        }

        // Convert TextLabels to text_items
        let scale = cce_ui::scale::scale_factor();
        for (label, bounds) in labels {
            let physical_size = label.font_size * scale;
            let metrics = Metrics::new(physical_size, physical_size * 1.4);
            let mut buf = Buffer::new(font_system, metrics);
            buf.set_text(font_system, &label.text, Attrs::new(), glyphon::Shaping::Advanced);
            buf.shape_until_scroll(font_system, true);
            self.text_items.push(TextItem {
                buffer: buf,
                x: label.x,
                y: label.y,
                color: glyphon::Color::rgb(label.color[0], label.color[1], label.color[2]),
                bounds,
            });
        }
    }
}

// Helpers to prevent preview text overflowing the panel width by wrapping or cropping
fn preview_text_buffer_clamped(mut buf: Buffer, font_system: &mut FontSystem, max_width: f32) -> Buffer {
    buf.set_size(font_system, Some(max_width), None);
    buf.shape_until_scroll(font_system, true);
    buf
}

impl Application for TypefaceApp {
    type Message = AppMessage;

    fn ui_context(&self) -> Option<&cce_ui::context::UiContext> {
        Some(&self.ui_context)
    }

    fn new(_qh: &QueueHandle<EngineState<Self>>, _sender: calloop::channel::Sender<Self::Message>) -> Self {
        cce_ui::scale::set_scale_factor(1.0);
        // Parse command line arguments
        let args: Vec<String> = std::env::args().collect();
        let select_mode = args.iter().any(|arg| arg == "--select");

        let mut search_box = TextBox::new(String::new()).with_multiline(false).with_draw_bg_border(true);
        search_box.font_size = 12.0;

        let font_list = ScrollingList::new(24.0, 4.0);

        let style_dropdown = Dropdown::new(Vec::new(), 0).with_label("Style:");

        let mut size_slider = Slider::new().with_range(8.0, 120.0).with_readout(true).with_label("Size:");
        size_slider.set_scaled_value(32.0);

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
            size_slider,
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
            text_items: Vec::new(),
            font_system: {
                let mut fs = FontSystem::new();
                fs.db_mut().load_fonts_dir("/home/lsgalante/Dropbox/Fonts");
                fs
            },
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
            bottom_bar: Plate::new(0.0, 0.0, 0.0, 0.0).with_blur(false).with_draggable(false),
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
            app.size_slider.set_scaled_value(sz);
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
                self.preview_box.font_size = self.size_slider.get_scaled_value();
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

        fn view(&mut self, quads: &mut Vec<(f32, f32, f32, f32, [f32; 4])>, size: LogicalSize, scale: f64) {
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
            self.root_window.background_color = Some(cce_ui::colors::page_low_color());

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
            let list_y = 60.0;
            let list_w = left_panel_w - 20.0;
            let list_h = (content_h - 50.0).max(100.0);
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
            let size_slider_h = cce_ui::layout::slider_height() + cce_ui::widget::label_offset(&self.size_slider);
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

            // Size slider (virtual_y = 120.0, h = size_slider_h)
            let slider_draw_y = 10.0 + 120.0 - scroll_y;
            if slider_draw_y + size_slider_h >= viewport_top && slider_draw_y <= viewport_bottom {
                self.size_slider.set_rect(mid_panel_x + 10.0, slider_draw_y, mid_panel_w - 20.0, size_slider_h);
            } else {
                self.size_slider.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            // Preview box (virtual_y = 190.0, h = preview_box_h)
            let preview_draw_y = 10.0 + 190.0 - scroll_y;
            if preview_draw_y + preview_box_h >= viewport_top && preview_draw_y <= viewport_bottom {
                self.preview_box.set_rect(mid_panel_x + 10.0, preview_draw_y, mid_panel_w - 20.0, preview_box_h);
            } else {
                self.preview_box.set_rect(-9999.0, -9999.0, 0.0, 0.0);
            }

            if self.select_mode {
                self.select_cancel_btn.set_rect(w_f32 - 190.0, bar_y + 10.0, 80.0, 28.0);
                self.select_confirm_btn.set_rect(w_f32 - 100.0, bar_y + 10.0, 80.0, 28.0);
            }

            self.rebuild_hierarchy();
            self.rebuild_text_items();
            self.needs_rebuild = false;
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
        quads.push((0.0, 0.0, w_f32, h_f32, bg_color));

        // Draw active containers and all child widgets (including panel plates)
        for &child_ptr in &self.root_window.children {
            unsafe {
                if let Some(child) = child_ptr.as_ref() {
                    if child.visible() {
                        quads.extend(child.extra_quads());
                    }
                }
            }
        }

        // 5. Page Content Outline Borders
        // Left Panel Borders
        quads.push((left_panel_x, 10.0, left_panel_w, 1.0, border_col));
        quads.push((left_panel_x, 10.0 + content_h, left_panel_w, 1.0, border_col));
        quads.push((left_panel_x, 10.0, 1.0, content_h, border_col));
        quads.push((left_panel_x + left_panel_w, 10.0, 1.0, content_h, border_col));

        // Middle Panel Borders
        quads.push((mid_panel_x, 10.0, mid_panel_w, 1.0, border_col));
        quads.push((mid_panel_x, 10.0 + content_h, mid_panel_w, 1.0, border_col));
        quads.push((mid_panel_x, 10.0, 1.0, content_h, border_col));
        quads.push((mid_panel_x + mid_panel_w, 10.0, 1.0, content_h, border_col));

        // The ScrollBox now automatically draws its own borders and scrollbar.

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

                    quads.push((preview_box_x, draw_y_start, preview_box_w, draw_h, alphabet_bg)); // alphabet box bg
                    
                    if alphabet_draw_y >= viewport_top {
                        quads.push((preview_box_x, alphabet_draw_y, preview_box_w, 1.0, alphabet_border)); // Top border
                    }
                    if alphabet_draw_y + alphabet_box_h <= viewport_bottom {
                        quads.push((preview_box_x, alphabet_draw_y + alphabet_box_h, preview_box_w, 1.0, alphabet_border)); // Bottom border
                    }
                    quads.push((preview_box_x, draw_y_start, 1.0, draw_h, alphabet_border)); // Left border
                    quads.push((preview_box_x + preview_box_w, draw_y_start, 1.0, draw_h, alphabet_border)); // Right border
                }
            }
        }

        if self.select_mode {
            let bar_y = h_f32 - select_bar_h - 10.0;
            // Draw bottom bar separator, side borders, and bottom border
            quads.push((left_panel_x, bar_y, w_f32 - 20.0, 1.0, border_col));
            quads.push((left_panel_x, bar_y, 1.0, select_bar_h, border_col));
            quads.push((w_f32 - 10.0, bar_y, 1.0, select_bar_h, border_col));
            quads.push((left_panel_x, bar_y + select_bar_h, w_f32 - 20.0, 1.0, border_col));
        }
    }

    fn overlay_quads(&mut self, quads: &mut Vec<(f32, f32, f32, f32, [f32; 4])>, _size: LogicalSize, _scale: f64) {
        if self.selected_family.is_some() {
            // Style Dropdown popover rendered on top of everything
            let mut pc = cce_ui::layout::PopoverCollector::new();
            self.style_dropdown.render_popover(&mut pc);
            quads.extend(pc.rects.iter().map(|&(c, x, y, w, h)| (x, y, w, h, c)));
        }
    }

    fn text_items(&self) -> &[TextItem] {
        &self.text_items
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

        let new_size = self.size_slider.get_scaled_value();
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
                    print!("{} {:.0}", selected, self.size_slider.get_scaled_value());
                    std::process::exit(0);
                }
                self.last_click_idx = Some(idx);
                self.click_timer = 0.35;
                msg_out = Some(AppMessage::SelectFamily(idx));
                break;
            }
        }

        let old_size = self.preview_box.font_size;
        let new_size = self.size_slider.get_scaled_value();
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
                print!("{} {:.0}", selected, self.size_slider.get_scaled_value());
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
                        let (_, max) = self.size_slider.range();
                        let new_sz = (self.size_slider.get_scaled_value() + 2.0).min(max);
                        self.size_slider.set_scaled_value(new_sz);
                        msg_out = Some(AppMessage::FontSizeChanged);
                        handled = true;
                    }
                }
                Key::Character(ref ch) if ch == "-" => {
                    if self.selected_family.is_some() {
                        let (min, _) = self.size_slider.range();
                        let new_sz = (self.size_slider.get_scaled_value() - 2.0).max(min);
                        self.size_slider.set_scaled_value(new_sz);
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
