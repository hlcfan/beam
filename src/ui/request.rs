use crate::types::{AuthType, Environment, HttpMethod, RequestConfig, RequestTab};
use crate::ui::{IconName, icon, url_input};
use iced::widget::button::Status;
use iced::widget::{
    Space, button, column, container, mouse_area, pick_list, row, scrollable, space, stack, text,
    text_editor, text_input,
};
use iced::{Background, Border, Color, Element, Fill, Length, Shadow, Theme, Vector};
use log::info;
use std::time::Instant;
use url_input::UrlInput;

// Action is returned from update function, to trigger a side effect, used in the main

// Message is used within the component, to communicate a user action or event from the UI to the update function.

#[derive(Debug)]
pub enum Action {
    UpdateCurrentRequest(RequestConfig),
    // MonitorRequest(RequestConfig, Instant),
    SendRequest(RequestConfig, Instant),
    CancelRequest(),
    UpdateActiveEnvironment(usize),
    // The components needs to run a task
    Run(iced::Task<Message>),
    EditRequestBody(text_editor::Action),
    // The component does not require any additional actions
    None,
}

#[derive(Debug, Clone)]
pub enum Message {
    ClickSendRequest,
    CancelRequest,
    UrlInputChanged(String),
    UrlInputUndo,
    UrlInputRedo,
    SetProcessingCmdZ(bool),
    UrlInputFocused,
    UrlInputUnfocused,
    MethodChanged(HttpMethod),

    SendButtonHovered(bool),
    CancelButtonHovered(bool),

    TabSelected(RequestTab),
    HeaderKeyChanged(usize, String),
    HeaderValueChanged(usize, String),
    AddHeader,
    RemoveHeader(usize),
    ParamKeyChanged(usize, String),
    ParamValueChanged(usize, String),
    AddParam,
    RemoveParam(usize),
    BodyChanged(text_editor::Action),
    AuthTypeChanged(AuthType),
    BearerTokenChanged(String),
    BasicUsernameChanged(String),
    BasicPasswordChanged(String),
    ApiKeyChanged(String),
    ApiKeyHeaderChanged(String),

    // Environment management
    OpenEnvironmentPopup,
    CloseEnvironmentPopup,
    ToggleMethodMenu,
    CloseMethodMenu,
    DoNothing, // Used to prevent event propagation
    EnvironmentSelected(usize),
}

// #[derive(Debug, Clone, PartialEq)]
// pub enum RequestField {
//     Url,
//     Method,
//     Body,
//     Headers,
//     Params,
//     Auth,
// }

#[derive(Debug, Clone)]
pub struct RequestPanel {
    pub environments: Vec<Environment>,
    pub active_environment: Option<usize>,
    pub method_menu_open: bool,
    pub send_button_hovered: bool,
    pub cancel_button_hovered: bool,
    pub selected_tab: RequestTab,
}

impl Default for RequestPanel {
    fn default() -> Self {
        Self {
            // request_body_content: text_editor::Content::new(),
            selected_tab: RequestTab::Body,
            // current_request: RequestConfig {
            //     name: String::new(),
            //     method: HttpMethod::GET,
            //     url: String::new(),
            //     headers: vec![
            //         ("Content-Type".to_string(), "application/json".to_string()),
            //         ("User-Agent".to_string(), "BeamApp/1.0".to_string()),
            //     ],
            //     params: vec![],
            //     body: String::new(),
            //     content_type: "application/json".to_string(),
            //     auth_type: AuthType::None,
            //     bearer_token: String::new(),
            //     basic_username: String::new(),
            //     basic_password: String::new(),
            //     api_key: String::new(),
            //     api_key_header: "X-API-Key".to_string(),
            //     collection_index: 0,
            //     request_index: 0,
            //     metadata: None,
            // },
            environments: Vec::new(),
            active_environment: None,
            method_menu_open: false,
            send_button_hovered: false,
            cancel_button_hovered: false,
        }
    }
}

