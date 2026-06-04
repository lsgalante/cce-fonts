use iced::widget::{column, combo_box, container, row, slider, text, text_input};
use iced::{Color, Element, Length, Task};

use crate::Message;
use crate::pages::browse::{FontEntry, entries_for_family};

// ── Data ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct PreviewState {
    pub family: String,
    pub style: String,
    pub file: String,
    pub preview_text: String,
    pub font_size: f32,
    pub font: iced::Font,
    /// All FontEntry items for the currently selected family.
    pub family_entries: Vec<FontEntry>,
    /// Combo box state for the style dropdown.
    pub style_combo: combo_box::State<String>,
}

impl PreviewState {
    pub fn new() -> Self {
        Self {
            family: String::new(),
            style: String::new(),
            file: String::new(),
            preview_text: String::from("The quick brown fox jumps over the lazy dog"),
            font_size: 32.0,
            font: iced::Font::DEFAULT,
            family_entries: Vec::new(),
            style_combo: combo_box::State::new(Vec::new()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum PreviewMessage {
    SetFamily { family: String },
    StyleSelected(String),
    PreviewTextChanged(String),
    FontSizeChanged(f32),
}

// ── View ────────────────────────────────────────────────────────────

pub fn view(state: &PreviewState) -> Element<'_, Message> {
    let accent = Color::from_rgb8(0x5c, 0x90, 0x60);
    let text_fg = Color::from_rgb8(0xd4, 0xd4, 0xd4);
    let text_dim = Color::from_rgb8(0x88, 0x88, 0x99);

    let font_label = if state.family.is_empty() {
        text("No font selected").size(14).color(text_dim)
    } else {
        text(&state.family).size(14).color(accent)
    };

    let style_label = text("Style:").size(12).color(text_dim);

    let selected_style = if state.style.is_empty() {
        None
    } else {
        Some(state.style.clone())
    };

    let style_dropdown = combo_box(
        &state.style_combo,
        "Select style...",
        selected_style.as_ref(),
        |style| Message::Preview(PreviewMessage::StyleSelected(style)),
    )
    .padding([6, 10]);

    let style_row = row![style_label, style_dropdown]
        .spacing(8)
        .align_y(iced::Alignment::Center);

    let size_label = text(format!("Size: {:.0}pt", state.font_size))
        .size(12)
        .color(text_dim);

    let size_slider = slider(8.0..=120.0, state.font_size, |v| {
        Message::Preview(PreviewMessage::FontSizeChanged(v))
    })
    .step(1.0);

    let text_input = text_input("Preview text...", &state.preview_text)
        .on_input(|s| Message::Preview(PreviewMessage::PreviewTextChanged(s)))
        .padding([8, 12]);

    let preview_font = state.font;

    let preview_text_widget = if state.family.is_empty() {
        text("Select a font from Browse to preview it here")
            .size(20)
            .color(text_dim)
    } else {
        text(state.preview_text.clone())
            .font(preview_font)
            .size(state.font_size)
            .color(text_fg)
    };

    let preview_area = container(preview_text_widget)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(20)
        .style(|_theme| container::Style {
            background: Some(Color::from_rgb8(0x14, 0x22, 0x16).into()),
            border: iced::Border {
                radius: 8.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        });

    let alpha_text = text("ABCDEFGHIJKLMNOPQRSTUVWXYZ\nabcdefghijklmnopqrstuvwxyz\n0123456789\n!@#$%^&*()_+-=[]{}|;':\",./<>?")
        .font(state.font)
        .size(state.font_size * 0.6)
        .color(text_dim);

    let alpha_area = container(alpha_text)
        .width(Length::Fill)
        .padding(16)
        .style(|_theme| container::Style {
            background: Some(Color::from_rgb8(0x14, 0x22, 0x16).into()),
            border: iced::Border {
                radius: 8.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        });

    column![
        font_label,
        style_row,
        row![size_label, size_slider,].spacing(8).align_y(iced::Alignment::Center),
        text_input,
        preview_area,
        alpha_area,
    ]
    .spacing(10)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ── Update ──────────────────────────────────────────────────────────

pub fn update(state: &mut PreviewState, msg: PreviewMessage, all_fonts: &[FontEntry]) -> Task<Message> {
    match msg {
        PreviewMessage::SetFamily { family } => {
            state.family = family;
            state.family_entries = entries_for_family(all_fonts, &state.family)
                .into_iter()
                .cloned()
                .collect();

            let styles: Vec<String> = state.family_entries
                .iter()
                .map(|e| e.style.clone())
                .collect();

            // Pick the first style (or "Regular" if available)
            let default_style = styles
                .iter()
                .find(|s| *s == "Regular")
                .or(styles.first())
                .cloned()
                .unwrap_or_default();

            state.style_combo = combo_box::State::new(styles);
            state.style = default_style.clone();

            // Find the file for the default style
            state.file = state.family_entries
                .iter()
                .find(|e| e.style == default_style)
                .map(|e| e.file.clone())
                .unwrap_or_default();

            // Update the iced Font
            if state.family.is_empty() {
                state.font = iced::Font::DEFAULT;
            } else {
                let static_str: &'static str = state.family.clone().leak();
                state.font = iced::Font::with_name(static_str);
            }
            Task::none()
        }
        PreviewMessage::StyleSelected(style) => {
            state.style = style.clone();
            state.file = state.family_entries
                .iter()
                .find(|e| e.style == style)
                .map(|e| e.file.clone())
                .unwrap_or_default();
            Task::none()
        }
        PreviewMessage::PreviewTextChanged(s) => {
            state.preview_text = s;
            Task::none()
        }
        PreviewMessage::FontSizeChanged(v) => {
            state.font_size = v;
            Task::none()
        }
    }
}

// ── Subscription ────────────────────────────────────────────────────

pub fn subscription(_state: &PreviewState) -> iced::Subscription<Message> {
    iced::Subscription::none()
}
