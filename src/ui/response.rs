use crate::types::{ResponseData, ResponseTab, ResponseHighlighter, Message};
use iced::widget::{
    button, column, container, row, text, scrollable, text_editor, Space
};
use iced::{Element, Length, Color, Background, Border};
use iced::widget::container::Style;
use iced::widget::button::Status;

pub fn response_panel<'a>(
    response: &'a Option<ResponseData>,
    response_body_content: &'a text_editor::Content,
    selected_tab: ResponseTab,
    is_loading: bool,
) -> Element<'a, Message> {
    match response {
        Some(resp) => {
            let status_color = if resp.status >= 200 && resp.status < 300 {
                Color::from_rgb(0.0, 0.8, 0.0)
            } else if resp.status >= 400 {
                Color::from_rgb(0.8, 0.0, 0.0)
            } else {
                Color::from_rgb(1.0, 0.6, 0.0)
            };

            let mut status_row = vec![
                container(
                    text(format!("{} {}", resp.status, resp.status_text))
                        .color(status_color)
                        .size(16)
                )
                .style(move |theme| Style {
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
            ];

            // Add loading indicator if loading
            if is_loading {
                status_row.push(Space::with_width(10).into());
                status_row.push(
                    container(
                        text("Loading...")
                            .size(14)
                            .color(Color::from_rgb(0.0, 0.5, 1.0))
                    )
                    .style(|theme| Style {
                        background: Some(Background::Color(Color::from_rgba(0.0, 0.5, 1.0, 0.1))),
                        border: Border {
                            radius: 4.0.into(),
                            ..Border::default()
                        },
                        ..Style::default()
                    })
                    .padding([4, 8])
                    .into(),
                );
            }

            status_row.push(Space::with_width(20).into());
            status_row.push(
                text(format!("Time: {}ms", resp.time))
                    .size(14)
                    .color(Color::from_rgb(0.5, 0.5, 0.5))
                    .into(),
            );
            status_row.push(Space::with_width(20).into());
            status_row.push(
                text(format!("Size: {} bytes", resp.size))
                    .size(14)
                    .color(Color::from_rgb(0.5, 0.5, 0.5))
                    .into(),
            );

            let status_info = row(status_row)
                .align_y(iced::Alignment::Center);

            let tabs = row![
                response_tab_button("Body", selected_tab == ResponseTab::Body, ResponseTab::Body),
                response_tab_button("Headers", selected_tab == ResponseTab::Headers, ResponseTab::Headers),
            ]
            .spacing(5);

            let tab_content = match selected_tab {
                ResponseTab::Body => response_body_tab(response_body_content),
                ResponseTab::Headers => response_headers_tab(resp),
            };

            column![
                status_info,
                Space::with_height(20),
                tabs,
                Space::with_height(10),
                tab_content
            ]
            .spacing(10)
            .padding(20)
            .into()
        }
        None => {
            if is_loading {
                // Show loading status when no response exists yet
                column![
                    Space::with_height(20),
                    row![
                        container(
                            text("Loading...")
                                .size(14)
                                .color(Color::from_rgb(0.0, 0.5, 1.0))
                        )
                        .style(|theme| Style {
                            background: Some(Background::Color(Color::from_rgba(0.0, 0.5, 1.0, 0.1))),
                            border: Border {
                                radius: 4.0.into(),
                                ..Border::default()
                            },
                            ..Style::default()
                        })
                        .padding([4, 8]),
                    ]
                    .align_y(iced::Alignment::Center),
                    Space::with_height(20),
                ]
                .spacing(10)
                .padding(20)
                .into()
            } else {
                container(
                    column![
                        Space::with_height(100),
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
                        Space::with_height(100),
                    ]
                )
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            }
        }
    }
}

fn response_tab_button<'a>(label: &'a str, is_active: bool, tab: ResponseTab) -> Element<'a, Message> {
    button(text(label))
        .on_press(Message::ResponseTabSelected(tab))
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

fn response_body_tab<'a>(content: &'a text_editor::Content) -> Element<'a, Message> {
    column![
        text("Response Body"),
        text_editor(content)
            .on_action(Message::ResponseBodyAction)
            .height(Length::Fill)
    ]
    .spacing(10)
    .into()
}

fn response_headers_tab<'a>(response: &'a ResponseData) -> Element<'a, Message> {
    let mut content = column![
        text("Response Headers").size(16),
        Space::with_height(10)
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