impl RequestPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, message: Message, current_request: &RequestConfig) -> Action {
        match message {
            Message::UrlInputChanged(url) => {
                info!("===URL updated to: {:?}", url);
                let mut request = current_request.clone();
                request.url = url;

                Action::UpdateCurrentRequest(request)
            }
            Message::UrlInputUndo => {
                info!("DEBUG: UrlInputUndo message received");
                // TODO: Implement undo functionality for UrlInput
                Action::None
            }
            Message::UrlInputRedo => {
                info!("DEBUG: UrlInputRedo message received");
                // TODO: Implement redo functionality for UrlInput
                Action::None
            }
            Message::UrlInputFocused => {
                // TODO: Implement focus handling for UrlInput
                Action::None
            }
            Message::UrlInputUnfocused => {
                // TODO: Implement unfocus handling for UrlInput
                Action::None
            }
            Message::SetProcessingCmdZ(processing) => {
                // self.processing_cmd_z = processing;
                Action::None
            }
            Message::MethodChanged(method) => {
                let mut request = current_request.clone();
                request.method = method;
                self.method_menu_open = false; // Close menu after selection

                Action::UpdateCurrentRequest(request)
            }
            Message::ClickSendRequest => {
                // TODO: Parent to check this action
                // Action::MonitorRequest(current_request.clone(), std::time::Instant::now())
                Action::SendRequest(current_request.clone(), std::time::Instant::now())
            }
            Message::CancelRequest => Action::CancelRequest(),
            Message::SendButtonHovered(hovered) => {
                self.send_button_hovered = hovered;
                Action::None
            }
            Message::CancelButtonHovered(hovered) => {
                self.cancel_button_hovered = hovered;
                Action::None
            }
            Message::TabSelected(tab) => {
                // TODO: handle selected tab on component level
                // current_request.clone().selected_tab = tab;
                Action::None
            }

            Message::HeaderKeyChanged(index, key) => {
                if let Some(header) = current_request.clone().headers.get_mut(index) {
                    header.0 = key;
                }

                Action::UpdateCurrentRequest(current_request.clone())
            }
            Message::HeaderValueChanged(index, value) => {
                if let Some(header) = current_request.clone().headers.get_mut(index) {
                    header.1 = value;
                }

                Action::UpdateCurrentRequest(current_request.clone())
            }
            Message::AddHeader => {
                current_request
                    .clone()
                    .headers
                    .push((String::new(), String::new()));
                Action::None
            }
            Message::RemoveHeader(index) => {
                if index < current_request.clone().headers.len() {
                    current_request.clone().headers.remove(index);
                }
                Action::None
            }
            Message::ParamKeyChanged(index, key) => {
                if let Some(param) = current_request.clone().params.get_mut(index) {
                    param.0 = key;
                }

                Action::UpdateCurrentRequest(current_request.clone().clone())
            }
            Message::ParamValueChanged(index, value) => {
                if let Some(param) = current_request.clone().params.get_mut(index) {
                    param.1 = value;
                }

                Action::UpdateCurrentRequest(current_request.clone().clone())
            }
            Message::AddParam => {
                current_request
                    .clone()
                    .params
                    .push((String::new(), String::new()));

                Action::UpdateCurrentRequest(current_request.clone())
            }
            Message::RemoveParam(index) => {
                if index < current_request.clone().params.len() {
                    current_request.clone().params.remove(index);
                }

                Action::UpdateCurrentRequest(current_request.clone())
            }
            Message::BodyChanged(action) => Action::EditRequestBody(action),
            Message::AuthTypeChanged(auth_type) => {
                current_request.clone().auth_type = auth_type;

                Action::UpdateCurrentRequest(current_request.clone())
            }
            Message::BearerTokenChanged(token) => {
                current_request.clone().bearer_token = token;

                Action::UpdateCurrentRequest(current_request.clone())
            }
            Message::BasicUsernameChanged(username) => {
                current_request.clone().basic_username = username;

                Action::UpdateCurrentRequest(current_request.clone())
            }
            Message::BasicPasswordChanged(password) => {
                current_request.clone().basic_password = password;

                Action::UpdateCurrentRequest(current_request.clone())
            }
            Message::ApiKeyChanged(api_key) => {
                current_request.clone().api_key = api_key;

                Action::UpdateCurrentRequest(current_request.clone())
            }
            Message::ApiKeyHeaderChanged(header) => {
                current_request.clone().api_key_header = header;

                Action::UpdateCurrentRequest(current_request.clone())
            }
            // Environment message handlers
            Message::OpenEnvironmentPopup => {
                // TODO
                // self.show_environment_popup = true;
                Action::None
            }
            Message::CloseEnvironmentPopup => {
                // TODO
                // self.show_environment_popup = false;
                Action::None
            }
            Message::EnvironmentSelected(index) => {
                if index < self.environments.len() {
                    self.active_environment = Some(index);
                    Action::UpdateActiveEnvironment(index)

                    //     // Save the active environment to storage
                    //     let environments = self.environments.clone();
                    //     let active_env_name = self.environments[index].name.clone();
                    //     Task::perform(
                    //         async move {
                    //             match storage::StorageManager::with_default_config().await {
                    //                 Ok(storage_manager) => {
                    //                     if let Err(e) = storage_manager
                    //                         .storage()
                    //                         .save_environments_with_active(
                    //                             &environments,
                    //                             Some(&active_env_name),
                    //                         )
                    //                         .await
                    //                     {
                    //                         error!("Failed to save active environment: {}", e);
                    //                     }
                    //                 }
                    //                 Err(e) => error!("Failed to create storage manager: {}", e),
                    //             }
                    //             Message::DoNothing
                    //         },
                    //         |msg| msg,
                    //     )
                    // } else {
                    //     Action::None
                    // }
                } else {
                    Action::None
                }
            }
            Message::ToggleMethodMenu => {
                self.method_menu_open = !self.method_menu_open;
                Action::None
            }
            Message::CloseMethodMenu => {
                self.method_menu_open = false;
                Action::None
            }
            Message::DoNothing => Action::None,
        }

