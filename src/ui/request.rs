use crate::constant::{REQUEST_BODY_EDITOR_ID, REQUEST_BODY_SCROLLABLE_ID};
use crate::types::{AuthType, BodyFormat, Environment, HttpMethod, RequestConfig, RequestTab};
use crate::ui::floating_element;
use crate::ui::undoable_editor::UndoableEditor;
use crate::ui::undoable_input::UndoableInput;
use crate::ui::{IconName, icon, undoable_editor, undoable_input};
use iced::highlighter::{self};
use iced::widget::button::Status;
use iced::widget::{
    Space, button, column, container, mouse_area, pick_list, row, scrollable, space, stack, text,
    text_editor, text_input,
};
use iced::{
    Background, Border, Color, Element, Fill, Length, Padding, Shadow, Task, Theme, Vector,
};
use log::info;
use std::time::Instant;
use tokio;

// Action is returned from update function, to trigger a side effect, used in the main

// Message is used within the component, to communicate a user action or event from the UI to the update function.

#[derive(Debug)]
pub enum Action {
    UpdateCurrentRequest(RequestConfig),
    // MonitorRequest(RequestConfig, Instant),
    SendRequest(Instant),
    CancelRequest(),
    UpdateActiveEnvironment(usize),
    // The components needs to run a task
    Run(iced::Task<Message>),
    EditRequestBody(text_editor::Action),
    EditRequestPostRequestScript(text_editor::Action),
    Focus(iced::widget::Id),
    SearchNext(iced::widget::Id),
    SearchPrevious(iced::widget::Id),
    SubmitSearch(iced::widget::Id),
    FormatRequestBody(String),
    OpenEnvironmentPopup,
    // The component does not require any additional actions
    None,
}

#[derive(Debug, Clone)]
pub enum Message {
    ClickSendRequest,
    CancelRequest,
    UrlInputMessage(undoable_input::Message),
    EditorMessage(undoable_editor::Message),
    // UrlInputChanged(String),
    // SetProcessingCmdZ(bool),
    // UrlInputFocused,
    // UrlInputUnfocused,
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
    BodyFormatChanged(BodyFormat),
    AuthTypeChanged(AuthType),
    BearerTokenChanged(String),
    BasicUsernameChanged(String),
    BasicPasswordChanged(String),
    ApiKeyChanged(String),
    ApiKeyHeaderChanged(String),
    ScriptChanged(text_editor::Action),

    // Environment management
    OpenEnvironmentPopup,
    ToggleMethodMenu,
    CloseMethodMenu,
    ToggleBodyFormatMenu,
    CloseBodyFormatMenu,
    FormatRequestBody,
    DoNothing, // Used to prevent event propagation
    EnvironmentSelected(usize),
    SearchQueryChanged(String),
    FindNext,
    FindPrevious,
    SubmitSearch,
    CloseSearch,
    SearchFound(text_editor::Position, text_editor::Position),
    SearchNotFound,
    FocusSearch,
    ScrollToMatchResponse(f32),
}

#[derive(Debug, Clone)]
pub struct RequestPanel {
    pub method_menu_open: bool,
    pub body_format_menu_open: bool,
    pub send_button_hovered: bool,
    pub cancel_button_hovered: bool,
    pub selected_tab: RequestTab,
    pub script_editor_content: text_editor::Content,
    pub show_search: bool,
    pub search_query: String,
    pub search_input_id: iced::widget::Id,
    pub search_selection: Option<(text_editor::Position, text_editor::Position)>,
    pub url_input: UndoableInput,
    pub body_editor: UndoableEditor,
}

impl Default for RequestPanel {
    fn default() -> Self {
        Self {
            selected_tab: RequestTab::Body,
            url_input: UndoableInput::new_empty("Enter URL...".to_string())
                .size(14.0)
                .padding(8.0),
            body_editor: UndoableEditor::new_empty().height(iced::Length::Fixed(200.0)),
            method_menu_open: false,
            body_format_menu_open: false,
            send_button_hovered: false,
            cancel_button_hovered: false,

            script_editor_content: text_editor::Content::new(),
            show_search: false,
            search_query: String::new(),
            search_input_id: iced::widget::Id::unique(),
            search_selection: None,
        }
    }
}

