use crate::types::{ResponseData, ResponseTab, ResponseHighlighter, Message};
use iced::widget::{
    button, column, container, row, text, scrollable, text_editor, Space
};
use iced::{Element, Length, Color, Background, Border};
use iced::widget::container::Style;
use iced::widget::button::Status;

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

pub fn response_panel<'a>(
    response: &'a Option<ResponseData>,
    response_body_content: &'a text_editor::Content,
    selected_tab: ResponseTab,
    is_loading: bool,
    current_elapsed_time: u64,
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

            let mut status_row = vec![];

            // Add loading indicator if loading (on the left)
            if is_loading {
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
                status_row.push(Space::with_width(10).into());
            }

            // Add response status
            status_row.push(
                container(
                    text(format!("{} {}", resp.status, resp.status_text))
                        .color(status_color)
                        .size(14)
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
            );

            status_row.push(Space::with_width(20).into());
            let time_text = if is_loading {
                format!("Time: {}ms", current_elapsed_time)
            } else {
                format!("Time: {}ms", resp.time)
            };
            status_row.push(
                text(time_text)
                    .size(12)
                    .color(Color::from_rgb(0.5, 0.5, 0.5))
                    .into(),
            );
            status_row.push(Space::with_width(20).into());
            status_row.push(
                text(format!("Size: {}", format_bytes(resp.size)))
                    .size(12)
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
                ResponseTab::Body => response_body_tab(response_body_content, response),
                ResponseTab::Headers => response_headers_tab(resp),
            };

            column![
                status_info,
                Space::with_height(10),
                tabs,
                Space::with_height(5),
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
                        Space::with_width(20),
                        text(format!("Time: {}ms", current_elapsed_time))
                            .size(12)
                            .color(Color::from_rgb(0.5, 0.5, 0.5)),
                    ]
                    .align_y(iced::Alignment::Center),
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

fn response_body_tab<'a>(content: &'a text_editor::Content, response: &'a Option<ResponseData>) -> Element<'a, Message> {
    let mut body_column = column![];
    
    // Check if this is a binary response
    if let Some(resp) = response {
        if resp.is_binary {
            // For binary responses, show metadata instead of content
            let binary_info = column![
                container(
                    text("Binary Response")
                        .size(16)
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
                .padding([8, 12]),
                
                Space::with_height(10),
                
                text(format!("Content-Type: {}", resp.content_type))
                    .size(14)
                    .color(Color::from_rgb(0.3, 0.3, 0.3)),
                    
                text(format!("Size: {}", format_bytes(resp.size)))
                    .size(14)
                    .color(Color::from_rgb(0.3, 0.3, 0.3)),
                    
                Space::with_height(15),
                
                text("Preview (first 100 bytes as hex):")
                    .size(14)
                    .color(Color::from_rgb(0.3, 0.3, 0.3)),
                    
                Space::with_height(5),
                
                scrollable(
                    container(
                        text(&resp.body)
                            .size(12)
                            .color(Color::from_rgb(0.5, 0.5, 0.5))
                    )
                    .style(|theme| Style {
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
            
            body_column = body_column.push(binary_info);
        } else {
            // For text responses, use the normal text editor
            body_column = body_column
                .push(
                    text_editor(content)
                        .on_action(Message::ResponseBodyAction)
                        .height(Length::Fill)
                );
        }
    } else {
        // No response yet, show empty text editor
        body_column = body_column
            .push(
                text_editor(content)
                    .on_action(Message::ResponseBodyAction)
                    .height(Length::Fill)
            );
    }
    
    body_column.spacing(10).into()
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