        // // Only trigger auto-save for actual content-changing actions
        // let should_save = match &action {
        //     text_editor::Action::Edit(_) => true, // Only Edit actions change content
        //     _ => false, // All other actions (Move, Select, Click, Drag, Scroll) don't change content
        // };

        // self.request_body_content.perform(action);
        // let body_text = self.request_body_content.text();

        // (should_save, body_text)
    }

    pub fn view<'a>(
        &'a self,
        current_request: &'a RequestConfig,
        request_body_content: &'a text_editor::Content,
        is_loading: bool,
        environments: &'a [Environment],
        active_environment: Option<usize>,
    ) -> Element<'_, Message> {
        // Environment pick_list for the URL row
        let env_pick_list = {
            // Create list of environment options including all self.environments plus "Configure"
            let mut env_options: Vec<String> =
                environments.iter().map(|env| env.name.clone()).collect();
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

            pick_list(env_options, selected_env, |selected| {
                if selected == "Configure" {
                    Message::OpenEnvironmentPopup
                } else {
                    // Find the index of the selected environment
                    if let Some(index) = self
                        .environments
                        .iter()
                        .position(|env| env.name == selected)
                    {
                        Message::EnvironmentSelected(index)
                    } else {
                        Message::DoNothing
                    }
                }
            })
            .width(150)
            .placeholder("No Environment")
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
        let method_label = method_button(&current_request.method);

        // Create send/cancel button based on loading state
        let url = current_request.url.clone();
        let url_valid = !url.trim().is_empty()
            && (url.starts_with("http://") || url.starts_with("https://") || url.contains("{{"));

        let send_button = if is_loading {
            // Show cancel icon when loading
            let cancel_color = if self.cancel_button_hovered {
                Color::from_rgb(0.3, 0.3, 0.3) // Darker gray on hover
            } else {
                Color::from_rgb(0.5, 0.5, 0.5) // Light gray default
            };

            mouse_area(
                button(icon(IconName::Cancel).size(16).color(cancel_color))
                    .padding(8)
                    .on_press(Message::CancelRequest)
                    .style(icon_button_style(true)),
            )
            .on_enter(Message::CancelButtonHovered(true))
            .on_exit(Message::CancelButtonHovered(false))
        } else {
            // Show send icon when not loading
            let send_color = if self.send_button_hovered {
                Color::from_rgb(0.3, 0.3, 0.3) // Darker gray on hover
            } else {
                Color::from_rgb(0.5, 0.5, 0.5) // Light gray default
            };

            if url_valid {
                mouse_area(
                    button(icon(IconName::Send).size(16).color(send_color))
                        .padding(8)
                        .on_press(Message::ClickSendRequest)
                        .style(icon_button_style(true)),
                )
                .on_enter(Message::SendButtonHovered(true))
                .on_exit(Message::SendButtonHovered(false))
            } else {
                mouse_area(
                    button(icon(IconName::Send).size(16).color(send_color))
                        .padding(8)
                        .style(icon_button_style(false)),
                )
                .on_enter(Message::SendButtonHovered(true))
                .on_exit(Message::SendButtonHovered(false))
            }
        };

        info!("====URL: {:?}", url);
        let base_input = container(row![
            method_label,
            UrlInput::new("Enter URL...", &url).on_input(Message::UrlInputChanged),
            space().width(5),
            send_button,
        ])
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

        let url_row = row![connected_input].align_y(iced::Alignment::Center);

        let tabs = row![
            tab_button(
                "Body",
                self.selected_tab == RequestTab::Body,
                RequestTab::Body
            ),
            tab_button(
                "Params",
                self.selected_tab == RequestTab::Params,
                RequestTab::Params
            ),
            tab_button(
                "Headers",
                self.selected_tab == RequestTab::Headers,
                RequestTab::Headers
            ),
            tab_button(
                "Auth",
                self.selected_tab == RequestTab::Auth,
                RequestTab::Auth
            ),
        ]
        .spacing(5);

        // let request_body_content = text_editor::Content::with_text(current_request.body.as_str());
        let tab_content = match self.selected_tab {
            RequestTab::Body => body_tab(&request_body_content),
            RequestTab::Params => params_tab(&current_request),
            RequestTab::Headers => headers_tab(&current_request),
            RequestTab::Auth => auth_tab(&current_request),
            // RequestTab::Environment => body_tab(&request_body_content), // Fallback to body tab if somehow Environment is selected
        };

        let content = column![
            env_bar,
            space().height(5),
            url_row,
            space().height(5),
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
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.9, 0.9, 0.9,
                ))), // Gray
                ..Default::default()
            });

        // Create right border
        let right_border = container("")
            .width(Length::Fixed(1.0))
            .height(Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.9, 0.9, 0.9,
                ))), // Gray
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

        let base_layout = if self.method_menu_open {
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

        base_layout
    }

    // pub fn set_url(&mut self, url: String) {
    //     self.url = url;
    // }

    // pub fn set_body_content(&mut self, body: String) {
    //     self.request_body_content
    //         .perform(text_editor::Action::SelectAll);
    //     self.request_body_content
    //         .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
    //             body.into(),
    //         )));
    // }

    // pub fn get_body_text(&self) -> String {
    //     self.request_body_content.text()
    // }

    // pub fn handle_body_action(&mut self, action: text_editor::Action) {
    //     self.request_body_content.perform(action);
    // }
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

