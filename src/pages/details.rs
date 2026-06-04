use iced::widget::{button, column, row, text};
use iced::{Color, Element, Length, Task};
use std::process::Command;

use crate::Message;

fn detail_row<'a>(
    label: &'a str,
    value: &'a str,
    label_color: Color,
    value_color: Color,
) -> iced::widget::Row<'a, Message, iced::Theme, iced::Renderer> {
    row![
        text(format!("{}:", label))
            .size(12)
            .color(label_color)
            .width(80),
        text(value).size(12).color(value_color),
    ]
    .spacing(4)
}

// ── Data ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct DetailsState {
    pub family: String,
    pub style: String,
    pub file: String,
    pub full_name: String,
    pub charset_count: usize,
    pub charset_str: String,
    pub is_user_font: bool,
}

#[derive(Debug, Clone)]
pub enum DetailsMessage {
    SetFont {
        family: String,
        style: String,
        file: String,
    },
    RemoveFont,
    OpenFolder,
}

// ── Helpers ─────────────────────────────────────────────────────────

fn is_user_font(file: &str) -> bool {
    file.starts_with("/home/") || file.contains(".local/share/fonts") || file.contains(".fonts")
}

fn count_chars(file: &str) -> usize {
    // Try to get character count from fc-query
    let output = Command::new("fc-query")
        .arg("--format=%{charset}")
        .arg(file)
        .output()
        .ok();

    match output {
        Some(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            // charset is a space-separated list of ranges like "0-127 160-255"
            let mut count = 0usize;
            for range in s.split_whitespace() {
                if let Some((start, end)) = range.split_once('-') {
                    if let (Ok(s), Ok(e)) = (start.parse::<u32>(), end.parse::<u32>()) {
                        count += (e - s + 1) as usize;
                    }
                } else if let Ok(_v) = range.parse::<u32>() {
                    count += 1;
                }
            }
            count
        }
        None => 0,
    }
}

// ── View ────────────────────────────────────────────────────────────

pub fn view(state: &DetailsState) -> Element<'_, Message> {
    let accent = Color::from_rgb8(0x5c, 0x90, 0x60);
    let text_fg = Color::from_rgb8(0xd4, 0xd4, 0xd4);
    let text_dim = Color::from_rgb8(0x88, 0x88, 0x99);
    let danger = Color::from_rgb8(0xaa, 0x33, 0x33);
    let safe = Color::from_rgb8(0x33, 0x55, 0x38);

    if state.family.is_empty() {
        return column![text("Select a font from Browse to see details")
            .size(14)
            .color(text_dim),]
            .width(Length::Fill)
            .into();
    }

    let info_rows = column![
        detail_row("Family", &state.family, text_dim, text_fg),
        detail_row("Style", &state.style, text_dim, text_fg),
        detail_row("File", &state.file, text_dim, text_fg),
        detail_row("Characters", &state.charset_str, text_dim, text_fg),
        detail_row("Location", if state.is_user_font { "User" } else { "System" }, text_dim, text_fg),
    ]
    .spacing(6);

    let open_btn = button(text("Open Folder").color(Color::WHITE).size(12))
        .padding([8, 14])
        .style(move |_theme, _status| button::Style {
            background: Some(safe.into()),
            border: iced::Border {
                radius: 6.0.into(),
                ..iced::Border::default()
            },
            ..button::Style::default()
        })
        .on_press(Message::Details(DetailsMessage::OpenFolder));

    let remove_btn = if state.is_user_font {
        button(text("Remove Font").color(Color::WHITE).size(12))
            .padding([8, 14])
            .style(move |_theme, _status| button::Style {
                background: Some(danger.into()),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..iced::Border::default()
                },
                ..button::Style::default()
            })
            .on_press(Message::Details(DetailsMessage::RemoveFont))
    } else {
        button(text("Remove Font").color(text_dim).size(12))
            .padding([8, 14])
            .style(move |_theme, _status| button::Style {
                background: Some(Color::from_rgb8(0x26, 0x26, 0x26).into()),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..iced::Border::default()
                },
                ..button::Style::default()
            })
    };

    let actions = row![open_btn, remove_btn,].spacing(8);

    column![
        text("Font Details").size(18).color(accent),
        info_rows,
        actions,
    ]
    .spacing(12)
    .width(Length::Fill)
    .into()
}

// ── Update ──────────────────────────────────────────────────────────

pub fn update(state: &mut DetailsState, msg: DetailsMessage) -> Task<Message> {
    match msg {
        DetailsMessage::SetFont { family, style, file } => {
            let charset_count = count_chars(&file);
            let is_user = is_user_font(&file);
            state.family = family;
            state.style = style;
            state.file = file;
            state.full_name = format!("{} {}", state.family, state.style);
            state.charset_count = charset_count;
            state.charset_str = charset_count.to_string();
            state.is_user_font = is_user;
            Task::none()
        }
        DetailsMessage::RemoveFont => {
            if !state.file.is_empty() && state.is_user_font {
                let file = state.file.clone();
                // Move to trash instead of rm for safety
                let _ = Command::new("gio")
                    .args(["trash", &file])
                    .spawn();
                let _ = Command::new("fc-cache")
                    .arg("-f")
                    .spawn();
            }
            Task::none()
        }
        DetailsMessage::OpenFolder => {
            if !state.file.is_empty() {
                let path = std::path::Path::new(&state.file);
                if let Some(parent) = path.parent() {
                    let _ = Command::new("xdg-open")
                        .arg(parent)
                        .spawn();
                }
            }
            Task::none()
        }
    }
}

// ── Subscription ────────────────────────────────────────────────────

pub fn subscription(_state: &DetailsState) -> iced::Subscription<Message> {
    iced::Subscription::none()
}
