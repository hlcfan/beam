use crate::types::{RequestConfig, RequestTab, HttpMethod, AuthType, Message, Environment};
use crate::ui::{icon, IconName};
use iced::widget::{
    button, column, container, pick_list, row, text, text_input, scrollable,
    text_editor, space, mouse_area, stack, Space
};
use iced::{Element, Fill, Length, Color, Background, Border, Theme, Shadow, Vector};
use iced::widget::button::Status;

// Helper function for send/cancel button styling
fn icon_button_style(is_interactive: bool) -> impl Fn(&Theme, Status) -> button::Style {
    move |_theme, status| {
        let base = button::Style::default();
        match status {
            Status::Hovered if is_interactive => button::Style {
                background: Some(Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 4.0.into(),
                },
                ..base
            },
            _ => button::Style {
                background: Some(Background::Color(Color::TRANSPARENT)),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 4.0.into(),
                },
                ..base
            },
        }
    }
}


pub fn request_panel<'a>(
    config: &'a RequestConfig,
    is_loading: bool,
    environments: &'a [Environment],
    active_environment: Option<usize>,
    method_menu_open: bool,
    show_url_tooltip: bool,
    tooltip_variable_name: &'a str,
    tooltip_variable_value: &'a str,
    tooltip_position: (f32, f32),
) -> Element<'a, Message> {
    // Environment pick_list for the URL row
    let env_pick_list = {
        // Create list of environment options including all environments plus "Configure"
        let mut env_options: Vec<String> = environments.iter().map(|env| env.name.clone()).collect();
        env_options.push("Configure".to_string());

        // Determine the selected value
        let selected_env = if let Some(active_idx) = active_environment {
            if let Some(env) = environments.get(active_idx) {
                Some(env.name.clone())
            } else {
                None
            }
        } else {
            None
        };

        pick_list(
            env_options,
            selected_env,
            |selected| {
                if selected == "Configure" {
                    Message::OpenEnvironmentPopup
                } else {
                    // Find the index of the selected environment
                    if let Some(index) = environments.iter().position(|env| env.name == selected) {
                        Message::EnvironmentSelected(index)
                    } else {
                        Message::DoNothing
                    }
                }
            }
        )
        .width(150)
        .placeholder("🌍 No Environment")
    };

    // Environment bar
    let env_bar = row![
        text("Environment:").size(14),
        space().width(5),
        env_pick_list,
        space().width(Fill), // Push everything to the left
    ]
    .align_y(iced::Alignment::Center);

    // Method label with dynamic width
    let method_label = method_button(&config.method);

    // Connected method label and URL input with overlay dropdown
    let url_input_with_hover = mouse_area(
        text_input("Enter URL", &config.url)
            .on_input(Message::UrlChanged)
            .width(Length::Fill)
            .style(|theme: &Theme, _status: text_input::Status| {
                text_input::Style {
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    background: Background::Color(theme.palette().background),
                    icon: Color::TRANSPARENT,
                    placeholder: theme.palette().text,
                    value: theme.palette().text,
                    selection: theme.palette().primary,
                }
            })
    )
    .on_move(move |_point| {
        // Detect all environment variables in the URL
        use regex::Regex;
        let re = Regex::new(r"\{\{([^}]+)\}\}").unwrap();

        let mut variables = Vec::new();
        for captures in re.captures_iter(&config.url) {
            let variable_name = captures.get(1).unwrap().as_str().to_string();
            let variable_value = if let Some(active_idx) = active_environment {
                if let Some(env) = environments.get(active_idx) {
                    env.get_variable(&variable_name)
                        .map(|v| v.clone())
                        .unwrap_or_else(|| "undefined".to_string())
                } else {
                    "undefined".to_string()
                }
            } else {
                "undefined".to_string()
            };
            variables.push((variable_name, variable_value));
        }

        if !variables.is_empty() {
            // Show all variables in the tooltip, one per line
            let all_vars = variables.iter()
                .map(|(name, value)| format!("{}: {}", name, value))
                .collect::<Vec<_>>()
                .join("\n");

            Message::ShowUrlTooltip(
                "Variables".to_string(),
                all_vars,
                20.0, // Fixed left padding
                100.0, // Fixed position above URL input (environment bar + some spacing)
            )
        } else {
            Message::HideUrlTooltip
        }
    })
    .on_exit(Message::HideUrlTooltip);

    // Create send/cancel button based on loading state
    let url_valid = !config.url.trim().is_empty() &&
                   (config.url.starts_with("http://") ||
                    config.url.starts_with("https://") ||
                    config.url.contains("{{"));

    let send_button = if is_loading {
        // Show cancel icon when loading
        button(
            icon(IconName::Cancel)
                .size(16)
        )
        .padding(8)
        .on_press(Message::CancelRequest)
        .style(icon_button_style(true))
    } else {
        // Show send icon when not loading
        let send_btn = button(
            icon(IconName::Send)
                .size(16)
        )
        .padding(8);

        if url_valid {
            send_btn
                .on_press(Message::SendRequest)
                .style(icon_button_style(true))
        } else {
            send_btn
                .style(icon_button_style(false))
        }
    };

    let base_input = container(
        row![
            method_label,
            url_input_with_hover,
            space().width(5),
            send_button,
        ]
    )
    .padding(2)
    .style(|_theme| container::Style {
        background: Some(Background::Color(Color::WHITE)),
        border: Border {
            color: Color::from_rgb(0.8, 0.8, 0.8),
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: None,
        shadow: Default::default(),
        snap: true,
    });

    let connected_input = base_input;

    let url_row = row![connected_input]
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
        RequestTab::Environment => body_tab(config), // Fallback to body tab if somehow Environment is selected
    };

    let content = column![
        env_bar,
        space().height(5),
        url_row,
        space().height(10),
        tabs,
        space().height(5),
        tab_content
    ]
    .spacing(5)
    .padding(15);

    // Create left border
    let left_border = container("")
                                 .width(Length::Fixed(1.0))
                                 .height(Fill)
                                 .style(|_theme| container::Style {
                                     background: Some(iced::Background::Color(iced::Color::from_rgb(0.9, 0.9, 0.9))), // Gray
                                     ..Default::default()
                                 });

    // Create right border
    let right_border = container("")
                                 .width(Length::Fixed(1.0))
                                 .height(Fill)
                                 .style(|_theme| container::Style {
                                     background: Some(iced::Background::Color(iced::Color::from_rgb(0.9, 0.9, 0.9))), // Gray
                                     ..Default::default()
                                 });
    let main_content = row![
        left_border,
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(0),
        right_border
    ];

    let base_layout = if method_menu_open {
        stack![
            main_content,
            // Transparent overlay to detect clicks outside the menu
            button(Space::new().width(Length::Fill).height(Length::Fill))
                .on_press(Message::CloseMethodMenu)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_theme, _status| button::Style {
                    background: Some(Background::Color(Color::TRANSPARENT)),
                    border: Border::default(),
                    shadow: Shadow::default(),
                    text_color: Color::TRANSPARENT,
                    snap: true
                }),
            // The actual dropdown menu
            container(method_dropdown())
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::new(12.0).top(100.0)) // Left padding 12, top padding 100 to position dropdown
        ]
        .into()
    } else {
        main_content.into()
    };

    // Add tooltip overlay if needed
    if show_url_tooltip {
        stack![
            base_layout,
            // Tooltip overlay
            container(
                container(
                    column![
                        text(tooltip_variable_name)
                            .size(12)
                            .color(Color::from_rgb(0.8, 0.8, 1.0)), // Light blue for header
                        text(tooltip_variable_value)
                            .size(11)
                            .color(Color::WHITE),
                    ]
                    .spacing(4)
                )
                .padding(8)
                .style(|_theme| container::Style {
                    background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 1.0))),
                    border: Border {
                        color: Color::from_rgb(0.6, 0.6, 0.6),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    text_color: Some(Color::WHITE),
                    shadow: Shadow {
                        color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                        offset: Vector::new(2.0, 2.0),
                        blur_radius: 4.0,
                    },
                    snap: true,
                })
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::new(tooltip_position.0).top(tooltip_position.1))
        ]
        .into()
    } else {
        base_layout
    }
}

