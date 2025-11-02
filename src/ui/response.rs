use crate::types::{ResponseData, ResponseTab};
use crate::ui::Spinner;
use iced::highlighter::{self};
use iced::widget::button::Status;
use iced::widget::container::Style;
use iced::widget::{button, column, container, row, scrollable, space, text, text_editor};
use iced::{Background, Border, Color, Element, Length, Theme};
use log::info;

#[derive(Debug, Clone)]
pub enum Action {
    ResponseBodyAction(text_editor::Action),
    None,
}

#[derive(Debug, Clone)]
pub enum Message {
    ResponseBodyAction(text_editor::Action),
    TabSelected(ResponseTab),
    DoNothing, // Used to prevent event propagation
}

#[derive(Debug)]
pub struct ResponsePanel {
    pub selected_tab: ResponseTab,
    pub spinner: Spinner,
}

impl ResponsePanel {
    pub fn new() -> Self {
        Self {
            selected_tab: ResponseTab::Body,
            spinner: Spinner::new(),
        }
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::ResponseBodyAction(text_action) => {
                // Filter actions to allow only read-only operations
                // Allow: select, copy, scroll, move cursor
                // Block: edit actions (insert, paste, delete, etc.)
                match &text_action {
                    text_editor::Action::Move(_)
                    | text_editor::Action::Select(_)
                    | text_editor::Action::SelectWord
                    | text_editor::Action::SelectLine
                    | text_editor::Action::SelectAll
                    | text_editor::Action::Click(_)
                    | text_editor::Action::Drag(_)
                    | text_editor::Action::Scroll { .. } => {
                        // Allow read-only actions
                        // response_body_content.perform(text_action.clone());
                        return Action::ResponseBodyAction(text_action);
                    }
                    text_editor::Action::Edit(_) => {
                        // Block all edit actions (insert, paste, delete, etc.)
                        // Do nothing - this prevents editing
                    }
                }
                Action::None
            }
            Message::TabSelected(tab) => {
                self.selected_tab = tab;
                Action::None
            }
            Message::DoNothing => Action::None,
        }
    }

    pub fn view<'a>(
        &'a self,
        response: &'a Option<ResponseData>,
        response_body_content: &'a text_editor::Content,
        is_loading: bool,
        elapsed_time: u64,
    ) -> Element<'_, Message> {
        let mut status_row = vec![];

        // Add loading indicator if loading (on the left)
        if is_loading {
            status_row.push(container(self.spinner.view()).padding([0, 3]).into());
        }

        // Add response status or placeholder
        match response {
            Some(resp) => {
                let status_color = if resp.status >= 200 && resp.status < 300 {
                    Color::from_rgb(0.0, 0.8, 0.0)
                } else if resp.status >= 400 {
                    Color::from_rgb(0.8, 0.0, 0.0)
                } else {
                    Color::from_rgb(1.0, 0.6, 0.0)
                };

                status_row.push(
                    container(
                        text(format!("{} {}", resp.status, resp.status_text))
                            .color(status_color)
                            .size(14),
                    )
                    .style(move |_theme| Style {
                        background: Some(Background::Color(Color::from_rgba(
                            status_color.r,
                            status_color.g,
                            status_color.b,
                            0.1,
                        ))),
                        border: Border {
                            radius: 4.0.into(),
                            ..Border::default()
                        },
                        ..Style::default()
                    })
                    .padding([4, 8])
                    .into(),
                );

                status_row.push(space().width(20).into());
                let time_text = if is_loading {
                    format!("Time: {}ms", elapsed_time)
                } else {
                    format!("Time: {}ms", resp.time)
                };
                status_row.push(
                    text(time_text)
                        .size(14)
                        .color(Color::from_rgb(0.5, 0.5, 0.5))
                        .into(),
                );
                status_row.push(space().width(20).into());
                status_row.push(
                    text(format!("Size: {}", format_bytes(resp.size)))
                        .size(14)
                        .color(Color::from_rgb(0.5, 0.5, 0.5))
                        .into(),
                );

                let status_info: Element<'_, Message> =
                    Element::from(row(status_row).align_y(iced::Alignment::Center))
                        .map(|_| Message::DoNothing);

                let tabs = row![
                    response_tab_button(
                        "Body",
                        self.selected_tab == ResponseTab::Body,
                        ResponseTab::Body
                    ),
                    response_tab_button(
                        "Headers",
                        self.selected_tab == ResponseTab::Headers,
                        ResponseTab::Headers
                    ),
                ]
                .spacing(5);

                let tab_content = match self.selected_tab {
                    ResponseTab::Body => response_body_tab(resp, response_body_content),
                    ResponseTab::Headers => match response {
                        Some(resp) => response_headers_tab(&resp),
                        None => container(
                            text("No headers available")
                                .size(14)
                                .color(Color::from_rgb(0.5, 0.5, 0.5)),
                        )
                        .padding(20)
                        .center_x(Length::Fill)
                        .into(),
                    },
                };

                column![
                    status_info,
                    space().height(0.5),
                    tabs,
                    space().height(0.5),
                    tab_content
                ]
                .spacing(10)
                .padding(15)
                .into()
            }
            None => {
                if !is_loading {
                    container(column![
                        space().height(100),
                        container(
                            text("No response yet")
                                .size(16)
                                .color(Color::from_rgb(0.5, 0.5, 0.5))
                        )
                        .center_x(Length::Fill)
                        .width(Length::Fill),
                        container(
                            text("Send a request to see the response here")
                                .size(14)
                                .color(Color::from_rgb(0.7, 0.7, 0.7))
                        )
                        .center_x(Length::Fill)
                        .width(Length::Fill),
                        space().height(100),
                    ])
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
                } else {
                    // container(row![]).into()
                    // Show loading status when no response exists yet
                    column![
                        row![
                            container(self.spinner.view().map(|_| Message::DoNothing))
                                .padding([0, 3]),
                            space().width(20),
                            text(format!("Time: {}ms", elapsed_time))
                                .size(12)
                                .color(Color::from_rgb(0.5, 0.5, 0.5)),
                        ]
                        .align_y(iced::Alignment::Center),
                    ]
                    .spacing(10)
                    .padding(10)
                    .into()
                }
            }
        }
    }

    pub fn set_selected_tab(&mut self, tab: ResponseTab) {
        self.selected_tab = tab;
    }

    pub fn update_spinner(&mut self) {
        self.spinner.update();
    }
}

