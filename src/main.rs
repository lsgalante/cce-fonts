mod pages;

use iced::widget::{column, container, row, text};
use iced::{keyboard, Color, Element, Length, Task, Theme};

use pages::Page;

// ── State ───────────────────────────────────────────────────────────

struct AppState {
    current_page: Page,
    browse: pages::browse::BrowseState,
    preview: pages::preview::PreviewState,
    details: pages::details::DetailsState,
    keybindings: pages::keybindings::KeybindingsState,
}

// ── Messages ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    SwitchPage(Page),
    Browse(pages::browse::BrowseMessage),
    Preview(pages::preview::PreviewMessage),
    Details(pages::details::DetailsMessage),
    Keybindings(pages::keybindings::KeybindingsMessage),
    KeyboardEvent(keyboard::Event),
}

// ── View ────────────────────────────────────────────────────────────

fn view(state: &AppState) -> Element<'_, Message> {
    let bg = Color::from_rgb8(0x1a, 0x2a, 0x1c);
    let sidebar_bg = Color::from_rgb8(0x16, 0x24, 0x18);

    let mut sidebar = column![text("Clear Typeface")
        .size(15)
        .color(Color::from_rgb8(0x5c, 0x90, 0x60))]
    .spacing(4)
    .padding([16, 10]);

    for page in Page::ALL {
        sidebar = sidebar.push(pages::sidebar_button(page, page == state.current_page));
    }

    let sidebar_container = container(sidebar)
        .width(200)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(sidebar_bg.into()),
            border: iced::Border {
                radius: 12.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        });

    let content: Element<Message> = match state.current_page {
        Page::Browse => browse_preview_view(state),
        Page::Keybindings => pages::keybindings::view(&state.keybindings),
    };

    let content_container = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(20)
        .style(move |_theme| container::Style {
            background: Some(bg.into()),
            border: iced::Border {
                radius: 12.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        });

    let inner = row![sidebar_container, content_container]
        .width(Length::Fill)
        .height(Length::Fill)
        .spacing(6);

    container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(6)
        .style(move |_theme| container::Style {
            background: Some(bg.into()),
            ..container::Style::default()
        })
        .into()
}