fn tab_button<'a>(label: &'a str, is_active: bool, tab: RequestTab) -> Element<'a, Message> {
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

fn body_tab<'a>(config: &'a RequestConfig) -> Element<'a, Message> {
    column![
        text_editor(&config.body)
            .on_action(Message::BodyChanged)
            .height(Length::Fill)
            .style(|theme: &Theme, _status: text_editor::Status| {
                text_editor::Style {
                    background: Background::Color(theme.palette().background),
                    border: Border {
                        color: Color::from_rgb(0.9, 0.9, 0.9),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    placeholder: Color::from_rgb(0.6, 0.6, 0.6),
                    value: theme.palette().text,
                    selection: theme.palette().primary,
                }
            })
    ]
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
            .style(move |_theme, status| {
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
            .style(move |_theme, status| {
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
    let auth_type_picker = column![
        text("Authentication Type"),
        pick_list(
            vec![AuthType::None, AuthType::Bearer, AuthType::Basic, AuthType::ApiKey],
            Some(config.auth_type.clone()),
            Message::AuthTypeChanged
        ),
    ]
    .spacing(5);

    let auth_config = match config.auth_type {
        AuthType::None => {
            column![text("No authentication required")]
        }
        AuthType::Bearer => {
            column![
                text("Bearer Token"),
                text_input("Enter bearer token", &config.bearer_token)
                    .on_input(Message::BearerTokenChanged)
                    .width(Fill),
            ]
            .spacing(5)
        }
        AuthType::Basic => {
            column![
                text("Basic Authentication"),
                text("Username"),
                text_input("Enter username", &config.basic_username)
                    .on_input(Message::BasicUsernameChanged)
                    .width(Fill),
                text("Password"),
                text_input("Enter password", &config.basic_password)
                    .on_input(Message::BasicPasswordChanged)
                    .width(Fill),
            ]
            .spacing(5)
        }
        AuthType::ApiKey => {
            column![
                text("API Key Authentication"),
                text("Header Name"),
                text_input("Header name (e.g., X-API-Key)", &config.api_key_header)
                    .on_input(Message::ApiKeyHeaderChanged)
                    .width(Fill),
                text("API Key"),
                text_input("Enter API key", &config.api_key)
                    .on_input(Message::ApiKeyChanged)
                    .width(Fill),
            ]
            .spacing(5)
        }
    };

    column![
        auth_type_picker,
        space().height(10),
        auth_config
    ]
    .spacing(10)
    .into()
}

fn method_button(method: &HttpMethod) -> Element<'_, Message> {
    button(text(method.to_string()))
        .on_press(Message::ToggleMethodMenu)
        .width(Length::Fixed(match method {
            HttpMethod::GET => 50.0,
            HttpMethod::PUT => 50.0,
            HttpMethod::POST => 60.0,
            HttpMethod::HEAD => 60.0,
            HttpMethod::PATCH => 75.0,
            HttpMethod::DELETE => 80.0,
            HttpMethod::OPTIONS => 90.0,
        }))
        .style(|theme: &Theme, _status: Status| {
            button::Style {
                background: Some(Background::Color(theme.palette().background)),
                text_color: theme.palette().text,
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                shadow: Default::default(),
                snap: true,
            }
        })
        .into()
}

fn method_dropdown() -> Element<'static, Message> {
    let button_style = |_theme: &Theme, status: Status| {
        match status {
            Status::Hovered => button::Style {
                background: Some(Background::Color(Color::from_rgb(0.9, 0.9, 0.9))), // Light gray on hover
                text_color: Color::BLACK,
                border: Border::default(),
                shadow: Shadow::default(),
                snap: true,
            },
            _ => button::Style {
                background: Some(Background::Color(Color::WHITE)), // White default
                text_color: Color::BLACK,
                border: Border::default(),
                shadow: Shadow::default(),
                snap: true,
            }
        }
    };

    container(
        column![
            button(text("GET")).on_press(Message::MethodChanged(HttpMethod::GET)).width(Length::Fixed(90.0)).style(button_style),
            button(text("POST")).on_press(Message::MethodChanged(HttpMethod::POST)).width(Length::Fixed(90.0)).style(button_style),
            button(text("PUT")).on_press(Message::MethodChanged(HttpMethod::PUT)).width(Length::Fixed(90.0)).style(button_style),
            button(text("DELETE")).on_press(Message::MethodChanged(HttpMethod::DELETE)).width(Length::Fixed(90.0)).style(button_style),
            button(text("PATCH")).on_press(Message::MethodChanged(HttpMethod::PATCH)).width(Length::Fixed(90.0)).style(button_style),
            button(text("HEAD")).on_press(Message::MethodChanged(HttpMethod::HEAD)).width(Length::Fixed(90.0)).style(button_style),
            button(text("OPTIONS")).on_press(Message::MethodChanged(HttpMethod::OPTIONS)).width(Length::Fixed(90.0)).style(button_style),
        ]
        .spacing(1)
    )
    .padding(4)
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(Color::WHITE)),
        border: Border {
            color: Color::from_rgb(0.9, 0.9, 0.9),
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: None,
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.1),
            offset: Vector::new(0.0, 2.0),
            blur_radius: 4.0,
        },
        snap: true,
    })
    .into()
}