impl RequestPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_undo_histories<'a>(&mut self) {
        self.url_input = UndoableInput::new_empty("Enter URL...".to_string());
        self.body_editor = UndoableEditor::new_empty();
    }

    pub fn update<'a>(
        &mut self,
        message: Message,
        current_request: &RequestConfig,
        environments: &'a Vec<Environment>,
        request_body_content: &mut text_editor::Content,
    ) -> Action {
        match message {
            // Message::UrlInputChanged(url) => {
            //     let mut request = current_request.clone();
            //     request.url = url;

            //     Action::UpdateCurrentRequest(request)
            // }
            // Message::UrlInputFocused => {
            //     // TODO: Implement focus handling for UrlInput
            //     Action::None
            // }
            // Message::UrlInputUnfocused => {
            //     // TODO: Implement unfocus handling for UrlInput
            //     Action::None
            // }
            // Message::SetProcessingCmdZ(processing) => {
            //     // self.processing_cmd_z = processing;
            //     Action::None
            // }
            Message::EditorMessage(message) => {
                if let undoable_editor::Message::Find = message {
                    self.show_search = true;
                    return Action::Run(Task::perform(
                        async {
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        },
                        |_| Message::FocusSearch,
                    ));
                }

                if let Some(new_text) = self.body_editor.update(message, request_body_content) {
                    // println!("Editor changed to: {:?}", new_text);
                    let mut request = current_request.clone();
                    request.body = new_text;

                    return Action::UpdateCurrentRequest(request);
                }
                Action::None

                // match message {
                //     undoable_editor::EditorMessage::Action(action) => {
                //         info!("===Action: {:?}", action);
                //         // match action {
                //         //     text_editor::Ch
                //         // }
                //         // self.url_undo_history.push(url.clone());
                //         // request.url = url;

                //         // Action::UpdateCurrentRequest(request)

                //         Action::EditRequestBody(action)
                //     }
                //     undoable_editor::EditorMessage::Undo => {
                //         // if let Some(prev) = self.body_undo_history.undo() {
                //         //     info!("===editor message: {:?}", message);
                //         //     request.body = prev;

                //         //     return Action::UpdateCurrentRequest(request);
                //         // }

                //         info!("===no prev");
                //         Action::None
                //     }
                //     undoable_editor::EditorMessage::Redo => {
                //         // if let Some(next) = self.body_undo_history.redo() {
                //         //     request.body = next;

                //         //     return Action::UpdateCurrentRequest(request);
                //         // }

                //         Action::None
                //     }
                // }
            }
            Message::UrlInputMessage(message) => {
                info!("====UrlInputMessage: {:?}", message);
                if let Some(new_value) = self.url_input.update(message) {
                    println!("URL Input changed to: {:?}", new_value);
                    let mut request = current_request.clone();
                    request.url = new_value;

                    return Action::UpdateCurrentRequest(request);
                }

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
                Action::SendRequest(std::time::Instant::now())
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
                self.selected_tab = tab;

                Action::None
            }
            Message::HeaderKeyChanged(index, key) => {
                let mut request = current_request.clone();

                if let Some(header) = request.headers.get_mut(index) {
                    header.0 = key;
                }

                Action::UpdateCurrentRequest(request)
            }
            Message::HeaderValueChanged(index, value) => {
                let mut request = current_request.clone();

                if let Some(header) = request.headers.get_mut(index) {
                    header.1 = value;
                }

                Action::UpdateCurrentRequest(request)
            }
            Message::AddHeader => {
                let mut request = current_request.clone();

                request.headers.push((String::new(), String::new()));

                Action::UpdateCurrentRequest(request)
            }
            Message::RemoveHeader(index) => {
                let mut request = current_request.clone();

                if index < request.clone().headers.len() {
                    request.headers.remove(index);
                }

                Action::UpdateCurrentRequest(request)
            }
            Message::ParamKeyChanged(index, key) => {
                let mut request = current_request.clone();

                if let Some(param) = request.params.get_mut(index) {
                    param.0 = key;
                }

                Action::UpdateCurrentRequest(request)
            }
            Message::ParamValueChanged(index, value) => {
                let mut request = current_request.clone();

                if let Some(param) = request.params.get_mut(index) {
                    param.1 = value;
                }

                Action::UpdateCurrentRequest(request)
            }
            Message::AddParam => {
                let mut request = current_request.clone();

                request.params.push((String::new(), String::new()));

                Action::UpdateCurrentRequest(request)
            }
            Message::RemoveParam(index) => {
                let mut request = current_request.clone();

                if index < request.clone().params.len() {
                    request.params.remove(index);
                }

                Action::UpdateCurrentRequest(request)
            }
            Message::BodyChanged(action) => Action::EditRequestBody(action),
            Message::BodyFormatChanged(format) => {
                let mut request = current_request.clone();
                request.body_format = format;
                // Update Content-Type based on selected body format
                request.content_type = match format {
                    BodyFormat::Json => "application/json".to_string(),
                    BodyFormat::Xml => "application/xml".to_string(),
                    BodyFormat::Text => "text/plain".to_string(),
                    BodyFormat::GraphQL => "application/graphql".to_string(),
                    BodyFormat::None => request.content_type,
                };
                // Dismiss the dropdown after selecting a format
                self.body_format_menu_open = false;
                // Keep focus on Body tab when changing format
                self.selected_tab = RequestTab::Body;

                Action::UpdateCurrentRequest(request)
            }
            Message::FormatRequestBody => {
                let request = current_request.clone();
                let mut formatted_body = None;

                // Format the body based on the current format
                match request.body_format {
                    BodyFormat::Json => {
                        if let Ok(json_value) =
                            serde_json::from_str::<serde_json::Value>(&request.body)
                        {
                            let formatted = serde_json::to_string_pretty(&json_value).unwrap();

                            log::info!(
                                "JSON formatting: original length={}, formatted length={}",
                                request.body.len(),
                                formatted.len()
                            );
                            formatted_body = Some(formatted);
                        } else {
                            log::info!("JSON formatting failed: invalid JSON input");
                        }
                    }
                    BodyFormat::Xml => {
                        // For XML, we'll just trim whitespace for now
                        // In a real implementation, you'd use an XML formatter
                        let trimmed = request.body.trim().to_string();
                        log::info!(
                            "XML formatting: original length={}, trimmed length={}",
                            request.body.len(),
                            trimmed.len()
                        );
                        formatted_body = Some(trimmed);
                    }
                    BodyFormat::Text => {
                        // For text, trim leading/trailing whitespace
                        let trimmed = request.body.trim().to_string();
                        log::info!(
                            "Text formatting: original length={}, trimmed length={}",
                            request.body.len(),
                            trimmed.len()
                        );
                        formatted_body = Some(trimmed);
                    }
                    BodyFormat::GraphQL | BodyFormat::None => {
                        // No formatting for GraphQL or None
                        log::info!("No formatting applied for {:?}", request.body_format);
                    }
                }

                if let Some(formatted) = formatted_body {
                    Action::FormatRequestBody(formatted)
                } else {
                    Action::None
                }
            }
            Message::AuthTypeChanged(auth_type) => {
                let mut request = current_request.clone();

                request.auth_type = auth_type;

                Action::UpdateCurrentRequest(request)
            }
            Message::BearerTokenChanged(token) => {
                let mut request = current_request.clone();
                request.bearer_token = token;

                Action::UpdateCurrentRequest(request)
            }
            Message::BasicUsernameChanged(username) => {
                let mut request = current_request.clone();
                request.basic_username = username;

                Action::UpdateCurrentRequest(request)
            }
            Message::BasicPasswordChanged(password) => {
                let mut request = current_request.clone();
                request.basic_password = password;

                Action::UpdateCurrentRequest(request)
            }
            Message::ApiKeyChanged(api_key) => {
                let mut request = current_request.clone();
                request.api_key = api_key;

                Action::UpdateCurrentRequest(request)
            }
            Message::ApiKeyHeaderChanged(header) => {
                let mut request = current_request.clone();
                request.api_key_header = header;

                Action::UpdateCurrentRequest(request)
            }
            // Environment message handlers
            Message::OpenEnvironmentPopup => Action::OpenEnvironmentPopup,
            Message::EnvironmentSelected(index) => {
                if index < environments.len() {
                    Action::UpdateActiveEnvironment(index)
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
                        iced::widget::Id::new(crate::constant::REQUEST_BODY_SCROLLABLE_ID),
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
            Message::FocusSearch => Action::Focus(self.search_input_id.clone()),

            Message::ToggleMethodMenu => {
                self.method_menu_open = !self.method_menu_open;
                Action::None
            }
            Message::CloseMethodMenu => {
                self.method_menu_open = false;
                Action::None
            }
            Message::ToggleBodyFormatMenu => {
                self.body_format_menu_open = !self.body_format_menu_open;
                self.selected_tab = RequestTab::Body; // Always select Body tab when toggling dropdown
                Action::None
            }
            Message::CloseBodyFormatMenu => {
                self.body_format_menu_open = false;
                Action::None
            }
            Message::ScriptChanged(action) => {
                return Action::EditRequestPostRequestScript(action);
                // // Update the script editor content and sync to request
                // self.script_editor_content.perform(action);
                // let mut request = current_request.clone();
                // request.post_request_script = Some(self.script_editor_content.text());
                // Action::UpdateCurrentRequest(request)
            }
            Message::DoNothing => Action::None,
        }

        // Action::None
    }

    pub fn view<'a>(
        &'a self,
        current_request: &'a RequestConfig,
        request_body_content: &'a text_editor::Content,
        post_script_content: &'a text_editor::Content,
        is_loading: bool,
        environments: &'a [Environment],
        active_environment: Option<usize>,
    ) -> Element<'a, Message> {
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
                    if let Some(index) = environments.iter().position(|env| env.name == selected) {
                        Message::EnvironmentSelected(index)
                    } else {
                        Message::DoNothing
                    }
                }
            })
            .width(Length::Fill)
            .placeholder("No Environment")
        };

        // Environment bar
        let env_bar = row![
            text("Environment:").size(14),
            space().width(5),
            env_pick_list,
        ]
        .align_y(iced::Alignment::Center)
        .width(Length::Fill);

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

        let base_input = container(row![
            method_label,
            self.url_input
                .view(&current_request.url)
                .map(Message::UrlInputMessage),
            space().width(1),
            send_button,
        ])
        .padding(2)
        .align_y(iced::alignment::Vertical::Center)
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

        // Body tab button (format button moved into body editor overlay)
        // Label: default "Body"; after selection show selected format using Content-Type
        let body_label = if !current_request.content_type.is_empty() {
            match body_label_from_content_type(&current_request.content_type) {
                Some(lbl) => lbl.to_string(),
                None => current_request.body_format.to_string(),
            }
        } else {
            "Body".to_string()
        };

        let body_tab_button = tab_button(
            body_label,
            self.selected_tab == RequestTab::Body,
            RequestTab::Body,
        );

        let tabs = row![
            body_tab_button,
            tab_button(
                "Params".to_string(),
                self.selected_tab == RequestTab::Params,
                RequestTab::Params
            ),
            tab_button(
                "Headers".to_string(),
                self.selected_tab == RequestTab::Headers,
                RequestTab::Headers
            ),
            tab_button(
                "Auth".to_string(),
                self.selected_tab == RequestTab::Auth,
                RequestTab::Auth
            ),
            tab_button(
                "Script".to_string(),
                self.selected_tab == RequestTab::PostScript,
                RequestTab::PostScript
            ),
        ]
        .spacing(5);

        let tab_content = match self.selected_tab {
            RequestTab::Body => self.body_tab(&request_body_content, current_request.body_format),
            RequestTab::Params => params_tab(&current_request),
            RequestTab::Headers => headers_tab(&current_request),
            RequestTab::Auth => auth_tab(&current_request),
            RequestTab::PostScript => post_script_tab(&post_script_content),
            // RequestTab::Environment => body_tab(&request_body_content); // Fallback to body tab if somehow Environment is selected
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

        let base_layout = if self.method_menu_open || self.body_format_menu_open {
            let overlay_message = if self.method_menu_open {
                Message::CloseMethodMenu
            } else {
                Message::CloseBodyFormatMenu
            };

            let dropdown_content = if self.method_menu_open {
                method_dropdown()
            } else {
                body_format_dropdown()
            };

            let dropdown_padding = if self.method_menu_open {
                iced::Padding::new(12.0).top(100.0)
            } else {
                iced::Padding::new(12.0).top(150.0) // Position body format dropdown lower
            };

            stack![
                main_content,
                // Transparent overlay to detect clicks outside the menu
                button(Space::new().width(Length::Fill).height(Length::Fill))
                    .on_press(overlay_message)
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
                container(dropdown_content)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(dropdown_padding)
            ]
            .into()
        } else {
            main_content.into()
        };

        base_layout
    }

    fn body_tab<'a>(
        &'a self,
        request_body: &'a text_editor::Content,
        body_format: BodyFormat,
    ) -> Element<'a, Message> {
        match body_format {
            BodyFormat::None => container(
                text("No body")
                    .size(14)
                    .color(Color::from_rgb(0.6, 0.6, 0.6)),
            )
            .center_x(Fill)
            .center_y(Fill)
            .width(Fill)
            .height(Fill)
            .into(),
            _ => {
                let syntax = match body_format {
                    BodyFormat::Json => Some("json"),
                    BodyFormat::Xml => Some("xml"),
                    BodyFormat::GraphQL => Some("graphql"),
                    _ => None,
                };

                let editor_area = scrollable(
                    self.body_editor
                        .view(
                            REQUEST_BODY_EDITOR_ID,
                            request_body,
                            syntax,
                            Some(self.search_query.as_str()),
                            self.search_selection,
                        )
                        .map(Message::EditorMessage),
                )
                .id(iced::widget::Id::new(REQUEST_BODY_SCROLLABLE_ID))
                .height(Length::Fill);

                let format_button = body_format_button();

                let editor_with_format =
                    floating_element::FloatingElement::new(editor_area, format_button)
                        .offset(iced::Vector::new(10.0, 5.0))
                        .position(floating_element::AnchorPosition::TopRight)
                        .height(Length::Fill);

                if self.show_search {
                    let search_bar = iced::widget::container(
                        iced::widget::row![
                            iced::widget::text_input("Find", &self.search_query)
                                .id(self.search_input_id.clone())
                                .on_input(Message::SearchQueryChanged)
                                .on_submit(Message::SubmitSearch)
                                .width(Length::Fixed(200.0))
                                .padding(1)
                                .style(|theme: &Theme, _status| iced::widget::text_input::Style {
                                    background: iced::Background::Color(iced::Color::WHITE),
                                    border: iced::Border {
                                        width: 0.0,
                                        color: iced::Color::TRANSPARENT,
                                        radius: 6.0.into(),
                                    },
                                    icon: theme.palette().text,
                                    placeholder: iced::Color::from_rgb(0.6, 0.6, 0.6),
                                    value: theme.palette().text,
                                    selection: theme.palette().primary,
                                }),
                            iced::widget::button(
                                icon(IconName::ChevronDown)
                                    .size(14)
                                    .color(iced::Color::from_rgb(0.4, 0.4, 0.4))
                            )
                            .on_press(Message::FindNext)
                            .padding(1)
                            .style(|_theme, status| {
                                let base = iced::widget::button::Style {
                                    background: None,
                                    border: iced::Border {
                                        radius: 6.0.into(),
                                        ..iced::Border::default()
                                    },
                                    ..iced::widget::button::Style::default()
                                };
                                match status {
                                    iced::widget::button::Status::Hovered => {
                                        iced::widget::button::Style {
                                            background: Some(iced::Background::Color(
                                                iced::Color::from_rgb(0.85, 0.85, 0.85),
                                            )),
                                            ..base
                                        }
                                    }
                                    _ => base,
                                }
                            }),
                            iced::widget::button(
                                icon(IconName::ChevronUp)
                                    .size(14)
                                    .color(iced::Color::from_rgb(0.4, 0.4, 0.4))
                            )
                            .on_press(Message::FindPrevious)
                            .padding(1)
                            .style(|_theme, status| {
                                let base = iced::widget::button::Style {
                                    background: None,
                                    border: iced::Border {
                                        radius: 6.0.into(),
                                        ..iced::Border::default()
                                    },
                                    ..iced::widget::button::Style::default()
                                };
                                match status {
                                    iced::widget::button::Status::Hovered => {
                                        iced::widget::button::Style {
                                            background: Some(iced::Background::Color(
                                                iced::Color::from_rgb(0.85, 0.85, 0.85),
                                            )),
                                            ..base
                                        }
                                    }
                                    _ => base,
                                }
                            }),
                            iced::widget::button(
                                icon(IconName::Close)
                                    .size(14)
                                    .color(iced::Color::from_rgb(0.4, 0.4, 0.4))
                            )
                            .on_press(Message::CloseSearch)
                            .padding(1)
                            .style(|_theme, status| {
                                let base = iced::widget::button::Style {
                                    background: None,
                                    border: iced::Border {
                                        radius: 6.0.into(),
                                        ..iced::Border::default()
                                    },
                                    ..iced::widget::button::Style::default()
                                };
                                match status {
                                    iced::widget::button::Status::Hovered => {
                                        iced::widget::button::Style {
                                            background: Some(iced::Background::Color(
                                                iced::Color::from_rgb(0.85, 0.85, 0.85),
                                            )),
                                            ..base
                                        }
                                    }
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
    }
}

fn tab_button<'a>(label: String, is_active: bool, tab: RequestTab) -> Element<'a, Message> {
    // Special handling for Body tab - clicking it should toggle the format dropdown
    let message = match tab {
        RequestTab::Body => Message::ToggleBodyFormatMenu,
        _ => Message::TabSelected(tab.clone()),
    };

    let content: Element<'a, Message> = match tab {
        RequestTab::Body => {
            let chevron =
                icon(IconName::ChevronDown)
                    .size(Length::Fixed(14.0))
                    .color(if is_active {
                        Color::WHITE
                    } else {
                        Color::from_rgb(0.2, 0.2, 0.2)
                    });

            row![
                text(label),
                space::Space::new().width(Length::Fixed(6.0)),
                chevron,
            ]
            .align_y(iced::alignment::Vertical::Center)
            .into()
        }
        _ => text(label).into(),
    };

    button(content)
        .on_press(message)
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

fn dropdown_item_style() -> impl Fn(&Theme, Status) -> button::Style {
    |_theme: &Theme, status: Status| match status {
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
        .padding(Padding::from(7))
        .width(Length::Fixed(match method {
            HttpMethod::GET => 40.0,
            HttpMethod::PUT => 40.0,
            HttpMethod::POST => 50.0,
            HttpMethod::HEAD => 50.0,
            HttpMethod::PATCH => 65.0,
            HttpMethod::DELETE => 65.0,
            HttpMethod::OPTIONS => 80.0,
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

fn body_format_button() -> Element<'static, Message> {
    button(
        icon(IconName::Indent)
            .size(28)
            .color(Color::from_rgb(0.5, 0.5, 0.5)),
    )
    .on_press(Message::FormatRequestBody)
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

fn method_dropdown() -> Element<'static, Message> {
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
                .style(dropdown_item_style())
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

// Map request Content-Type to a short Body tab label
fn body_label_from_content_type(content_type: &str) -> Option<&'static str> {
    let ct = content_type.to_lowercase();
    if ct.contains("json") {
        Some("JSON")
    } else if ct.contains("xml") || ct.contains("html") {
        Some("XML")
    } else if ct.contains("graphql") {
        Some("GraphQL")
    } else if ct.starts_with("text/") || ct.contains("plain") {
        Some("Text")
    } else {
        None
    }
}

fn body_format_dropdown() -> Element<'static, Message> {
    // Array of all body formats
    let formats = [
        BodyFormat::None,
        BodyFormat::Json,
        BodyFormat::Xml,
        BodyFormat::Text,
    ];

    // Create buttons for each format using a loop
    let format_buttons: Vec<Element<'static, Message>> = formats
        .iter()
        .map(|format| {
            button(text(format.to_string()))
                .on_press(Message::BodyFormatChanged(format.clone()))
                .width(Length::Fixed(90.0))
                .style(dropdown_item_style())
                .into()
        })
        .collect();

    container(column(format_buttons))
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

fn post_script_tab<'a>(script_content: &'a text_editor::Content) -> Element<'a, Message> {
    // let script_content = config
    //     .post_request_script
    //     .as_deref()
    //     .unwrap_or("// No script defined");

    // let help_text = text("Post-request scripts run after receiving a response. Use 'pm' object to access response data and environment variables.")
    //     .size(12)
    //     .color(Color::from_rgb(0.6, 0.6, 0.6));

    // let example_text =
    //     text("Example: pm.environment.set('token', pm.response.json().access_token);")
    //         .size(11)
    //         .color(Color::from_rgb(0.5, 0.5, 0.5));

    // let script_display = container(scrollable(text(script_content).size(14)))
    //     .height(Length::Fill)
    //     .padding(10)
    //     .style(|theme: &Theme| container::Style {
    //         background: Some(Background::Color(Color::from_rgb(0.95, 0.95, 0.95))),
    //         border: Border {
    //             color: Color::from_rgb(0.8, 0.8, 0.8),
    //             width: 1.0,
    //             radius: 4.0.into(),
    //         },
    //         ..Default::default()
    //     });
    let script_editor_widget = text_editor(script_content)
        .highlight("javascript", highlighter::Theme::Base16Mocha)
        .on_action(Message::ScriptChanged)
        .placeholder("// Enter your post-request script here...")
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

    scrollable(script_editor_widget).height(Length::Fill).into()
}
