use crate::constant::{RESPONSE_BODY_EDITOR_ID, RESPONSE_BODY_SCROLLABLE_ID};
use crate::types::{ResponseData, ResponseTab};
use crate::ui::floating_element;
use crate::ui::undoable_editor::{self, UndoableEditor};
use crate::ui::{IconName, Spinner, icon};
use iced::highlighter::{self};
use iced::widget::button::Status;
use iced::widget::container::Style;
use iced::widget::{
    button, column, container, row, scrollable, space, text, text_editor, text_input,
};
use iced::{Background, Border, Color, Element, Length, Padding, Theme};

#[derive(Debug)]
pub enum Action {
    ResponseBodyAction(text_editor::Action),
    FormatResponseBody(String),
    SearchNext(iced::widget::Id),
    SearchPrevious(iced::widget::Id),
    SubmitSearch(iced::widget::Id),
    Focus(iced::widget::Id),
    // The components needs to run a task
    Run(iced::Task<Message>),
    None,
}

#[derive(Debug, Clone)]
pub enum Message {
    EditorMessage(undoable_editor::Message),
    TabSelected(ResponseTab),
    FormatResponseBody,
    SearchQueryChanged(String),
    FindNext,
    FindPrevious,
    SubmitSearch,
    CloseSearch,
    OpenSearch,
    SearchFound(text_editor::Position, text_editor::Position),
    SearchNotFound,
    FocusSearch,
    DoNothing, // Used to prevent event propagation
    ScrollToMatchResponse(f32),
}

#[derive(Debug)]
pub struct ResponsePanel {
    pub selected_tab: ResponseTab,
    pub spinner: Spinner,
    pub show_search: bool,
    pub search_query: String,
    pub search_input_id: iced::widget::Id,
    pub search_selection: Option<(text_editor::Position, text_editor::Position)>,
    pub body_editor: UndoableEditor,
}

impl ResponsePanel {
    pub fn new() -> Self {
        Self {
            selected_tab: ResponseTab::Body,
            spinner: Spinner::new(),
            show_search: false,
            search_query: String::new(),
            search_input_id: iced::widget::Id::unique(),
            search_selection: None,
            body_editor: UndoableEditor::new_empty(),
        }
    }

