use crate::types::{ResponseData, ResponseTab, Message, ResponsePanel};
use iced::widget::{
    button, column, container, row, text, scrollable, text_editor, space
};
use iced::{Element, Length, Color, Background, Border, Task};
use iced::widget::container::Style;
use iced::widget::button::Status;
use iced::highlighter::{self};
use log::{info};

fn response_text_editor_style(theme: &iced::Theme, _status: text_editor::Status) -> text_editor::Style {
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

fn response_tab_button<'a>(label: &'a str, is_active: bool, tab: ResponseTab) -> Element<'a, Action> {
    button(text(label))
        .on_press(Action::TabSelected(tab))
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

fn response_body_tab<'a>(content: &'a text_editor::Content, response: &'a Option<ResponseData>) -> Element<'a, Action> {
    let mut body_column = column![];

    info!("===response body content");
    // For text responses, use the normal text editor with dynamic syntax highlighting
    // TODO: move the syntax to main file
    // let syntax_language = get_syntax_from_content_type(&response.content_type);
    body_column = body_column
      .push(
        text_editor(content)
        .highlight("json", highlighter::Theme::SolarizedDark)
        .on_action(Action::ResponseBodyAction)
        .height(Length::Fill)
        .style(response_text_editor_style)
      );
    info!("===response body updated");

    // Check if this is a binary response
    // if let Some(resp) = response {
    //     if resp.is_binary {
    //         // For binary responses, show metadata instead of content
    //         let binary_info = column![
    //             container(
    //                 text("Binary Response")
    //                     .size(16)
    //                     .color(Color::from_rgb(0.0, 0.5, 1.0))
    //             )
    //             .style(|_theme| Style {
    //                 background: Some(Background::Color(Color::from_rgba(0.0, 0.5, 1.0, 0.1))),
    //                 border: Border {
    //                     radius: 4.0.into(),
    //                     ..Border::default()
    //                 },
    //                 ..Style::default()
    //             })
    //             .padding([8, 12]),

    //             space().height(10),

    //             text(format!("Content-Type: {}", resp.content_type))
    //                 .size(14)
    //                 .color(Color::from_rgb(0.3, 0.3, 0.3)),

    //             text(format!("Size: {}", format_bytes(resp.size)))
    //                 .size(14)
    //                 .color(Color::from_rgb(0.3, 0.3, 0.3)),

    //             space().height(15),

    //             text("Preview (first 100 bytes as hex):")
    //                 .size(14)
    //                 .color(Color::from_rgb(0.3, 0.3, 0.3)),

    //             space().height(5),

    //             scrollable(
    //                 container(
    //                     text(&resp.body)
    //                         .size(12)
    //                         .color(Color::from_rgb(0.5, 0.5, 0.5))
    //                 )
    //                 .style(|_theme| Style {
    //                     background: Some(Background::Color(Color::from_rgb(0.98, 0.98, 0.98))),
    //                     border: Border {
    //                         radius: 4.0.into(),
    //                         width: 1.0,
    //                         color: Color::from_rgb(0.9, 0.9, 0.9),
    //                     },
    //                     ..Style::default()
    //                 })
    //                 .padding(10)
    //             )
    //             .height(Length::Fill)
    //         ]
    //         .spacing(5);

    //         body_column = body_column.push(binary_info);
    //     } else {
    //     }
    // } else {
    //     // No response yet, show empty text editor
    //     body_column = body_column
    //         .push(
    //             text_editor(content)
    //                 .on_action(Message::ResponseBodyAction)
    //                 .height(Length::Fill)
    //                 .style(|theme: &iced::Theme, _status: text_editor::Status| {
    //                     text_editor::Style {
    //                         background: Background::Color(theme.palette().background),
    //                         border: Border {
    //                             color: Color::from_rgb(0.9, 0.9, 0.9),
    //                             width: 1.0,
    //                             radius: 4.0.into(),
    //                         },
    //                         placeholder: Color::from_rgb(0.6, 0.6, 0.6),
    //                         value: theme.palette().text,
    //                         selection: theme.palette().primary,
    //                     }
    //                 })
    //         );
    // }

    body_column.spacing(0.0).into()
}

fn response_headers_tab<'a>(response: &'a ResponseData) -> Element<'a, Action> {
    let mut content = column![
        text("Response Headers").size(16),
        space().height(10)
    ];

    for (key, value) in &response.headers {
        let header_row = row![
            container(
                text(key)
                    .size(14)
                    .color(Color::from_rgb(0.3, 0.3, 0.3))
            )
            .width(Length::FillPortion(1)),
            container(
                text(value)
                    .size(14)
            )
            .width(Length::FillPortion(2))
        ]
        .spacing(20)
        .padding([5, 0]);

        content = content.push(header_row);
    }

    scrollable(content.spacing(5))
        .height(Length::Fill)
        .into()
}

