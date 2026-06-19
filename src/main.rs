mod pages;

use wayland_client::QueueHandle;
use glyphon::{FontSystem, Buffer, Metrics, Attrs};
use cce_ui::engine::{Application, EngineState, LogicalPosition, LogicalSize, WindowSettings};
use cce_ui::widget::{
    MouseButton, ElementState, MouseScrollDelta, KeyEvent, TextItem, Element,
    TextBox, Button, TextLabel, Key, NamedKey, ScrollingList, Dropdown, Slider,
    Paginator, Container, Plate
};
use cce_ui::widget::focus::link_parent_child;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Browse,
    Keybindings,
}

impl Page {
    pub const ALL: [Page; 2] = [
        Page::Browse,
        Page::Keybindings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Browse => "Browse",
            Page::Keybindings => "Keys",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Page::Browse => "🔤",
            Page::Keybindings => "⌨",
        }
    }
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
    SwitchPage(Page),
    SelectFamily(usize),
    SelectStyle(usize),
    FontSizeChanged,
    SearchChanged,
    OpenFolder,
    RemoveFont,
    RefreshFonts,
}

struct TypefaceApp {
    // Sidebar
    paginator: Paginator,

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
    current_page: Page,
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
    page_root_container: Container,
    left_panel: Plate,
    mid_panel: Plate,
    right_panel: Plate,
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
        self.page_root_container.clear_children(ctx);
        self.left_panel.clear_children(ctx);
        self.mid_panel.clear_children(ctx);
        self.right_panel.clear_children(ctx);
        self.bottom_bar.clear_children(ctx);

        // 1. Link active page-level containers to root
        link_parent_child(&mut self.page_root_container, &mut self.paginator, ctx);
        link_parent_child(&mut self.page_root_container, &mut self.left_panel, ctx);
        link_parent_child(&mut self.page_root_container, &mut self.mid_panel, ctx);
        link_parent_child(&mut self.page_root_container, &mut self.right_panel, ctx);
        
        if self.select_mode {
            link_parent_child(&mut self.page_root_container, &mut self.bottom_bar, ctx);
        }

        if self.current_page == Page::Browse {
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
                link_parent_child(&mut self.mid_panel, &mut self.style_dropdown, ctx);
                link_parent_child(&mut self.mid_panel, &mut self.size_slider, ctx);
                link_parent_child(&mut self.mid_panel, &mut self.preview_box, ctx);
            }

            // Right Panel (Details)
            if self.selected_family.is_some() {
                link_parent_child(&mut self.right_panel, &mut self.btn_open_folder, ctx);
                link_parent_child(&mut self.right_panel, &mut self.btn_remove_font, ctx);
            }

            // Bottom bar
            if self.select_mode {
                link_parent_child(&mut self.bottom_bar, &mut self.select_cancel_btn, ctx);
                link_parent_child(&mut self.bottom_bar, &mut self.select_confirm_btn, ctx);
            }
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
        self.paginator.prepare_text(font_system);
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

        // 1. Sidebar Page Buttons
        labels.extend(self.paginator.text_labels_with_bounds(&self.ui_context));

