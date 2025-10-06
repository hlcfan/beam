use crate::types::{RequestConfig, RequestTab, HttpMethod, AuthType, Message};
use iced::widget::{
    button, column, container, row, text, text_input, pick_list, scrollable,
    text_editor, Space
};
use iced::{Element, Fill, Length, Color, Background, Border};
use iced::widget::container::Style;
use iced::widget::button::Status;

pub fn request_panel<'a>(
    config: &'a RequestConfig,
    is_loading: bool,
) -> Element<'a, Message> {
    let url_row = row![
        pick_list(
            vec![
                HttpMethod::GET,
                HttpMethod::POST,
                HttpMethod::PUT,
                HttpMethod::DELETE,
                HttpMethod::PATCH,
                HttpMethod::HEAD,
                HttpMethod::OPTIONS,
            ],
            Some(config.method.clone()),
            Message::MethodChanged
        )
        .width(100),
        Space::with_width(10),
        text_input("Enter URL", &config.url)
            .on_input(Message::UrlChanged)
            .width(Length::Fill),
        Space::with_width(10),
        if is_loading {
            button(text("Cancel"))
                .on_press(Message::CancelRequest)
                .style(move |theme, status| {
                    let base = button::Style::default();
                    match status {
                        Status::Hovered => button::Style {
                            background: Some(Background::Color(Color::from_rgb(0.9, 0.2, 0.2))),
                            text_color: Color::WHITE,
                            ..base
                        },
                        _ => button::Style {
                            background: Some(Background::Color(Color::from_rgb(0.8, 0.0, 0.0))),
                            text_color: Color::WHITE,
                            ..base
                        },
                    }
                })
        } else {
            let url_valid = !config.url.trim().is_empty() &&
                           (config.url.starts_with("http://") || config.url.starts_with("https://"));

            let send_button = button(text("Send"));

            if url_valid {
                send_button
                    .on_press(Message::SendRequest)
                    .style(move |theme, status| {
                        let base = button::Style::default();
                        match status {
                            Status::Hovered => button::Style {
                                background: Some(Background::Color(Color::from_rgb(0.2, 0.7, 0.2))),
                                text_color: Color::WHITE,
                                ..base
                            },
                            _ => button::Style {
                                background: Some(Background::Color(Color::from_rgb(0.0, 0.6, 0.0))),
                                text_color: Color::WHITE,
                                ..base
                            },
                        }
                    })
            } else {
                send_button
                    .style(move |theme, status| {
                        let base = button::Style::default();
                        button::Style {
                            background: Some(Background::Color(Color::from_rgb(0.6, 0.6, 0.6))),
                            text_color: Color::from_rgb(0.8, 0.8, 0.8),
                            ..base
                        }
                    })
            }
        }
    ]
    .align_y(iced::Alignment::Center);

    let tabs = row![
        tab_button("Body", config.selected_tab == RequestTab::Body, RequestTab::Body),
        tab_button("Params", config.selected_tab == RequestTab::Params, RequestTab::Params),
        tab_button("Headers", config.selected_tab == RequestTab::Headers, RequestTab::Headers),
        tab_button("Auth", config.selected_tab == RequestTab::Auth, RequestTab::Auth),
    ]
    .spacing(5);

    let tab_content = match config.selected_tab {
        RequestTab::Body => body_tab(config),
        RequestTab::Params => params_tab(config),
        RequestTab::Headers => headers_tab(config),
        RequestTab::Auth => auth_tab(config),
    };

    let content = column![
        url_row,
        Space::with_height(20),
        tabs,
        Space::with_height(10),
        tab_content
    ]
    .spacing(10)
    .padding(20);

    // Create left border
    let left_border = container("")
                                 .width(Length::Fixed(1.0))
                                 .height(Fill)
                                 .style(|_theme| container::Style {
                                     background: Some(iced::Background::Color(iced::Color::from_rgb(0.7, 0.7, 0.7))), // Gray
                                     ..Default::default()
                                 });

    // Create right border
    let right_border = container("")
                                 .width(Length::Fixed(1.0))
                                 .height(Fill)
                                 .style(|_theme| container::Style {
                                     background: Some(iced::Background::Color(iced::Color::from_rgb(0.7, 0.7, 0.7))), // Gray
                                     ..Default::default()
                                 });
    row![
        left_border,
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(0),
        right_border
    ]
    .into()
}

fn tab_button<'a>(label: &'a str, is_active: bool, tab: RequestTab) -> Element<'a, Message> {
    button(text(label))
        .on_press(Message::TabSelected(tab))
        .style(move |theme, status| {
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

fn body_tab<'a>(config: &'a RequestConfig) -> Element<'a, Message> {
    column![
        text("Request Body"),
        text_editor(&config.body)
            .on_action(Message::BodyChanged)
            .height(Length::Fill)
    ]
    .spacing(10)
    .into()
}

fn params_tab<'a>(config: &'a RequestConfig) -> Element<'a, Message> {
    let mut content = column![
        row![
            text("Key").width(Length::FillPortion(1)),
            text("Value").width(Length::FillPortion(1)),
            text("").width(50) // For delete button
        ]
        .spacing(10)
    ];

    for (index, (key, value)) in config.params.iter().enumerate() {
        let param_row = row![
            text_input("Key", key)
                .on_input(move |input| Message::ParamKeyChanged(index, input))
                .width(Length::FillPortion(1)),
            text_input("Value", value)
                .on_input(move |input| Message::ParamValueChanged(index, input))
                .width(Length::FillPortion(1)),
            button(text("×"))
                .on_press(Message::RemoveParam(index))
                .width(50)
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        content = content.push(param_row);
    }

    content = content.push(
        button(text("Add Parameter"))
            .on_press(Message::AddParam)
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
    );

    scrollable(content.spacing(10))
        .height(Length::Fill)
        .into()
}

fn headers_tab<'a>(config: &'a RequestConfig) -> Element<'a, Message> {
    let mut content = column![
        row![
            text("Key").width(Length::FillPortion(1)),
            text("Value").width(Length::FillPortion(1)),
            text("").width(50) // For delete button
        ]
        .spacing(10)
    ];

    for (index, (key, value)) in config.headers.iter().enumerate() {
        let header_row = row![
            text_input("Header name", key)
                .on_input(move |input| Message::HeaderKeyChanged(index, input))
                .width(Length::FillPortion(1)),
            text_input("Header value", value)
                .on_input(move |input| Message::HeaderValueChanged(index, input))
                .width(Length::FillPortion(1)),
            button(text("×"))
                .on_press(Message::RemoveHeader(index))
                .width(50)
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        content = content.push(header_row);
    }

    content = content.push(
        button(text("Add Header"))
            .on_press(Message::AddHeader)
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
    );

    scrollable(content.spacing(10))
        .height(Length::Fill)
        .into()
}

fn auth_tab<'a>(config: &'a RequestConfig) -> Element<'a, Message> {
    column![
        text("Authentication"),
        pick_list(
            vec![AuthType::None, AuthType::Bearer, AuthType::Basic, AuthType::ApiKey],
            Some(config.auth_type.clone()),
            Message::AuthTypeChanged
        ),
        text("Authentication configuration will be implemented here")
    ]
    .spacing(10)
    .into()
}