fn response_tab_button<'a>(
    label: &'a str,
    is_active: bool,
    tab: ResponseTab,
) -> Element<'a, Message> {
    button(text(label))
        .on_press(Message::TabSelected(tab))
        .style(move |_theme, status| {
            let base = button::Style::default();
            if is_active {
                button::Style {
                    background: Some(Background::Color(Color::from_rgb(0.0, 0.5, 1.0))),
                    text_color: Color::WHITE,
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..base
                }
            } else {
                match status {
                    Status::Hovered => button::Style {
                        background: Some(Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                        border: Border {
                            radius: 4.0.into(),
                            ..Border::default()
                        },
                        ..base
                    },
                    _ => button::Style {
                        background: Some(Background::Color(Color::from_rgb(0.95, 0.95, 0.95))),
                        border: Border {
                            radius: 4.0.into(),
                            ..Border::default()
                        },
                        ..base
                    },
                }
            }
        })
        .into()
}

fn response_body_tab<'a>(
    resp: &'a ResponseData,
    content: &'a text_editor::Content,
) -> Element<'a, Message> {
    info!("===response body content");
    // For text responses, use the normal text editor with dynamic syntax highlighting
    // TODO: move the syntax to main file
    info!("===response body updated");

    // let resp = response.as_ref().unwrap();
    // Check if this is a binary response
    if resp.is_binary {
        // For binary responses, show metadata instead of content
        let binary_info = column![
            container(
                text("Binary Response")
                    .size(16)
                    .color(Color::from_rgb(0.0, 0.5, 1.0))
            )
            .style(|_theme| Style {
                background: Some(Background::Color(Color::from_rgba(0.0, 0.5, 1.0, 0.1))),
                border: Border {
                    radius: 4.0.into(),
                    ..Border::default()
                },
                ..Style::default()
            })
            .padding([8, 12]),
            space().height(10),
            text(format!("Content-Type: {}", resp.content_type))
                .size(14)
                .color(Color::from_rgb(0.3, 0.3, 0.3)),
            text(format!("Size: {}", format_bytes(resp.size)))
                .size(14)
                .color(Color::from_rgb(0.3, 0.3, 0.3)),
            space().height(15),
            text("Preview (first 100 bytes as hex):")
                .size(14)
                .color(Color::from_rgb(0.3, 0.3, 0.3)),
            space().height(5),
            scrollable(
                container(
                    text(resp.body.as_str())
                        .size(12)
                        .color(Color::from_rgb(0.5, 0.5, 0.5))
                )
                .style(|_theme| Style {
                    background: Some(Background::Color(Color::from_rgb(0.98, 0.98, 0.98))),
                    border: Border {
                        radius: 4.0.into(),
                        width: 1.0,
                        color: Color::from_rgb(0.9, 0.9, 0.9),
                    },
                    ..Style::default()
                })
                .padding(10)
            )
            .height(Length::Fill)
        ]
        .spacing(5);

        scrollable(binary_info).height(Length::Fill).into()
    } else {
        let syntax_language = get_syntax_from_content_type(&resp.content_type);
        let body_column = text_editor(content)
            .highlight(syntax_language, highlighter::Theme::SolarizedDark)
            .on_action(Message::ResponseBodyAction)
            .style(
                |theme: &Theme, _status: text_editor::Status| text_editor::Style {
                    background: Background::Color(theme.palette().background),
                    border: Border {
                        color: Color::from_rgb(0.9, 0.9, 0.9),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    placeholder: Color::from_rgb(0.6, 0.6, 0.6),
                    value: theme.palette().text,
                    selection: theme.palette().primary,
                },
            );

        scrollable(body_column).height(Length::Fill).into()
    }
}