        // 2. Page Content
        if self.current_page == Page::Browse {
            labels.extend(self.left_panel.text_labels_with_bounds(&self.ui_context));
            labels.extend(self.mid_panel.text_labels_with_bounds(&self.ui_context));
            labels.extend(self.right_panel.text_labels_with_bounds(&self.ui_context));

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
                    labels.push((TextLabel {
                        text: content,
                        x: tx,
                        y: ty,
                        font_size: size,
                        color: color_u8,
                    }, None));
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
                
                let mid_panel_w = self.right_panel.base.base.x - 12.0 - self.mid_panel.base.base.x;
                let preview_box_x = self.mid_panel.base.base.x + 10.0;
                let preview_box_w = mid_panel_w - 20.0;
                let alphabet_box_y = if self.select_mode { 320.0 } else { 380.0 };
                let alphabet_box_h = if self.select_mode { 102.0 } else { 120.0 };

                self.text_items.push(TextItem {
                    buffer: preview_text_buffer_clamped(alph_buf, font_system, mid_panel_w - 40.0),
                    x: self.mid_panel.base.base.x + 20.0,
                    y: alphabet_box_y + 12.0,
                    color: glyphon::Color::rgb(0x88, 0x88, 0x99),
                    bounds: Some([preview_box_x, alphabet_box_y, preview_box_x + preview_box_w, alphabet_box_y + alphabet_box_h]),
                });
            }
        } else if self.current_page == Page::Keybindings {
            labels.push((TextLabel {
                text: "Keybindings".to_string(),
                x: 220.0,
                y: 30.0,
                font_size: 18.0,
                color: [0x5c, 0x90, 0x60],
            }, None));

            let sections: [(&str, &[(&str, &str)]); 3] = [
                ("🧭 Navigation", &[
                    ("Ctrl + F", "Search fonts"),
                    ("Up / Down", "Navigate font list"),
                    ("Enter", "Select font"),
                    ("Ctrl + 1-2", "Switch page"),
                ]),
                ("👁 Preview", &[
                    ("+ / -", "Increase / decrease font size"),
                    ("Ctrl + E", "Edit preview text"),
                ]),
                ("⚡ Actions", &[
                    ("Ctrl + O", "Open font folder"),
                    ("Delete", "Remove user font"),
                    ("Ctrl + R / F5", "Refresh font list"),
                ]),
            ];

            let mut y_offset = 70.0;
            for (sec_title, bindings) in &sections {
                labels.push((TextLabel {
                    text: sec_title.to_string(),
                    x: 220.0,
                    y: y_offset,
                    font_size: 14.0,
                    color: [0xdd, 0xdd, 0xe2],
                }, None));
                y_offset += 24.0;

                for (keys, action) in *bindings {
                    labels.push((TextLabel {
                        text: keys.to_string(),
                        x: 230.0,
                        y: y_offset,
                        font_size: 12.0,
                        color: [0x5c, 0x90, 0x60],
                    }, None));
                    labels.push((TextLabel {
                        text: action.to_string(),
                        x: 370.0,
                        y: y_offset,
                        font_size: 12.0,
                        color: [0x88, 0x88, 0x99],
                    }, None));
                    y_offset += 20.0;
                }
                y_offset += 16.0;
            }
        }

        if self.current_page == Page::Browse && self.select_mode {
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

    fn new(_qh: &QueueHandle<EngineState<Self>>, _sender: calloop::channel::Sender<Self::Message>) -> Self {
        cce_ui::scale::set_scale_factor(1.0);
        // Parse command line arguments
        let args: Vec<String> = std::env::args().collect();
        let select_mode = args.iter().any(|arg| arg == "--select");

        let mut paginator = Paginator::new(56.0, vec![
            "Browse".to_string(),
            "Keys".to_string(),
        ]);
        paginator.tabs_rotated = true;
        paginator.tabs_at_top = false;

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
            paginator,
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
            current_page: Page::Browse,
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
            page_root_container: Container::new(),
            left_panel: Plate::new(0.0, 0.0, 0.0, 0.0).with_blur(false).with_draggable(false),
            mid_panel: Plate::new(0.0, 0.0, 0.0, 0.0).with_blur(false).with_draggable(false),
            right_panel: Plate::new(0.0, 0.0, 0.0, 0.0).with_blur(false).with_draggable(false),
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
            AppMessage::SwitchPage(page) => {
                self.current_page = page;
                let page_idx = match page {
                    Page::Browse => 0,
                    Page::Keybindings => 1,
                };
                self.paginator.set_selected_page(page_idx);
                *needs_rebuild = true;
                self.needs_rebuild = true;
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

    fn tick(&mut self, dt: f32, needs_rebuild: &mut bool) {
        if self.click_timer > 0.0 {
            self.click_timer -= dt;
        }
        if self.paginator.tick(dt, &mut self.ui_context) {
            *needs_rebuild = true;
            self.needs_rebuild = true;
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

        let sidebar_w = self.paginator.sidebar_w();
        let left_panel_x = sidebar_w + 10.0;
        let left_panel_w = 270.0;
        let right_panel_w = 286.0;
        let right_panel_x = w_f32 - 10.0 - right_panel_w;
        let mid_panel_x = left_panel_x + left_panel_w + 12.0;
        let mid_panel_w = right_panel_x - 12.0 - mid_panel_x;

        let select_bar_h = 48.0;
        let content_h = if self.select_mode {
            (h_f32 - 30.0 - select_bar_h).max(100.0)
        } else {
            (h_f32 - 20.0).max(100.0)
        };

        if self.needs_rebuild || size_changed {
            cce_ui::scale::set_scale_factor(scale as f32);
            self.paginator.set_rect(0.0, 0.0, sidebar_w, h_f32);

            // Position panel Plates
            self.left_panel.set_rect(left_panel_x, 10.0, left_panel_w, content_h);
            self.mid_panel.set_rect(mid_panel_x, 10.0, mid_panel_w, content_h);
            self.right_panel.set_rect(right_panel_x, 10.0, right_panel_w, content_h);

            let bar_y = h_f32 - select_bar_h - 10.0;
            self.bottom_bar.set_rect(left_panel_x, bar_y, w_f32 - 76.0, select_bar_h);

            // Configure Plate visibility
            self.left_panel.visible = self.current_page == Page::Browse;
            self.mid_panel.visible = self.current_page == Page::Browse;
            self.right_panel.visible = self.current_page == Page::Browse;
            self.bottom_bar.visible = self.current_page == Page::Browse && self.select_mode;

            if self.current_page == Page::Browse {
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

                // Preview panel widgets
                let style_dropdown_h = cce_ui::layout::dropdown_height() + cce_ui::widget::label_offset(&self.style_dropdown);
                self.style_dropdown.set_rect(mid_panel_x + 10.0, 65.0, mid_panel_w - 20.0, style_dropdown_h);

                let size_slider_h = cce_ui::layout::slider_height() + cce_ui::widget::label_offset(&self.size_slider);
                self.size_slider.set_rect(mid_panel_x + 10.0, 120.0, mid_panel_w - 20.0, size_slider_h);
                let preview_box_h = if self.select_mode { 120.0 } else { 180.0 };
                self.preview_box.set_rect(mid_panel_x + 10.0, 190.0, mid_panel_w - 20.0, preview_box_h);

                // Details panel buttons
                self.btn_open_folder.set_rect(right_panel_x + 10.0, 200.0, 110.0, 28.0);
                self.btn_remove_font.set_rect(right_panel_x + 130.0, 200.0, 110.0, 28.0);

                if self.select_mode {
                    self.select_cancel_btn.set_rect(w_f32 - 190.0, bar_y + 10.0, 80.0, 28.0);
                    self.select_confirm_btn.set_rect(w_f32 - 100.0, bar_y + 10.0, 80.0, 28.0);
                }
            }

            self.rebuild_hierarchy();
            self.rebuild_text_items();
            self.needs_rebuild = false;
        }

        let base_low = cce_ui::colors::page_low_color();
        let border_col = cce_ui::colors::color_borders_color();
        let sidebar_bg = cce_ui::colors::sidebar_bg_color();

        // 1. General window background
        let bg_color = [
            (base_low[0] * 1.17).min(1.0),
            (base_low[1] * 1.17).min(1.0),
            (base_low[2] * 1.17).min(1.0),
            1.0,
        ];
        quads.push((0.0, 0.0, w_f32, h_f32, bg_color));

        // 2. Sidebar background panel
        quads.push((0.0, 0.0, sidebar_w, h_f32, sidebar_bg));
        quads.push((sidebar_w, 0.0, 1.0, h_f32, border_col)); // sidebar separator

        // Draw active containers and all child widgets (including paginator and panel plates)
        for &child_ptr in &self.page_root_container.children {
            unsafe {
                if let Some(child) = child_ptr.as_ref() {
                    if child.visible() {
                        quads.extend(child.extra_quads());
                    }
                }
            }
        }

        // 5. Page Content Outline Borders
        if self.current_page == Page::Browse {
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

            if self.selected_family.is_some() {
                // Alphabet preview box background & borders
                let preview_box_x = mid_panel_x + 10.0;
                let preview_box_w = mid_panel_w - 20.0;
                let alphabet_box_y = if self.select_mode { 320.0 } else { 380.0 };
                let alphabet_box_h = if self.select_mode { 102.0 } else { 120.0 };

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

                quads.push((preview_box_x, alphabet_box_y, preview_box_w, alphabet_box_h, alphabet_bg)); // alphabet box bg
                quads.push((preview_box_x, alphabet_box_y, preview_box_w, 1.0, alphabet_border));
                quads.push((preview_box_x, alphabet_box_y + alphabet_box_h, preview_box_w, 1.0, alphabet_border));
                quads.push((preview_box_x, alphabet_box_y, 1.0, alphabet_box_h, alphabet_border));
                quads.push((preview_box_x + preview_box_w, alphabet_box_y, 1.0, alphabet_box_h, alphabet_border));
            }

            // Right Panel Borders
            quads.push((right_panel_x, 10.0, right_panel_w, 1.0, border_col));
            quads.push((right_panel_x, 10.0 + content_h, right_panel_w, 1.0, border_col));
            quads.push((right_panel_x, 10.0, 1.0, content_h, border_col));
            quads.push((right_panel_x + right_panel_w, 10.0, 1.0, content_h, border_col));

            if self.select_mode {
                let bar_y = h_f32 - select_bar_h - 10.0;
                // Draw bottom bar separator and side borders
                quads.push((left_panel_x, bar_y, w_f32 - 76.0, 1.0, border_col));
                quads.push((left_panel_x, bar_y, 1.0, 48.0, border_col));
                quads.push((w_f32 - 10.0, bar_y, 1.0, 48.0, border_col));
            }
        } else if self.current_page == Page::Keybindings {
            // Keybindings page borders
            quads.push((210.0, 10.0, w_f32 - 220.0, 1.0, border_col));
            quads.push((210.0, h_f32 - 10.0, w_f32 - 220.0, 1.0, border_col));
            quads.push((210.0, 10.0, 1.0, h_f32 - 20.0, border_col));
            quads.push((w_f32 - 10.0, 10.0, 1.0, h_f32 - 20.0, border_col));
        }
    }

    fn overlay_quads(&mut self, quads: &mut Vec<(f32, f32, f32, f32, [f32; 4])>, _size: LogicalSize, _scale: f64) {
        if self.current_page == Page::Browse && self.selected_family.is_some() {
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

        for &child_ptr in &self.page_root_container.children {
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

        // Route input to page_root_container children in reverse order
        for &child_ptr in self.page_root_container.children.iter().rev() {
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

        // Process side effects / clicks
        if self.paginator.take_click() {
            let new_page = self.paginator.selected_page();
            let target_page = match new_page {
                0 => Page::Browse,
                1 => Page::Keybindings,
                _ => Page::Browse,
            };
            msg_out = Some(AppMessage::SwitchPage(target_page));
        }

        if self.current_page == Page::Browse {
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

        for &child_ptr in &self.page_root_container.children {
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
                    "1" => {
                        msg_out = Some(AppMessage::SwitchPage(Page::Browse));
                        handled = true;
                    }
                    "2" => {
                        msg_out = Some(AppMessage::SwitchPage(Page::Keybindings));
                        handled = true;
                    }
                    "r" => {
                        msg_out = Some(AppMessage::RefreshFonts);
                        handled = true;
                    }
                    "f" => {
                        self.current_page = Page::Browse;
                        self.search_box.focus();
                        self.ui_context.set_focused(&mut self.search_box);
                        self.preview_box.unfocus();
                        handled = true;
                    }
                    "e" => {
                        self.current_page = Page::Browse;
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

        if !handled && self.current_page == Page::Browse {
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
        if !handled && self.current_page == Page::Browse {
            let old_search_text = if self.search_box.editing { self.search_box.edit_buffer.clone() } else { self.search_box.text.clone() };
            
            for &child_ptr in &self.page_root_container.children {
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