    pub fn update(&mut self, message: Message, response: &Option<ResponseData>) -> Action {
        match message {
            Message::EditorMessage(editor_message) => {
                match editor_message {
                    undoable_editor::Message::Action(action) => {
                        match &action {
                            text_editor::Action::Move(_)
                            | text_editor::Action::Select(_)
                            | text_editor::Action::SelectWord
                            | text_editor::Action::SelectLine
                            | text_editor::Action::SelectAll
                            | text_editor::Action::Click(_)
                            | text_editor::Action::Drag(_)
                            | text_editor::Action::Scroll { .. } => {
                                // Allow read-only actions
                                return Action::ResponseBodyAction(action);
                            }
                            text_editor::Action::Edit(_) => {
                                // Block all edit actions (insert, paste, delete, etc.)
                                // Do nothing - this prevents editing
                            }
                        }
                    }
                    undoable_editor::Message::Find => {
                        return Action::Run(iced::Task::perform(async {}, |_| Message::OpenSearch));
                    }
                    _ => {}
                }
                Action::None
            }
            Message::TabSelected(tab) => {
                self.selected_tab = tab;
                Action::None
            }
            Message::FormatResponseBody => {
                if let Some(resp) = response {
                    let mut formatted_body = None;
                    let content_type = &resp.content_type;
                    let body = &resp.body;

                    if content_type.contains("json") {
                        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(body) {
                            if let Ok(formatted) = serde_json::to_string_pretty(&json_value) {
                                formatted_body = Some(formatted);
                            }
                        }
                    } else if content_type.contains("xml") || content_type.contains("html") {
                        // Simple trim for XML/HTML for now
                        formatted_body = Some(body.trim().to_string());
                    } else {
                        // Default trim
                        formatted_body = Some(body.trim().to_string());
                    }

                    if let Some(formatted) = formatted_body {
                        Action::FormatResponseBody(formatted)
                    } else {
                        Action::None
                    }
                } else {
                    Action::None
                }
            }
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
                Action::None
            }
            Message::FindNext => Action::SearchNext(self.search_input_id.clone()),
            Message::FindPrevious => Action::SearchPrevious(self.search_input_id.clone()),
            Message::SubmitSearch => Action::SubmitSearch(self.search_input_id.clone()),
            Message::SearchFound(start, end) => {
                self.search_selection = Some((start, end));
                Action::Run(
                    iced::advanced::widget::operate(crate::ui::editor_view::QueryScrollY::new(
                        start.line,
                    ))
                    .map(Message::ScrollToMatchResponse),
                )
            }
            Message::ScrollToMatchResponse(y) => {
                let viewport_height = 400.0;
                let offset_y = (y - viewport_height / 2.0).max(0.0);
                Action::Run(
                    iced::widget::operation::scroll_to(
                        iced::widget::Id::new(crate::constant::RESPONSE_BODY_SCROLLABLE_ID),
                        iced::widget::scrollable::AbsoluteOffset {
                            x: None,
                            y: Some(offset_y),
                        },
                    )
                    .map(|_: ()| Message::DoNothing),
                )
            }
            Message::SearchNotFound => {
                self.search_selection = None;
                Action::None
            }
            Message::CloseSearch => {
                self.show_search = false;
                self.search_query.clear();
                self.search_selection = None;
                Action::None
            }
            Message::OpenSearch => {
                self.show_search = true;
                Action::Focus(self.search_input_id.clone())
            }
            Message::FocusSearch => Action::Focus(self.search_input_id.clone()),
            Message::DoNothing => Action::None,
        }
    }

    pub fn view<'a>(
        &'a self,
        response: &'a Option<ResponseData>,
        response_body_content: &'a text_editor::Content,
        is_loading: bool,
        elapsed_time: u64,
    ) -> Element<'a, Message> {
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
                    ResponseTab::Body => self.response_body_tab(resp, response_body_content),
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

    fn response_body_tab<'a>(
        &'a self,
        resp: &'a ResponseData,
        content: &'a text_editor::Content,
    ) -> Element<'a, Message> {
        // For text responses, use the normal text editor with dynamic syntax highlighting
        // TODO: move the syntax to main file

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

            let body_column = self
                .body_editor
                .view(
                    iced::widget::Id::new(RESPONSE_BODY_EDITOR_ID),
                    content,
                    Some(syntax_language),
                    Some(self.search_query.as_str()),
                    self.search_selection,
                )
                .map(Message::EditorMessage);

            let format_button = response_format_button();

            let editor_with_format = floating_element::FloatingElement::new(
                scrollable(body_column)
                    .id(iced::widget::Id::new(RESPONSE_BODY_SCROLLABLE_ID))
                    .height(Length::Fill),
                format_button,
            )
            .offset(iced::Vector::new(10.0, 5.0))
            .position(floating_element::AnchorPosition::TopRight)
            .height(Length::Fill);

            if self.show_search {
                let search_bar = container(
                    row![
                        text_input("Find", &self.search_query)
                            .id(self.search_input_id.clone())
                            .on_input(Message::SearchQueryChanged)
                            .on_submit(Message::SubmitSearch)
                            .width(Length::Fixed(200.0))
                            .padding(1)
                            .style(|theme: &Theme, _status| text_input::Style {
                                background: Background::Color(Color::WHITE),
                                border: Border {
                                    width: 0.0,
                                    color: Color::TRANSPARENT,
                                    radius: 6.0.into(),
                                },
                                icon: theme.palette().text,
                                placeholder: Color::from_rgb(0.6, 0.6, 0.6),
                                value: theme.palette().text,
                                selection: theme.palette().primary,
                            }),
                        button(
                            icon(IconName::ChevronDown)
                                .size(14)
                                .color(Color::from_rgb(0.4, 0.4, 0.4))
                        )
                        .on_press(Message::FindNext)
                        .padding(1)
                        .style(|_theme, status| {
                            let base = button::Style {
                                background: None,
                                border: Border {
                                    radius: 6.0.into(),
                                    ..Border::default()
                                },
                                ..button::Style::default()
                            };
                            match status {
                                button::Status::Hovered => button::Style {
                                    background: Some(Background::Color(Color::from_rgb(
                                        0.85, 0.85, 0.85,
                                    ))),
                                    ..base
                                },
                                _ => base,
                            }
                        }),
                        button(
                            icon(IconName::ChevronUp)
                                .size(14)
                                .color(Color::from_rgb(0.4, 0.4, 0.4))
                        )
                        .on_press(Message::FindPrevious)
                        .padding(1)
                        .style(|_theme, status| {
                            let base = button::Style {
                                background: None,
                                border: Border {
                                    radius: 6.0.into(),
                                    ..Border::default()
                                },
                                ..button::Style::default()
                            };
                            match status {
                                button::Status::Hovered => button::Style {
                                    background: Some(Background::Color(Color::from_rgb(
                                        0.85, 0.85, 0.85,
                                    ))),
                                    ..base
                                },
                                _ => base,
                            }
                        }),
                        button(
                            icon(IconName::Close)
                                .size(14)
                                .color(Color::from_rgb(0.4, 0.4, 0.4))
                        )
                        .on_press(Message::CloseSearch)
                        .padding(1)
                        .style(|_theme, status| {
                            let base = button::Style {
                                background: None,
                                border: Border {
                                    radius: 6.0.into(),
                                    ..Border::default()
                                },
                                ..button::Style::default()
                            };
                            match status {
                                button::Status::Hovered => button::Style {
                                    background: Some(Background::Color(Color::from_rgb(
                                        0.85, 0.85, 0.85,
                                    ))),
                                    ..base
                                },
                                _ => base,
                            }
                        })
                    ]
                    .spacing(3)
                    .align_y(iced::Alignment::Center),
                )
                .padding(1)
                .style(|_theme: &Theme| container::Style {
                    background: Some(Color::from_rgb(0.95, 0.95, 0.95).into()),
                    border: Border {
                        color: Color::from_rgb(0.8, 0.8, 0.8),
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                });

                floating_element::FloatingElement::new(editor_with_format, search_bar)
                    .offset(iced::Vector::new(10.0, 0.0))
                    .position(floating_element::AnchorPosition::BottomRight)
                    .height(Length::Fill)
                    .into()
            } else {
                editor_with_format.into()
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

fn response_format_button() -> Element<'static, Message> {
    button(
        icon(IconName::Indent)
            .size(28)
            .color(Color::from_rgb(0.5, 0.5, 0.5)),
    )
    .on_press(Message::FormatResponseBody)
    .width(Length::Fixed(32.0))
    .height(Length::Fixed(32.0))
    .padding(Padding::from(6.0))
    .style(move |_theme, status| {
        let base = button::Style::default();
        match status {
            Status::Hovered => button::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..base
            },
            _ => button::Style {
                background: Some(iced::Background::Color(Color::TRANSPARENT)),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..base
            },
        }
    })
    .into()
}

