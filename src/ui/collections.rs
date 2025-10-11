use crate::types::{RequestCollection, SavedRequest, Message, HttpMethod};
use crate::ui::{icon, IconName};
use iced::widget::{button, column, container, row, text, scrollable, space, svg};
use iced::{Element, Length, Color, Background, Border, Shadow, Vector};
use iced::widget::container::Style;
use iced::widget::button::Status;
use iced_aw::ContextMenu;

pub fn collections_panel<'a>(
    collections: &'a [RequestCollection],
    last_opened_request: Option<(usize, usize)>,
) -> Element<'a, Message> {
    let mut content = column![];

    for (collection_index, collection) in collections.iter().enumerate() {
        let collection_header = button(
            row![
                icon(
                    if collection.expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    }
                )
                .size(12),
                space().width(5),
                text(&collection.name).size(14)
            ]
            .align_y(iced::Alignment::Center)
        )
        .on_press(Message::CollectionToggled(collection_index))
        .style(move |theme, status| {
            let base = button::Style::default();
            match status {
                Status::Hovered => button::Style {
                    background: Some(Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                    ..base
                },
                _ => base,
            }
        })
        .width(Length::Fill);

        // Wrap the collection header with ContextMenu
        let collection_with_context_menu = ContextMenu::new(
            collection_header,
            move || {
                container(
                    column![
                        button(text("Add Request"))
                            .on_press(Message::AddHttpRequest(collection_index))
                            .width(Length::Fill)
                            .style(|theme, status| {
                                let base = button::Style::default();
                                match status {
                                    Status::Hovered => button::Style {
                                        background: Some(Background::Color(Color::from_rgb(0.7, 0.7, 0.7))),
                                        ..base
                                    },
                                    _ => button::Style {
                                        background: Some(Background::Color(Color::TRANSPARENT)),
                                        ..base
                                    },
                                }
                            }),
                        button(text("Add Folder"))
                            .on_press(Message::AddFolder(collection_index))
                            .width(Length::Fill)
                            .style(|theme, status| {
                                let base = button::Style::default();
                                match status {
                                    Status::Hovered => button::Style {
                                        background: Some(Background::Color(Color::from_rgb(0.7, 0.7, 0.7))),
                                        ..base
                                    },
                                    _ => button::Style {
                                        background: Some(Background::Color(Color::TRANSPARENT)),
                                        ..base
                                    },
                                }
                            }),
                        button(text("Rename"))
                            .on_press(Message::RenameFolder(collection_index))
                            .width(Length::Fill)
                            .style(|theme, status| {
                                let base = button::Style::default();
                                match status {
                                    Status::Hovered => button::Style {
                                        background: Some(Background::Color(Color::from_rgb(0.7, 0.7, 0.7))),
                                        ..base
                                    },
                                    _ => button::Style {
                                        background: Some(Background::Color(Color::TRANSPARENT)),
                                        ..base
                                    },
                                }
                            }),
                        button(text("Delete"))
                            .on_press(Message::DeleteFolder(collection_index))
                            .width(Length::Fill)
                            .style(|theme, status| {
                                let base = button::Style::default();
                                match status {
                                    Status::Hovered => button::Style {
                                        background: Some(Background::Color(Color::from_rgb(0.7, 0.7, 0.7))),
                                        ..base
                                    },
                                    _ => button::Style {
                                        background: Some(Background::Color(Color::TRANSPARENT)),
                                        ..base
                                    },
                                }
                            }),
                    ]
                    .spacing(2)
                )
                .width(Length::Fixed(150.0))
                .style(|theme| Style {
                    background: Some(Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                    border: Border {
                        color: Color::from_rgb(0.8, 0.8, 0.8),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    shadow: Shadow {
                        color: Color::from_rgba(0.0, 0.0, 0.0, 0.1),
                        offset: Vector::new(2.0, 2.0),
                        blur_radius: 4.0,
                    },
                    ..Style::default()
                })
                .padding(4)
                .into()
            }
        );

        content = content.push(collection_with_context_menu);

        if collection.expanded {
            for (request_index, request) in collection.requests.iter().enumerate() {
                let is_selected = last_opened_request == Some((collection_index, request_index));

                let request_button = button(
                    row![
                        space().width(20),
                        method_badge(&request.method),
                        space().width(8),
                        text(&request.name).size(12)
                    ]
                    .align_y(iced::Alignment::Center)
                )
                .on_press(Message::RequestSelected(collection_index, request_index))
                .style(move |theme, status| {
                    let base = button::Style::default();

                    match status {
                        Status::Pressed => {
                            if is_selected {
                                button::Style {
                                    background: Some(Background::Color(Color::from_rgb(0.78, 0.82, 0.996))), // #c7d2fe
                                    ..base
                                }
                            } else {
                                button::Style {
                                    background: Some(Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                                    ..base
                                }
                            }
                        }
                        Status::Hovered => {
                            if is_selected {
                                button::Style {
                                    background: Some(Background::Color(Color::from_rgb(0.78, 0.82, 0.996))), // #c7d2fe
                                    ..base
                                }
                            } else {
                                button::Style {
                                    background: Some(Background::Color(Color::from_rgb(0.95, 0.95, 0.95))),
                                    ..base
                                }
                            }
                        }
                        _ => {
                            if is_selected {
                                button::Style {
                                    background: Some(Background::Color(Color::from_rgb(0.78, 0.82, 0.996))), // #c7d2fe
                                    ..base
                                }
                            } else {
                                base
                            }
                        }
                    }
                })
                .width(Length::Fill);

                // Wrap the request button with ContextMenu
                let request_with_context_menu = ContextMenu::new(
                    request_button,
                    move || {
                        container(
                            column![
                                button(text("Send Request"))
                                    .on_press(Message::SendRequestFromMenu(collection_index, request_index))
                                    .width(Length::Fill)
                                    .style(|theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(Color::from_rgb(0.7, 0.7, 0.7))),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(Color::TRANSPARENT)),
                                                ..base
                                            },
                                        }
                                    }),
                                button(text("Copy as cURL"))
                                    .on_press(Message::CopyRequestAsCurl(collection_index, request_index))
                                    .width(Length::Fill)
                                    .style(|theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(Color::from_rgb(0.7, 0.7, 0.7))),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(Color::TRANSPARENT)),
                                                ..base
                                            },
                                        }
                                    }),
                                button(text("Rename"))
                                    .on_press(Message::RenameRequest(collection_index, request_index))
                                    .width(Length::Fill)
                                    .style(|theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(Color::from_rgb(0.7, 0.7, 0.7))),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(Color::TRANSPARENT)),
                                                ..base
                                            },
                                        }
                                    }),
                                button(text("Duplicate"))
                                    .on_press(Message::DuplicateRequest(collection_index, request_index))
                                    .width(Length::Fill)
                                    .style(|theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(Color::from_rgb(0.7, 0.7, 0.7))),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(Color::TRANSPARENT)),
                                                ..base
                                            },
                                        }
                                    }),
                                button(text("Delete"))
                                    .on_press(Message::DeleteRequest(collection_index, request_index))
                                    .width(Length::Fill)
                                    .style(|theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(Color::from_rgb(0.7, 0.7, 0.7))),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(Color::TRANSPARENT)),
                                                ..base
                                            },
                                        }
                                    }),
                            ]
                            .spacing(2)
                        )
                        .width(Length::Fixed(150.0))
                        .style(|theme| Style {
                            background: Some(Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                            border: Border {
                                color: Color::from_rgb(0.8, 0.8, 0.8),
                                width: 1.0,
                                radius: 4.0.into(),
                            },
                            shadow: Shadow {
                                color: Color::from_rgba(0.0, 0.0, 0.0, 0.1),
                                offset: Vector::new(2.0, 2.0),
                                blur_radius: 4.0,
                            },
                            ..Style::default()
                        })
                        .padding(4)
                        .into()
                    }
                );

                content = content.push(request_with_context_menu);
            }
        }
    }

    scrollable(content.spacing(2).padding(10))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn method_badge<'a>(method: &'a HttpMethod) -> Element<'a, Message> {
    let (color, text_color) = match method {
        HttpMethod::GET => (Color::from_rgb(0.0, 0.8, 0.0), Color::WHITE),
        HttpMethod::POST => (Color::from_rgb(1.0, 0.6, 0.0), Color::WHITE),
        HttpMethod::PUT => (Color::from_rgb(0.0, 0.4, 0.8), Color::WHITE),
        HttpMethod::DELETE => (Color::from_rgb(0.8, 0.0, 0.0), Color::WHITE),
        HttpMethod::PATCH => (Color::from_rgb(0.6, 0.0, 0.8), Color::WHITE),
        HttpMethod::HEAD => (Color::from_rgb(0.5, 0.5, 0.5), Color::WHITE),
        HttpMethod::OPTIONS => (Color::from_rgb(0.3, 0.3, 0.3), Color::WHITE),
    };

    // Truncate method name to maximum 4 characters
    let method_text = match method {
        HttpMethod::DELETE => "DELE".to_string(),
        HttpMethod::OPTIONS => "OPTN".to_string(),
        HttpMethod::PATCH => "PACH".to_string(),
        _ => {
            let method_str = method.to_string();
            if method_str.len() > 4 {
                method_str[..4].to_string()
            } else {
                method_str
            }
        }
    };

    container(
        text(method_text)
            .size(10)
            .color(text_color)
    )
    .width(Length::Fixed(32.0))
    .align_x(iced::alignment::Horizontal::Right)
    .style(move |theme| Style {
        background: Some(Background::Color(color)),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..Style::default()
    })
    .padding([2, 3])
    .into()
}