#[derive(Debug, Clone)]
pub enum Action {
    ResponseBodyAction(text_editor::Action),
    TabSelected(ResponseTab),
    None,
}

impl ResponsePanel {
    pub fn new() -> Self {
        Self {
            response: None,
            response_body_content: text_editor::Content::new(),
            selected_tab: ResponseTab::Body,
            is_loading: false,
            current_elapsed_time: 0,
            spinner: crate::ui::Spinner::new(),
        }
    }

    pub fn update(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::ResponseBodyAction(text_action) => {
                // Filter actions to allow only read-only operations
                // Allow: select, copy, scroll, move cursor
                // Block: edit actions (insert, paste, delete, etc.)
                match &text_action {
                    text_editor::Action::Move(_) |
                    text_editor::Action::Select(_) |
                    text_editor::Action::SelectWord |
                    text_editor::Action::SelectLine |
                    text_editor::Action::SelectAll |
                    text_editor::Action::Click(_) |
                    text_editor::Action::Drag(_) |
                    text_editor::Action::Scroll { .. } => {
                        // Allow read-only actions
                        self.response_body_content.perform(text_action.clone());
                    }
                    text_editor::Action::Edit(_) => {
                        // Block all edit actions (insert, paste, delete, etc.)
                        // Do nothing - this prevents editing
                    }
                }
                Task::none()
            }
            Action::TabSelected(tab) => {
                self.selected_tab = tab;
                Task::none()
            }
            Action::None => Task::none(),
        }
    }

    pub fn render<'a>(&'a self) -> Element<'a, Action> {
        let mut status_row = vec![];

        // Add loading indicator if loading (on the left)
        if self.is_loading {
            status_row.push(
                container(self.spinner.view())
                .padding([0, 3])
                .into(),
            );
        }

        // Add response status or placeholder
        match &self.response {
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
                            .size(14)
                    )
                    .style(move |_theme| Style {
                        background: Some(Background::Color(Color::from_rgba(
                            status_color.r, status_color.g, status_color.b, 0.1
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
                let time_text = if self.is_loading {
                    format!("Time: {}ms", self.current_elapsed_time)
                } else {
                    format!("Time: {}ms", resp.time)
                };
                status_row.push(
                    text(time_text)
                        .size(12)
                        .color(Color::from_rgb(0.5, 0.5, 0.5))
                        .into(),
                );
                status_row.push(space().width(20).into());
                status_row.push(
                    text(format!("Size: {}", format_bytes(resp.size)))
                        .size(12)
                        .color(Color::from_rgb(0.5, 0.5, 0.5))
                        .into(),
                );
            }
            None => {
                if !self.is_loading {
                    status_row.push(
                        container(
                            text("Ready to send request")
                                .size(14)
                                .color(Color::from_rgb(0.5, 0.5, 0.5))
                        )
                        .padding([4, 8])
                        .into(),
                    );
                } else {
                    status_row.push(space().width(20).into());
                    status_row.push(
                        text(format!("Time: {}ms", self.current_elapsed_time))
                            .size(12)
                            .color(Color::from_rgb(0.5, 0.5, 0.5))
                            .into(),
                    );
                }
            }
        }

        let status_info: Element<'_, Action> = Element::from(row(status_row)
            .align_y(iced::Alignment::Center))
            .map(|_| Action::None);

        let tabs = row![
            response_tab_button("Body", self.selected_tab == ResponseTab::Body, ResponseTab::Body),
            response_tab_button("Headers", self.selected_tab == ResponseTab::Headers, ResponseTab::Headers),
        ]
        .spacing(5);

        let tab_content = match self.selected_tab {
            ResponseTab::Body => response_body_tab(&self.response_body_content, &self.response),
            ResponseTab::Headers => {
                match &self.response {
                    Some(resp) => response_headers_tab(resp),
                    None => container(
                        text("No headers available")
                            .size(14)
                            .color(Color::from_rgb(0.5, 0.5, 0.5))
                    )
                    .padding(20)
                    .center_x(Length::Fill)
                    .into()
                }
            }
        };

        column![
            status_info,
            space().height(2),
            tabs,
            space().height(2),
            tab_content
        ]
        .spacing(10)
        .padding(15)
        .into()
    }

    pub fn set_response(&mut self, response: Option<ResponseData>) {
        self.response = response;
    }

    pub fn set_loading(&mut self, is_loading: bool) {
        self.is_loading = is_loading;
    }

    pub fn set_selected_tab(&mut self, tab: ResponseTab) {
        self.selected_tab = tab;
    }

    pub fn set_elapsed_time(&mut self, elapsed_time: u64) {
        self.current_elapsed_time = elapsed_time;
    }

    pub fn update_spinner(&mut self) {
        self.spinner.update();
    }

    pub fn get_response_body_content(&self) -> &text_editor::Content {
        &self.response_body_content
    }

    pub fn get_response_body_content_mut(&mut self) -> &mut text_editor::Content {
        &mut self.response_body_content
    }
}
