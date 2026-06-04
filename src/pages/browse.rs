use iced::widget::{button, column, operation, row, scrollable, text, text_input};
use iced::{Color, Element, Length, Task};
use iced::widget::scrollable::Viewport;
use std::process::Command;

use crate::Message;

// ── Data ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FontEntry {
    pub family: String,
    pub style: String,
    pub file: String,
}

#[derive(Debug, Clone, Default)]
pub struct BrowseState {
    /// All font entries from fc-list (one per family+style combo).
    pub all_fonts: Vec<FontEntry>,
    /// Deduplicated family names for the browse list.
    pub families: Vec<String>,
    /// Filtered subset of family names matching the search query.
    pub filtered: Vec<String>,
    pub search: String,
    /// Index into `filtered` — which family is highlighted.
    pub selected: Option<usize>,
    /// Tracks the scrollable viewport so we only scroll on
    /// keyboard navigation when the highlight is off-screen.
    pub scroll_offset_y: f32,
    pub viewport_height: f32,
}

#[derive(Debug, Clone)]
pub enum BrowseMessage {
    SearchChanged(String),
    SelectFamily(usize),
    FontsLoaded(Vec<FontEntry>),
    Scrolled(Viewport),
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseNavigation {
    Up,
    Down,
}

const LIST_SCROLL_ID: &str = "browse-font-list";
const LIST_ROW_HEIGHT: f32 = 34.0;
const LIST_ROW_SPACING: f32 = 2.0;

// ── Helpers ─────────────────────────────────────────────────────────

pub fn fetch_fonts() -> Vec<FontEntry> {
    let output = match Command::new("fc-list")
        .arg("--format=%{family}\\t%{style}\\t%{file}\\n")
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => return Vec::new(),
    };

    let mut fonts: Vec<FontEntry> = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() == 3 {
                Some(FontEntry {
                    family: parts[0].to_string(),
                    style: parts[1].to_string(),
                    file: parts[2].to_string(),
                })
            } else {
                None
            }
        })
        .collect();

    fonts.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()));
    fonts.dedup_by(|a, b| a.family == b.family && a.style == b.style);
    fonts
}

/// Extract deduplicated family names, sorted alphabetically.
fn extract_families(fonts: &[FontEntry]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    for f in fonts {
        seen.insert(f.family.clone());
    }
    seen.into_iter().collect()
}

fn filter_families(families: &[String], query: &str) -> Vec<String> {
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

/// Return all FontEntry items matching the given family name.
pub fn entries_for_family<'a>(all_fonts: &'a [FontEntry], family: &str) -> Vec<&'a FontEntry> {
    all_fonts
        .iter()
        .filter(|f| f.family == family)
        .collect()
}

pub fn next_selection_index(state: &BrowseState, direction: BrowseNavigation) -> Option<usize> {
    let len = state.filtered.len();

    if len == 0 {
        return None;
    }

    match (state.selected, direction) {
        (Some(index), BrowseNavigation::Up) => Some(index.saturating_sub(1)),
        (Some(index), BrowseNavigation::Down) => Some((index + 1).min(len - 1)),
        (None, BrowseNavigation::Up) => Some(len - 1),
        (None, BrowseNavigation::Down) => Some(0),
    }
}

/// Row-top Y position for the given index.
fn row_top(index: usize) -> f32 {
    index as f32 * (LIST_ROW_HEIGHT + LIST_ROW_SPACING)
}

/// Returns a scroll task only when the selected row is outside the
/// visible viewport.
pub fn scroll_to_selection_if_needed(
    state: &BrowseState,
    index: usize,
) -> Task<Message> {
    let top = row_top(index);
    let bottom = top + LIST_ROW_HEIGHT;
    let viewport_top = state.scroll_offset_y;
    let viewport_bottom = viewport_top + state.viewport_height;

    if bottom <= viewport_top {
        operation::scroll_to(
            LIST_SCROLL_ID,
            scrollable::AbsoluteOffset { x: 0.0, y: top },
        )
    } else if top >= viewport_bottom {
        let new_offset = (bottom - state.viewport_height).max(0.0);
        operation::scroll_to(
            LIST_SCROLL_ID,
            scrollable::AbsoluteOffset { x: 0.0, y: new_offset },
        )
    } else {
        Task::none()
    }
}

// ── View ────────────────────────────────────────────────────────────