fn browse_preview_view(state: &AppState) -> Element<'_, Message> {
    let panel_bg = Color::from_rgb8(0x16, 0x24, 0x18);
    let heading = Color::from_rgb8(0x8f, 0xd4, 0x8f);

    let browse_panel = container(
        column![
            text("Browse").size(13).color(heading),
            pages::browse::view(&state.browse),
        ]
        .spacing(10)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::FillPortion(2))
    .height(Length::Fill)
    .padding(14)
    .style(move |_theme| container::Style {
        background: Some(panel_bg.into()),
        border: iced::Border {
            radius: 10.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    });

    let preview_panel = container(
        column![
            text("Preview").size(13).color(heading),
            pages::preview::view(&state.preview),
        ]
        .spacing(10)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::FillPortion(3))
    .height(Length::Fill)
    .padding(14)
    .style(move |_theme| container::Style {
        background: Some(panel_bg.into()),
        border: iced::Border {
            radius: 10.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    });

    let details_panel = container(
        column![
            text("Details").size(13).color(heading),
            pages::details::view(&state.details),
        ]
        .spacing(10)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::FillPortion(2))
    .height(Length::Fill)
    .padding(14)
    .style(move |_theme| container::Style {
        background: Some(panel_bg.into()),
        border: iced::Border {
            radius: 10.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    });

    row![browse_panel, preview_panel, details_panel]
        .spacing(12)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// ── Update ──────────────────────────────────────────────────────────

fn update(state: &mut AppState, message: Message) -> Task<Message> {
    match message {
        Message::SwitchPage(page) => {
            state.current_page = page;
            Task::none()
        }
        Message::Browse(msg) => {
            let family = match &msg {
                pages::browse::BrowseMessage::SelectFamily(idx) => {
                    state.browse.filtered.get(*idx).cloned()
                }
                _ => None,
            };
            let scroll_task = match &msg {
                pages::browse::BrowseMessage::SelectFamily(idx) => {
                    Some(pages::browse::scroll_to_selection_if_needed(&state.browse, *idx))
                }
                _ => None,
            };
            let task = pages::browse::update(&mut state.browse, msg);
            // When a family is selected in browse, push it to preview
            if let Some(family) = family {
                let preview_task = pages::preview::update(
                    &mut state.preview,
                    pages::preview::PreviewMessage::SetFamily { family },
                    &state.browse.all_fonts,
                );

                // Also update details with the preview's current style/file
                let details_task = pages::details::update(
                    &mut state.details,
                    pages::details::DetailsMessage::SetFont {
                        family: state.preview.family.clone(),
                        style: state.preview.style.clone(),
                        file: state.preview.file.clone(),
                    },
                );

                if let Some(scroll_task) = scroll_task {
                    return Task::batch(vec![task, preview_task, details_task, scroll_task]);
                }

                return Task::batch(vec![task, preview_task, details_task]);
            }
            task
        }
        Message::Preview(msg) => {
            // If a style was selected, also push it to details
            let details_msg = match &msg {
                pages::preview::PreviewMessage::StyleSelected(_) => {
                    Some(pages::details::DetailsMessage::SetFont {
                        family: state.preview.family.clone(),
                        style: state.preview.style.clone(),
                        file: state.preview.file.clone(),
                    })
                }
                _ => None,
            };
            let task = pages::preview::update(
                &mut state.preview,
                msg,
                &state.browse.all_fonts,
            );
            if details_msg.is_some() {
                // Re-read after update so style/file are current
                let details_msg = pages::details::DetailsMessage::SetFont {
                    family: state.preview.family.clone(),
                    style: state.preview.style.clone(),
                    file: state.preview.file.clone(),
                };
                let details_task = pages::details::update(&mut state.details, details_msg);
                return Task::batch(vec![task, details_task]);
            }
            task
        }
        Message::Details(msg) => pages::details::update(&mut state.details, msg),
        Message::Keybindings(msg) => pages::keybindings::update(&mut state.keybindings, msg),
        Message::KeyboardEvent(event) => {
            if state.current_page != Page::Browse {
                return Task::none();
            }

            let direction = match event {
                keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::ArrowUp),
                    ..
                } => Some(pages::browse::BrowseNavigation::Up),
                keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::ArrowDown),
                    ..
                } => Some(pages::browse::BrowseNavigation::Down),
                _ => None,
            };

            if let Some(direction) = direction {
                if let Some(index) = pages::browse::next_selection_index(&state.browse, direction) {
                    return update(state, Message::Browse(pages::browse::BrowseMessage::SelectFamily(index)));
                }
            }

            Task::none()
        }
    }
}

// ── Subscription ────────────────────────────────────────────────────

fn subscription(state: &AppState) -> iced::Subscription<Message> {
    match state.current_page {
        Page::Browse => iced::Subscription::batch(vec![
            pages::browse::subscription(&state.browse),
            keyboard::listen().map(Message::KeyboardEvent),
        ]),
        Page::Keybindings => pages::keybindings::subscription(&state.keybindings),
    }
}

// ── Boot / Theme / Title ────────────────────────────────────────────

fn boot() -> (AppState, Task<Message>) {
    let state = AppState {
        current_page: Page::Browse,
        browse: pages::browse::BrowseState::default(),
        preview: pages::preview::PreviewState::new(),
        details: pages::details::DetailsState::default(),
        keybindings: pages::keybindings::KeybindingsState,
    };

    // Load fonts on startup
    let task = Task::perform(
        async { pages::browse::fetch_fonts() },
        |f| Message::Browse(pages::browse::BrowseMessage::FontsLoaded(f)),
    );

    (state, task)
}

fn title(_state: &AppState) -> String {
    String::from("Clear Typeface Interface")
}

fn theme(_state: &AppState) -> Theme {
    Theme::Dark
}

// ── Main ────────────────────────────────────────────────────────────

fn main() -> iced::Result {
    let mut window = iced::window::Settings::default();
    window.size = iced::Size::new(1200.0, 720.0);
    window.platform_specific.application_id = String::from("clear-typeface-interface");

    iced::application(boot, update, view)
        .title(title)
        .theme(theme)
        .subscription(subscription)
        .window(window)
        .run()
}
