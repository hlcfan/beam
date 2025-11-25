use crate::types::Environment;
use crate::ui::{IconName, icon};
use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Color, Element, Fill, Length, Padding, Theme, Vector};

#[derive(Debug, Clone)]
pub enum Action {
    AddEnvironment,
    DeleteEnvironment(usize),
    EnvironmentNameChanged(usize, String),
    EnvironmentDescriptionChanged(usize, String),
    VariableKeyChanged(usize, String, String), // (env_index, old_key, new_key)
    VariableValueChanged(usize, String, String), // (env_index, key, new_value)
    AddVariable(usize),
    RemoveVariable(usize, String), // (env_index, key)
    ToggleVariable(usize, String), // (env_index, key)
    ClosePopup,
    EnvironmentSelected(usize),
    None,
}

#[derive(Debug, Clone)]
pub enum Message {
    AddEnvironment,
    DeleteEnvironment(usize),
    EnvironmentNameChanged(usize, String),
    EnvironmentDescriptionChanged(usize, String),
    VariableKeyChanged(usize, String, String),
    VariableValueChanged(usize, String, String),
    AddVariable(usize),
    RemoveVariable(usize, String),
    ToggleVariable(usize, String),
    ClosePopup,
    EnvironmentSelected(usize),
}

#[derive(Debug, Clone)]
pub struct EnvironmentPanel {
    pub show_popup: bool,
}