pub fn view(state: &BrowseState) -> Element<'_, Message> {
    let text_fg = Color::from_rgb8(0xd4, 0xd4, 0xd4);
    let text_dim = Color::from_rgb8(0x88, 0x88, 0x99);
    let selected_bg = Color::from_rgb8(0x2a, 0x4a, 0x2e);
    let row_bg = Color::from_rgb8(0x1e, 0x2e, 0x20);

    let search_input = text_input("Search fonts...", &state.search)
        .on_input(|s| Message::Browse(BrowseMessage::SearchChanged(s)))
        .padding([8, 12]);

    let count_label = text(format!("{} families", state.filtered.len()))
        .size(11)
        .color(text_dim);

    let header = row![search_input, count_label]
        .spacing(8)
        .align_y(iced::Alignment::Center);

    let mut list = column![].spacing(2);

    for (i, family) in state.filtered.iter().enumerate() {
        let is_selected = state.selected == Some(i);
        let bg = if is_selected { selected_bg } else { row_bg };
        let fg = if is_selected {
            Color::from_rgb8(0x8f, 0xd4, 0x8f)
        } else {
            text_fg
        };

        let entry = button(
            text(family).size(13).color(fg),
        )
        .style(move |_theme, _status| button::Style {
            background: Some(bg.into()),
            border: iced::Border {
                radius: 4.0.into(),
                ..iced::Border::default()
            },
            ..button::Style::default()
        })
        .padding([6, 10])
        .height(LIST_ROW_HEIGHT)
        .width(Length::Fill)
        .on_press(Message::Browse(BrowseMessage::SelectFamily(i)));

        list = list.push(entry);
    }

    let scroll = scrollable(list)
        .id(LIST_SCROLL_ID)
        .height(Length::Fill)
        .on_scroll(|viewport| Message::Browse(BrowseMessage::Scrolled(viewport)));

    column![header, scroll,]
        .spacing(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// ── Update ──────────────────────────────────────────────────────────

pub fn update(state: &mut BrowseState, msg: BrowseMessage) -> Task<Message> {
    match msg {
        BrowseMessage::SearchChanged(q) => {
            state.search = q;
            state.filtered = filter_families(&state.families, &state.search);
            state.selected = None;
            Task::none()
        }
        BrowseMessage::SelectFamily(i) => {
            state.selected = Some(i);
            Task::none()
        }
        BrowseMessage::Scrolled(viewport) => {
            let offset = viewport.absolute_offset();
            state.scroll_offset_y = offset.y;
            state.viewport_height = viewport.bounds().height;
            Task::none()
        }
        BrowseMessage::FontsLoaded(fonts) => {
            let families = extract_families(&fonts);
            let filtered = filter_families(&families, &state.search);
            state.families = families;
            state.filtered = filtered;
            state.all_fonts = fonts;
            Task::none()
        }
        BrowseMessage::Tick => Task::perform(
            async { fetch_fonts() },
            |f| Message::Browse(BrowseMessage::FontsLoaded(f)),
        ),
    }
}

// ── Subscription ────────────────────────────────────────────────────

pub fn subscription(_state: &BrowseState) -> iced::Subscription<Message> {
    iced::Subscription::none()
}

#[cfg(test)]
mod tests {
    use super::{
        next_selection_index, row_top, BrowseNavigation, BrowseState,
        LIST_ROW_HEIGHT, LIST_ROW_SPACING, extract_families, filter_families,
    };

    fn state_with_count(count: usize) -> BrowseState {
        BrowseState {
            filtered: (0..count).map(|i| format!("Family {i}")).collect(),
            ..BrowseState::default()
        }
    }

    #[test]
    fn down_from_none_selects_first_entry() {
        let state = state_with_count(3);
        assert_eq!(next_selection_index(&state, BrowseNavigation::Down), Some(0));
    }

    #[test]
    fn up_from_none_selects_last_entry() {
        let state = state_with_count(3);
        assert_eq!(next_selection_index(&state, BrowseNavigation::Up), Some(2));
    }

    #[test]
    fn navigation_stays_in_bounds() {
        let mut state = state_with_count(3);

        state.selected = Some(0);
        assert_eq!(next_selection_index(&state, BrowseNavigation::Up), Some(0));

        state.selected = Some(2);
        assert_eq!(next_selection_index(&state, BrowseNavigation::Down), Some(2));
    }

    #[test]
    fn row_top_computes_y_position() {
        assert_eq!(row_top(0), 0.0);
        assert_eq!(row_top(1), LIST_ROW_HEIGHT + LIST_ROW_SPACING);
        assert_eq!(row_top(5), 5.0 * (LIST_ROW_HEIGHT + LIST_ROW_SPACING));
    }

    #[test]
    fn extract_families_deduplicates() {
        use super::{FontEntry, extract_families};
        let fonts = vec![
            FontEntry { family: "Arial".into(), style: "Regular".into(), file: "/a.ttf".into() },
            FontEntry { family: "Arial".into(), style: "Bold".into(), file: "/ab.ttf".into() },
            FontEntry { family: "Courier".into(), style: "Regular".into(), file: "/c.ttf".into() },
        ];
        let families = extract_families(&fonts);
        assert_eq!(families, vec!["Arial", "Courier"]);
    }

    #[test]
    fn filter_families_matches_substring() {
        let families = vec!["Arial".into(), "Courier".into(), "Arial Black".into()];
        let filtered = filter_families(&families, "arial");
        assert_eq!(filtered, vec!["Arial", "Arial Black"]);
    }
}