fn body_tab<'a>(request_body: &'a text_editor::Content) -> Element<'a, Message> {
    // let content = text_editor::Content::with_text(&request_body.clone());
    // let content = text_editor::Content::with_text(request_body.as_str());
    let text_editor_widget = text_editor(request_body)
        .on_action(Message::BodyChanged)
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

    scrollable(text_editor_widget).height(Length::Fill).into()
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
            }),
    );

    scrollable(content.spacing(10)).height(Length::Fill).into()
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
            }),
    );

    scrollable(content.spacing(10)).height(Length::Fill).into()
}

// Helper function for send/cancel button styling
fn icon_button_style(is_interactive: bool) -> impl Fn(&Theme, Status) -> button::Style {
    move |_theme, status| {
        let base = button::Style::default();
        match status {
            Status::Hovered if is_interactive => button::Style {
                background: Some(Background::Color(Color::from_rgb(0.93, 0.93, 0.93))),
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

fn auth_tab<'a>(config: &'a RequestConfig) -> Element<'a, Message> {
    let auth_type_picker = column![
        text("Authentication Type"),
        pick_list(
            vec![
                AuthType::None,
                AuthType::Bearer,
                AuthType::Basic,
                AuthType::ApiKey
            ],
            Some(config.auth_type.clone()),
            Message::AuthTypeChanged
        ),
    ]
    .spacing(5);

    let auth_config = match config.auth_type {
        AuthType::None => {
            column![text("No authentication required")]
        }
        AuthType::Bearer => column![
            text("Bearer Token"),
            text_input("Enter bearer token", &config.bearer_token)
                .on_input(Message::BearerTokenChanged)
                .width(Fill),
        ]
        .spacing(5),
        AuthType::Basic => column![
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
        .spacing(5),
        AuthType::ApiKey => column![
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
        .spacing(5),
    };

    column![auth_type_picker, space().height(10), auth_config]
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
        .style(|theme: &Theme, _status: Status| button::Style {
            background: Some(Background::Color(theme.palette().background)),
            text_color: theme.palette().text,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            shadow: Default::default(),
            snap: true,
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
            },
        }
    };

    // Array of all HTTP methods
    let methods = [
        HttpMethod::GET,
        HttpMethod::POST,
        HttpMethod::PUT,
        HttpMethod::DELETE,
        HttpMethod::PATCH,
        HttpMethod::HEAD,
        HttpMethod::OPTIONS,
    ];

    // Create buttons for each method using a loop
    let method_buttons: Vec<Element<'static, Message>> = methods
        .iter()
        .map(|method| {
            button(text(method.to_string()))
                .on_press(Message::MethodChanged(method.clone()))
                .width(Length::Fixed(90.0))
                .style(button_style)
                .into()
        })
        .collect();

    container(column(method_buttons))
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