impl EnvironmentPanel {
    pub fn new() -> Self {
        Self { show_popup: false }
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::AddEnvironment => Action::AddEnvironment,
            Message::DeleteEnvironment(index) => Action::DeleteEnvironment(index),
            Message::EnvironmentNameChanged(env_index, name) => {
                Action::EnvironmentNameChanged(env_index, name)
            }
            Message::EnvironmentDescriptionChanged(env_index, description) => {
                Action::EnvironmentDescriptionChanged(env_index, description)
            }
            Message::VariableKeyChanged(env_index, old_key, new_key) => {
                Action::VariableKeyChanged(env_index, old_key, new_key)
            }
            Message::VariableValueChanged(env_index, key, value) => {
                Action::VariableValueChanged(env_index, key, value)
            }
            Message::AddVariable(env_index) => Action::AddVariable(env_index),
            Message::RemoveVariable(env_index, key) => Action::RemoveVariable(env_index, key),
            Message::ToggleVariable(env_index, key) => Action::ToggleVariable(env_index, key),
            Message::ClosePopup => {
                self.show_popup = false;
                Action::ClosePopup
            }
            Message::EnvironmentSelected(index) => Action::EnvironmentSelected(index),
        }
    }

    pub fn view<'a>(
        &'a self,
        environments: &'a [Environment],
        active_environment: Option<usize>,
    ) -> Element<'a, Message> {
        // Header with title and close button
        let close_button = button(
            container(
                icon(IconName::Close)
                    .size(20)
                    .color(Color::from_rgb(0.5, 0.5, 0.5)),
            )
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center)
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .padding(Padding::from(6.0))
        .on_press(Message::ClosePopup)
        .width(32)
        .height(32)
        .style(|_theme: &Theme, status| {
            let base = button::Style::default();
            match status {
                button::Status::Hovered | button::Status::Pressed => button::Style {
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
        });

        let header = row![
            text("Environments")
                .size(16)
                .color(Color::from_rgb(0.3, 0.3, 0.3)),
            space().width(Fill),
            close_button
        ]
        .align_y(iced::Alignment::Center);

        // New Environment button (fixed at top)
        let new_env_button = button(
            row![
                icon(IconName::Add).size(14).color(Color::WHITE),
                space().width(8),
                text("New Environment").size(14)
            ]
            .align_y(iced::Alignment::Center),
        )
        .on_press(Message::AddEnvironment)
        .width(Fill)
        .padding([10, 16])
        .style(|_theme, status| match status {
            button::Status::Hovered => button::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.1, 0.1))),
                text_color: Color::WHITE,
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..button::Style::default()
            },
            _ => button::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.2, 0.2))),
                text_color: Color::WHITE,
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..button::Style::default()
            },
        });

        // Environment list (scrollable)
        let mut env_list = column![].spacing(8);

        for (idx, env) in environments.iter().enumerate() {
            let is_active = active_environment == Some(idx);
            let var_count = env.variables.len();

            let env_item = button(
                column![
                    row![
                        text(&env.name).size(14).color(if is_active {
                            Color::from_rgb(0.1, 0.1, 0.1)
                        } else {
                            Color::from_rgb(0.3, 0.3, 0.3)
                        }),
                        space().width(Fill),
                        if is_active {
                            container(text("Active").size(10).color(Color::WHITE))
                                .padding([2, 6])
                                .style(|_theme: &Theme| container::Style {
                                    background: Some(iced::Background::Color(Color::from_rgb(
                                        0.1, 0.1, 0.1,
                                    ))),
                                    border: iced::Border {
                                        radius: 4.0.into(),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })
                        } else {
                            container(text(""))
                        }
                    ]
                    .align_y(iced::Alignment::Center),
                    space().height(2),
                    text(format!("{} variables", var_count))
                        .size(12)
                        .color(Color::from_rgb(0.6, 0.6, 0.6))
                ]
                .spacing(0),
            )
            .on_press(Message::EnvironmentSelected(idx))
            .width(Fill)
            .padding([10, 12])
            .style(move |_theme, status| match status {
                button::Status::Hovered => button::Style {
                    background: Some(iced::Background::Color(if is_active {
                        Color::from_rgb(0.92, 0.92, 0.92)
                    } else {
                        Color::from_rgb(0.97, 0.97, 0.97)
                    })),
                    border: iced::Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..button::Style::default()
                },
                _ => button::Style {
                    background: Some(iced::Background::Color(if is_active {
                        Color::from_rgb(0.95, 0.95, 0.95)
                    } else {
                        Color::TRANSPARENT
                    })),
                    border: iced::Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..button::Style::default()
                },
            });

            env_list = env_list.push(env_item);
        }

        let sidebar_container = container(
            column![
                space().height(10),
                new_env_button,
                space().height(10),
                scrollable(env_list)
                    .height(Fill)
                    .direction(scrollable::Direction::Vertical(
                        scrollable::Scrollbar::new().width(0).scroller_width(0),
                    ))
            ]
            .spacing(0),
        )
        .width(240)
        .height(Fill)
        .padding(12)
        .style(|_theme: &Theme| container::Style {
            border: iced::Border {
                color: Color::from_rgb(0.9, 0.9, 0.9),
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        });

        // RIGHT PANEL - Variables display
        let right_panel = if let Some(active_idx) = active_environment {
            if let Some(active_env) = environments.get(active_idx) {
                let mut panel_content = column![].spacing(12);

                // Environment name input with Active badge
                let env_header = row![
                    text_input("", &active_env.name)
                        .on_input(move |input| {
                            Message::EnvironmentNameChanged(active_idx, input)
                        })
                        .padding(8)
                        .size(18)
                        .width(Length::FillPortion(3))
                        .style(|_theme, status| {
                            let (border_color, border_width) = match status {
                                text_input::Status::Focused { .. } => {
                                    (Color::from_rgb(0.7, 0.7, 0.7), 1.0)
                                }
                                _ => (Color::from_rgb(0.9, 0.9, 0.9), 1.0),
                            };
                            text_input::Style {
                                background: iced::Background::Color(Color::TRANSPARENT),
                                border: iced::Border {
                                    color: border_color,
                                    width: border_width,
                                    radius: 4.0.into(),
                                },
                                icon: Color::from_rgb(0.5, 0.5, 0.5),
                                placeholder: Color::from_rgb(0.7, 0.7, 0.7),
                                value: Color::from_rgb(0.1, 0.1, 0.1),
                                selection: Color::from_rgb(0.7, 0.85, 1.0),
                            }
                        }),
                    space().width(10),
                    container(text("Active").size(12).color(Color::WHITE))
                        .padding([4, 10])
                        .style(|_theme: &Theme| container::Style {
                            background: Some(iced::Background::Color(Color::from_rgb(
                                0.1, 0.1, 0.1
                            ))),
                            border: iced::Border {
                                radius: 12.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    space().width(Fill)
                ]
                .align_y(iced::Alignment::Center);

                panel_content = panel_content.push(env_header);

                // Description
                panel_content = panel_content.push(
                    text("Define variables that can be used across your requests")
                        .size(13)
                        .color(Color::from_rgb(0.5, 0.5, 0.5)),
                );

                panel_content = panel_content.push(space().height(10));

                // Variables section header
                let enabled_count = active_env.variables.values().filter(|v| v.enabled).count();
                let variables_header = row![
                    text("Variables").size(14),
                    space().width(10),
                    container(
                        text(format!("{} enabled", enabled_count))
                            .size(12)
                            .color(Color::from_rgb(0.5, 0.5, 0.5))
                    )
                    .padding([2, 8])
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb(
                            0.95, 0.95, 0.95
                        ))),
                        border: iced::Border {
                            radius: 10.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    space().width(Fill),
                    text("Show Values")
                        .size(13)
                        .color(Color::from_rgb(0.5, 0.5, 0.5))
                ]
                .align_y(iced::Alignment::Center);

                panel_content = panel_content.push(variables_header);
                panel_content = panel_content.push(space().height(8));

                // Variables table - create a column to hold header and rows
                let mut table_content = column![].spacing(0);

                // Variables table header
                let table_header = container(
                    row![
                        container(text("").width(40)), // Toggle button column
                        container(
                            text("Key")
                                .size(12)
                                .color(Color::from_rgb(0.3, 0.3, 0.3))
                                .font(iced::Font {
                                    weight: iced::font::Weight::Bold,
                                    ..Default::default()
                                })
                        )
                        .width(Length::FillPortion(1))
                        .padding([8, 8]),
                        container(
                            text("Value")
                                .size(12)
                                .color(Color::from_rgb(0.3, 0.3, 0.3))
                                .font(iced::Font {
                                    weight: iced::font::Weight::Bold,
                                    ..Default::default()
                                })
                        )
                        .width(Length::FillPortion(1))
                        .padding([8, 8]),
                        container(text("").width(40)) // Delete button column
                    ]
                    .spacing(10),
                )
                .padding([8, 8])
                .style(|_theme: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::TRANSPARENT)),
                    border: iced::Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                });

                table_content = table_content.push(table_header);

                // Add separator after header
                if !active_env.variables.is_empty() {
                    table_content =
                        table_content.push(container(space()).width(Length::Fill).height(1).style(
                            |_theme: &Theme| container::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(
                                    0.9, 0.9, 0.9,
                                ))),
                                ..Default::default()
                            },
                        ));
                }

                // Variables rows
                for (i, (key, var)) in active_env.variables.iter().enumerate() {
                    if i > 0 {
                        // Add separator between rows
                        table_content = table_content.push(
                            container(space()).width(Length::Fill).height(1).style(
                                |_theme: &Theme| container::Style {
                                    background: Some(iced::Background::Color(Color::from_rgb(
                                        0.9, 0.9, 0.9,
                                    ))),
                                    ..Default::default()
                                },
                            ),
                        );
                    }
                    let key_clone = key.clone();
                    let key_clone2 = key.clone();
                    let key_clone3 = key.clone();
                    let key_clone4 = key.clone();
                    let is_enabled = var.enabled;

                    // Toggle button
                    let toggle_button = button(
                        container(text(if is_enabled { "✓" } else { "○" }).size(16).color(
                            if is_enabled {
                                Color::from_rgb(0.2, 0.7, 0.3)
                            } else {
                                Color::from_rgb(0.7, 0.7, 0.7)
                            },
                        ))
                        .align_x(iced::alignment::Horizontal::Center)
                        .align_y(iced::alignment::Vertical::Center)
                        .width(Length::Fill)
                        .height(Length::Fill),
                    )
                    .on_press(Message::ToggleVariable(active_idx, key_clone4))
                    .width(32)
                    .height(32)
                    .style(|_theme: &Theme, status| {
                        let base = button::Style::default();
                        match status {
                            button::Status::Hovered | button::Status::Pressed => button::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(
                                    0.95, 0.95, 0.95,
                                ))),
                                border: iced::Border {
                                    radius: 4.0.into(),
                                    ..Default::default()
                                },
                                ..base
                            },
                            _ => button::Style {
                                background: Some(iced::Background::Color(Color::TRANSPARENT)),
                                ..base
                            },
                        }
                    });

                    let delete_button = button(
                        container(
                            icon(IconName::Close)
                                .size(14)
                                .color(Color::from_rgb(0.8, 0.3, 0.3)),
                        )
                        .align_x(iced::alignment::Horizontal::Center)
                        .align_y(iced::alignment::Vertical::Center)
                        .width(Length::Fill)
                        .height(Length::Fill),
                    )
                    .on_press(Message::RemoveVariable(active_idx, key_clone3))
                    .width(32)
                    .height(32)
                    .style(|_theme: &Theme, status| {
                        let base = button::Style::default();
                        match status {
                            button::Status::Hovered | button::Status::Pressed => button::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(
                                    0.98, 0.95, 0.95,
                                ))),
                                border: iced::Border {
                                    radius: 4.0.into(),
                                    ..Default::default()
                                },
                                ..base
                            },
                            _ => button::Style {
                                background: Some(iced::Background::Color(Color::TRANSPARENT)),
                                ..base
                            },
                        }
                    });

                    // Apply visual styling based on enabled state
                    let text_color = if is_enabled {
                        Color::from_rgb(0.1, 0.1, 0.1)
                    } else {
                        Color::from_rgb(0.6, 0.6, 0.6)
                    };

                    let variable_row = container(
                        row![
                            container(toggle_button)
                                .width(40)
                                .align_x(iced::alignment::Horizontal::Center),
                            text_input("", key)
                                .on_input(move |input| Message::VariableKeyChanged(
                                    active_idx,
                                    key_clone.clone(),
                                    input
                                ))
                                .padding(8)
                                .size(13)
                                .width(Length::FillPortion(1))
                                .style(move |_theme, status| {
                                    let (border_color, border_width) = match status {
                                        text_input::Status::Focused { .. } => {
                                            (Color::from_rgb(0.7, 0.7, 0.7), 1.0)
                                        }
                                        _ => (Color::from_rgb(0.9, 0.9, 0.9), 1.0),
                                    };
                                    text_input::Style {
                                        background: iced::Background::Color(Color::TRANSPARENT),
                                        border: iced::Border {
                                            color: border_color,
                                            width: border_width,
                                            radius: 4.0.into(),
                                        },
                                        icon: Color::from_rgb(0.5, 0.5, 0.5),
                                        placeholder: Color::from_rgb(0.7, 0.7, 0.7),
                                        value: text_color,
                                        selection: Color::from_rgb(0.7, 0.85, 1.0),
                                    }
                                }),
                            text_input("", &var.value)
                                .on_input(move |input| Message::VariableValueChanged(
                                    active_idx,
                                    key_clone2.clone(),
                                    input
                                ))
                                .padding(8)
                                .size(13)
                                .width(Length::FillPortion(1))
                                .style(move |_theme, status| {
                                    let (border_color, border_width) = match status {
                                        text_input::Status::Focused { .. } => {
                                            (Color::from_rgb(0.7, 0.7, 0.7), 1.0)
                                        }
                                        _ => (Color::from_rgb(0.9, 0.9, 0.9), 1.0),
                                    };
                                    text_input::Style {
                                        background: iced::Background::Color(Color::TRANSPARENT),
                                        border: iced::Border {
                                            color: border_color,
                                            width: border_width,
                                            radius: 4.0.into(),
                                        },
                                        icon: Color::from_rgb(0.5, 0.5, 0.5),
                                        placeholder: Color::from_rgb(0.7, 0.7, 0.7),
                                        value: text_color,
                                        selection: Color::from_rgb(0.7, 0.85, 1.0),
                                    }
                                }),
                            container(delete_button)
                                .width(40)
                                .align_x(iced::alignment::Horizontal::Center)
                        ]
                        .spacing(10)
                        .align_y(iced::Alignment::Center),
                    )
                    .padding([8, 8])
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::TRANSPARENT)),
                        border: iced::Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: 0.0.into(),
                        },
                        ..Default::default()
                    });

                    table_content = table_content.push(variable_row);
                }

                // Wrap the table in a container with rounded border
                let table_container =
                    container(table_content).style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::WHITE)),
                        border: iced::Border {
                            color: Color::from_rgb(0.9, 0.9, 0.9),
                            width: 1.0,
                            radius: 8.0.into(),
                        },
                        ..Default::default()
                    });

                panel_content = panel_content.push(table_container);

                // Add Variable button
                panel_content = panel_content.push(space().height(8));
                panel_content = panel_content.push(
                    button(
                        row![
                            icon(IconName::Add).size(14),
                            space().width(6),
                            text("Add Variable").size(13)
                        ]
                        .align_y(iced::Alignment::Center),
                    )
                    .on_press(Message::AddVariable(active_idx))
                    .padding([8, 12])
                    .style(|_theme, status| match status {
                        button::Status::Hovered => button::Style {
                            background: Some(iced::Background::Color(Color::from_rgb(
                                0.96, 0.96, 0.96,
                            ))),
                            text_color: Color::from_rgb(0.3, 0.3, 0.3),
                            border: iced::Border {
                                color: Color::from_rgb(0.85, 0.85, 0.85),
                                width: 1.0,
                                radius: 6.0.into(),
                            },
                            ..button::Style::default()
                        },
                        _ => button::Style {
                            background: Some(iced::Background::Color(Color::WHITE)),
                            text_color: Color::from_rgb(0.3, 0.3, 0.3),
                            border: iced::Border {
                                color: Color::from_rgb(0.9, 0.9, 0.9),
                                width: 1.0,
                                radius: 6.0.into(),
                            },
                            ..button::Style::default()
                        },
                    }),
                );

                // Using Variables section
                panel_content = panel_content.push(space().height(20));
                panel_content = panel_content.push(
                    container(
                        column![
                            text("Using Variables").size(14),
                            space().height(6),
                            text("Reference variables in your requests using the syntax: {{variable_name}}")
                                .size(12)
                                .color(Color::from_rgb(0.5, 0.5, 0.5)),
                            space().height(4),
                            text("Example: {{base_url}}/api/users")
                                .size(12)
                                .color(Color::from_rgb(0.5, 0.5, 0.5))
                        ]
                        .spacing(0)
                    )
                    .width(Length::Fill)
                    .padding(12)
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb(0.97, 0.97, 0.98))),
                        border: iced::Border {
                            color: Color::from_rgb(0.9, 0.9, 0.92),
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    })
                );

                // Environment management section
                panel_content = panel_content.push(space().height(20));
                panel_content = panel_content.push(
                    container(
                        column![
                            text("Environment Settings").size(14),
                            space().height(10),
                            text("Description")
                                .size(12)
                                .color(Color::from_rgb(0.5, 0.5, 0.5)),
                            space().height(4),
                            text_input(
                                "Environment description",
                                active_env.description.as_deref().unwrap_or(""),
                            )
                            .on_input(move |input| {
                                Message::EnvironmentDescriptionChanged(active_idx, input)
                            })
                            .padding(8)
                            .size(13)
                            .style(|_theme, status| {
                                let (border_color, border_width) = match status {
                                    text_input::Status::Focused { .. } => {
                                        (Color::from_rgb(0.7, 0.7, 0.7), 1.0)
                                    }
                                    _ => (Color::from_rgb(0.9, 0.9, 0.9), 1.0),
                                };
                                text_input::Style {
                                    background: iced::Background::Color(Color::TRANSPARENT),
                                    border: iced::Border {
                                        color: border_color,
                                        width: border_width,
                                        radius: 4.0.into(),
                                    },
                                    icon: Color::from_rgb(0.5, 0.5, 0.5),
                                    placeholder: Color::from_rgb(0.7, 0.7, 0.7),
                                    value: Color::from_rgb(0.1, 0.1, 0.1),
                                    selection: Color::from_rgb(0.7, 0.85, 1.0),
                                }
                            }),
                        ]
                        .spacing(0),
                    )
                    .padding(12)
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb(
                            0.97, 0.97, 0.98,
                        ))),
                        border: iced::Border {
                            color: Color::from_rgb(0.92, 0.92, 0.92),
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    }),
                );

                if environments.len() > 1 {
                    panel_content = panel_content.push(space().height(15));
                    panel_content = panel_content.push(
                        button(
                            row![
                                icon(IconName::Close)
                                    .size(14)
                                    .color(Color::from_rgb(0.8, 0.3, 0.3)),
                                space().width(6),
                                text("Delete Environment").size(13)
                            ]
                            .align_y(iced::Alignment::Center),
                        )
                        .on_press(Message::DeleteEnvironment(active_idx))
                        .padding([8, 12])
                        .style(|_theme, status| match status {
                            button::Status::Hovered => button::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(
                                    0.95, 0.3, 0.3,
                                ))),
                                text_color: Color::WHITE,
                                border: iced::Border {
                                    radius: 6.0.into(),
                                    ..Default::default()
                                },
                                ..button::Style::default()
                            },
                            _ => button::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(
                                    0.98, 0.95, 0.95,
                                ))),
                                text_color: Color::from_rgb(0.8, 0.3, 0.3),
                                border: iced::Border {
                                    color: Color::from_rgb(0.95, 0.85, 0.85),
                                    width: 1.0,
                                    radius: 6.0.into(),
                                },
                                ..button::Style::default()
                            },
                        }),
                    );
                }

                scrollable(panel_content)
                    .height(Fill)
                    .direction(scrollable::Direction::Vertical(
                        scrollable::Scrollbar::new().width(0).scroller_width(0),
                    ))
            } else {
                scrollable(column![
                    text("No environment selected")
                        .size(14)
                        .color(Color::from_rgb(0.5, 0.5, 0.5))
                ])
                .height(Fill)
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::new().width(0).scroller_width(0),
                ))
            }
        } else {
            scrollable(column![
                text("No environment selected")
                    .size(14)
                    .color(Color::from_rgb(0.5, 0.5, 0.5)),
                space().height(10),
                text("Select an environment from the sidebar or create a new one")
                    .size(13)
                    .color(Color::from_rgb(0.6, 0.6, 0.6))
            ])
            .height(Fill)
            .direction(scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(0).scroller_width(0),
            ))
        };

        let right_panel_container = container(right_panel).width(Fill).height(Fill).padding(12);

        // Main layout: header + two-panel content
        let main_content = row![
            sidebar_container,
            container(column![text("")].width(1))
                .height(Fill)
                .style(|_theme: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                    ..Default::default()
                }),
            right_panel_container
        ]
        .spacing(0)
        .height(Fill);

        container(column![header, main_content].spacing(0).height(Fill))
            .width(Length::Fixed(1000.0))
            .height(Length::Fixed(650.0))
            .padding(20)
            .style(|_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::WHITE)),
                border: iced::Border {
                    color: Color::from_rgb(0.85, 0.85, 0.85),
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                    offset: Vector::new(0.0, 4.0),
                    blur_radius: 20.0,
                },
                ..Default::default()
            })
            .into()
    }
}