fn response_headers_tab<'a>(response: &'a ResponseData) -> Element<'a, Message> {
    let mut content = column![text("Response Headers").size(16), space().height(10)];

    for (key, value) in &response.headers {
        let header_row = row![
            container(text(key).size(14).color(Color::from_rgb(0.3, 0.3, 0.3)))
                .width(Length::FillPortion(1)),
            container(text(value).size(14)).width(Length::FillPortion(2))
        ]
        .spacing(20)
        .padding([5, 0]);

        content = content.push(header_row);
    }

    scrollable(content.spacing(5)).height(Length::Fill).into()
}

fn response_text_editor_style(
    theme: &iced::Theme,
    _status: text_editor::Status,
) -> text_editor::Style {
    text_editor::Style {
        background: Background::Color(theme.palette().background),
        border: Border {
            color: Color::from_rgb(0.9, 0.9, 0.9),
            width: 1.0,
            radius: 4.0.into(),
        },
        placeholder: Color::from_rgb(0.6, 0.6, 0.6),
        value: theme.palette().text,
        selection: Color::from_rgba(0.0, 0.5, 1.0, 0.2), // Light blue selection with 20% opacity
    }
}

fn format_bytes(bytes: usize) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    const THRESHOLD: f64 = 1024.0;

    if bytes == 0 {
        return "0 B".to_string();
    }

    let bytes_f = bytes as f64;
    let unit_index = (bytes_f.log(THRESHOLD).floor() as usize).min(UNITS.len() - 1);
    let size = bytes_f / THRESHOLD.powi(unit_index as i32);

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else if size >= 100.0 {
        format!("{:.0} {}", size, UNITS[unit_index])
    } else if size >= 10.0 {
        format!("{:.1} {}", size, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

/// Maps content-type to appropriate syntax highlighting language
fn get_syntax_from_content_type(content_type: &str) -> &'static str {
    let content_type_lower = content_type.to_lowercase();

    if content_type_lower.contains("json") {
        "json"
    } else if content_type_lower.contains("xml") || content_type_lower.contains("html") {
        "xml"
    } else if content_type_lower.contains("javascript") || content_type_lower.contains("js") {
        "javascript"
    } else if content_type_lower.contains("css") {
        "css"
    } else if content_type_lower.contains("yaml") || content_type_lower.contains("yml") {
        "yaml"
    } else if content_type_lower.contains("sql") {
        "sql"
    } else if content_type_lower.contains("python") {
        "python"
    } else if content_type_lower.contains("rust") {
        "rust"
    } else if content_type_lower.contains("c++") || content_type_lower.contains("cpp") {
        "cpp"
    } else if content_type_lower.contains("java") {
        "java"
    } else if content_type_lower.contains("markdown") || content_type_lower.contains("md") {
        "markdown"
    } else if content_type_lower.contains("toml") {
        "toml"
    } else if content_type_lower.contains("ini") {
        "ini"
    } else if content_type_lower.contains("bash") || content_type_lower.contains("shell") {
        "bash"
    } else {
        // Default to JSON for unknown content types
        "json"
    }
}