fn response_headers_tab<'a>(response: &'a ResponseData) -> Element<'a, Message> {
    let mut content = column![];

    for (key, value) in &response.headers {
        let header_row = row![
            container(
                text_input("", key)
                    .size(14)
                    .style(|theme: &Theme, _status| text_input::Style {
                        background: Background::Color(Color::TRANSPARENT),
                        border: Border::default(),
                        icon: Color::TRANSPARENT,
                        placeholder: Color::from_rgb(0.3, 0.3, 0.3),
                        value: Color::from_rgb(0.3, 0.3, 0.3),
                        selection: theme.palette().primary,
                    })
                    .on_input(|_| Message::DoNothing) // Read-only behavior
            )
            .width(Length::FillPortion(1)),
            container(
                text_input("", value)
                    .size(14)
                    .style(|theme: &Theme, _status| text_input::Style {
                        background: Background::Color(Color::TRANSPARENT),
                        border: Border::default(),
                        icon: Color::TRANSPARENT,
                        placeholder: Color::from_rgb(0.0, 0.0, 0.0),
                        value: Color::from_rgb(0.0, 0.0, 0.0),
                        selection: theme.palette().primary,
                    })
                    .on_input(|_| Message::DoNothing) // Read-only behavior
            )
            .width(Length::FillPortion(2))
        ]
        .spacing(20)
        .padding([5, 0]);

        content = content.push(header_row);
    }

    scrollable(content.spacing(5)).height(Length::Fill).into()
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
