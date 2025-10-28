use crate::types::{HttpMethod, RenameTarget, RequestCollection, RequestConfig};
use crate::ui::{IconName, icon};
use iced::Task;
use iced::widget::button::Status;
use iced::widget::container::Style;
use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Background, Border, Color, Element, Length, Shadow, Vector};
use iced_aw::ContextMenu;
use log::{error, info};

#[derive(Debug, Clone)]
pub enum Action {
    UpdateCurrentCollection(RequestCollection),
    LoadRequestConfig(usize, usize),
    SaveRequestToCollection(RequestConfig),
    SaveNewCollection(RequestCollection),
    SendRequest(RequestConfig),
    DuplicateRequest(RequestConfig),
    DeleteRequest(usize, usize),
    RenameRequest(usize, usize, String),
    RenameCollection(usize, String),
    None,
}

#[derive(Debug, Clone)]
pub enum Message {
    CollectionToggled(usize),
    RequestSelected(usize, usize),

    ShowRenameModal(usize, usize), // (collection_index, request_index)
    // HideRenameModal,
    // RenameInputChanged(String),
    // ConfirmRename,
    AddHttpRequest(usize),
    DeleteFolder(usize),
    AddFolder(usize),
    RenameFolder(usize),

    // Request context menu actions
    SendRequestFromMenu(usize, usize),
    CopyRequestAsCurl(usize, usize),
    RenameRequest(usize, usize),
    DuplicateRequest(usize, usize),
    DeleteRequest(usize, usize),
}

#[derive(Debug, Clone)]
pub struct CollectionPanel {
    pub collections: Vec<RequestCollection>,

    // Double-click detection state
    pub last_click_time: Option<std::time::Instant>,
    pub last_click_target: Option<(usize, usize)>, // (collection_index, request_index)

    // Rename modal state
    pub show_rename_modal: bool,
    pub rename_input: String,
    pub rename_target: Option<RenameTarget>, // What is being renamed
}

impl CollectionPanel {
    pub fn new() -> Self {
        Self {
            collections: Vec::new(),
            last_click_time: None,
            last_click_target: None,

            show_rename_modal: false,
            rename_input: String::new(),
            rename_target: None,
        }
    }

    pub fn view(
        &mut self,
        collections: &[RequestCollection],
        last_opened_request: Option<(usize, usize)>,
    ) -> Element<'_, Message> {
        self.collections = collections.to_vec();
        let mut content = column![];

        for (collection_index, collection) in self.collections.iter().enumerate() {
            let collection_header = button(
                row![
                    icon(if collection.expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(12),
                    space().width(5),
                    text(&collection.name).size(14)
                ]
                .align_y(iced::Alignment::Center),
            )
            .on_press(Message::CollectionToggled(collection_index))
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
            .width(Length::Fill);

            // Wrap the collection header with ContextMenu
            let collection_with_context_menu = ContextMenu::new(collection_header, move || {
                container(
                    column![
                        button(text("Add Request"))
                            .on_press(Message::AddHttpRequest(collection_index))
                            .width(Length::Fill)
                            .style(|_theme, status| {
                                let base = button::Style::default();
                                match status {
                                    Status::Hovered => button::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.7, 0.7, 0.7,
                                        ))),
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
                            .style(|_theme, status| {
                                let base = button::Style::default();
                                match status {
                                    Status::Hovered => button::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.7, 0.7, 0.7,
                                        ))),
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
                            .style(|_theme, status| {
                                let base = button::Style::default();
                                match status {
                                    Status::Hovered => button::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.7, 0.7, 0.7,
                                        ))),
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
                            .style(|_theme, status| {
                                let base = button::Style::default();
                                match status {
                                    Status::Hovered => button::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.7, 0.7, 0.7,
                                        ))),
                                        ..base
                                    },
                                    _ => button::Style {
                                        background: Some(Background::Color(Color::TRANSPARENT)),
                                        ..base
                                    },
                                }
                            }),
                    ]
                    .spacing(2),
                )
                .width(Length::Fixed(150.0))
                .style(|_theme| Style {
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
            });

            content = content.push(collection_with_context_menu);

            if collection.expanded {
                for (request_index, request) in collection.requests.iter().enumerate() {
                    let is_selected =
                        last_opened_request == Some((collection_index, request_index));

                    let request_button = button(
                        row![
                            space().width(20),
                            method_badge(&request.method),
                            space().width(8),
                            text(&request.name).size(12)
                        ]
                        .align_y(iced::Alignment::Center),
                    )
                    .on_press(Message::RequestSelected(collection_index, request_index))
                    .style(move |_theme, status| {
                        let base = button::Style::default();

                        match status {
                            Status::Pressed => {
                                if is_selected {
                                    button::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.78, 0.82, 0.996,
                                        ))), // #c7d2fe
                                        ..base
                                    }
                                } else {
                                    button::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.9, 0.9, 0.9,
                                        ))),
                                        ..base
                                    }
                                }
                            }
                            Status::Hovered => {
                                if is_selected {
                                    button::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.78, 0.82, 0.996,
                                        ))), // #c7d2fe
                                        ..base
                                    }
                                } else {
                                    button::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.95, 0.95, 0.95,
                                        ))),
                                        ..base
                                    }
                                }
                            }
                            _ => {
                                if is_selected {
                                    button::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.78, 0.82, 0.996,
                                        ))), // #c7d2fe
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
                    let request_with_context_menu = ContextMenu::new(request_button, move || {
                        container(
                            column![
                                button(text("Send Request"))
                                    .on_press(Message::SendRequestFromMenu(
                                        collection_index,
                                        request_index
                                    ))
                                    .width(Length::Fill)
                                    .style(|_theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(
                                                    Color::from_rgb(0.7, 0.7, 0.7),
                                                )),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(
                                                    Color::TRANSPARENT,
                                                )),
                                                ..base
                                            },
                                        }
                                    }),
                                button(text("Copy as cURL"))
                                    .on_press(Message::CopyRequestAsCurl(
                                        collection_index,
                                        request_index
                                    ))
                                    .width(Length::Fill)
                                    .style(|_theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(
                                                    Color::from_rgb(0.7, 0.7, 0.7),
                                                )),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(
                                                    Color::TRANSPARENT,
                                                )),
                                                ..base
                                            },
                                        }
                                    }),
                                button(text("Rename"))
                                    .on_press(Message::RenameRequest(
                                        collection_index,
                                        request_index
                                    ))
                                    .width(Length::Fill)
                                    .style(|_theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(
                                                    Color::from_rgb(0.7, 0.7, 0.7),
                                                )),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(
                                                    Color::TRANSPARENT,
                                                )),
                                                ..base
                                            },
                                        }
                                    }),
                                button(text("Duplicate"))
                                    .on_press(Message::DuplicateRequest(
                                        collection_index,
                                        request_index
                                    ))
                                    .width(Length::Fill)
                                    .style(|_theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(
                                                    Color::from_rgb(0.7, 0.7, 0.7),
                                                )),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(
                                                    Color::TRANSPARENT,
                                                )),
                                                ..base
                                            },
                                        }
                                    }),
                                button(text("Delete"))
                                    .on_press(Message::DeleteRequest(
                                        collection_index,
                                        request_index
                                    ))
                                    .width(Length::Fill)
                                    .style(|__theme, status| {
                                        let base = button::Style::default();
                                        match status {
                                            Status::Hovered => button::Style {
                                                background: Some(Background::Color(
                                                    Color::from_rgb(0.7, 0.7, 0.7),
                                                )),
                                                ..base
                                            },
                                            _ => button::Style {
                                                background: Some(Background::Color(
                                                    Color::TRANSPARENT,
                                                )),
                                                ..base
                                            },
                                        }
                                    }),
                            ]
                            .spacing(2),
                        )
                        .width(Length::Fixed(150.0))
                        .style(|_theme| Style {
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
                    });

                    content = content.push(request_with_context_menu);
                }
            }
        }

        scrollable(content.spacing(2).padding(10))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::CollectionToggled(index) => {
                if let Some(collection) = self.collections.get_mut(index) {
                    collection.expanded = !collection.expanded;
                    Action::UpdateCurrentCollection(collection.clone())
                } else {
                    Action::None
                }
            }
            Message::RequestSelected(collection_index, request_index) => {
                info!("===select request1: {:?}", collection_index);
                info!("===collections: {:?}", self.collections);
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        let now = std::time::Instant::now();
                        let current_target = (collection_index, request_index);

                        // Check for double-click (within 500ms and same target)
                        let is_double_click = if let (Some(last_time), Some(last_target)) =
                            (self.last_click_time, self.last_click_target)
                        {
                            last_target == current_target
                                && now.duration_since(last_time).as_millis() < 500
                        } else {
                            false
                        };

                        // Update click tracking
                        self.last_click_time = Some(now);
                        self.last_click_target = Some(current_target);

                        if is_double_click {
                            info!("===select request2");
                            self.show_rename_modal = true;
                            self.rename_target =
                                Some(RenameTarget::Request(collection_index, request_index));
                            self.rename_input = request.name.clone();

                            Action::None
                        } else {
                            info!("===select request3");
                            return Action::LoadRequestConfig(collection_index, request_index);
                        }
                    } else {
                        info!("===select request4");
                        Action::None
                    }
                } else {
                    info!("===select request5");
                    Action::None
                }
            }
            Message::AddHttpRequest(collection_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    let mut new_request = RequestConfig::default();
                    new_request.collection_index = collection_index as u32;

                    if let Some(collection) = self.collections.get_mut(collection_index) {
                        let len = collection.requests.len();
                        new_request.name = format!("New Request {}", collection.requests.len() + 1);
                        new_request.request_index = len as u32;
                    }

                    Action::SaveRequestToCollection(new_request)
                } else {
                    Action::None
                }
            }
            Message::DeleteFolder(collection_index) => {
                if collection_index < self.collections.len() {
                    self.collections.remove(collection_index);
                }

                // After deleting a folder, we don't need to save anything since the collection is removed
                Action::None
            }
            Message::AddFolder(_collection_index) => {
                let new_collection = RequestCollection {
                    name: format!("New Collection {}", self.collections.len() + 1),
                    requests: vec![],
                    expanded: true,
                };

                Action::SaveNewCollection(new_collection)
            }
            Message::RenameFolder(collection_index) => {
                // Show the rename modal for the folder
                if let Some(collection) = self.collections.get(collection_index) {
                    self.show_rename_modal = true;
                    self.rename_input = collection.name.clone();
                    self.rename_target = Some(RenameTarget::Folder(collection_index));
                }
                Action::None
            }
            Message::SendRequestFromMenu(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        return Action::SendRequest(request.clone());
                    }
                }

                Action::None
            }
            Message::CopyRequestAsCurl(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        let curl_command = crate::http::generate_curl_command(request);
                        // TODO: In a real app, you'd copy to clipboard here
                        info!("Curl command: {}", curl_command);
                    }
                }
                Action::None
            }
            Message::RenameRequest(collection_index, request_index) => {
                // Show the rename modal with the current request name
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        self.show_rename_modal = true;
                        self.rename_input = request.name.clone();
                        self.rename_target =
                            Some(RenameTarget::Request(collection_index, request_index));
                    }
                }

                Action::None
            }
            Message::DuplicateRequest(collection_index, request_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    if let Some(request) = collection.requests.get(request_index).cloned() {
                        let mut new_request = request;
                        new_request.name = format!("{} (Copy)", new_request.name);
                        new_request.collection_index = collection_index as u32;
                        new_request.request_index = request_index as u32;

                        Action::DuplicateRequest(new_request)
                    } else {
                        Action::None
                    }
                } else {
                    Action::None
                }
            }
            Message::DeleteRequest(collection_index, request_index) => {
                Action::DeleteRequest(collection_index, request_index)
            }
            Message::ShowRenameModal(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        self.show_rename_modal = true;
                        self.rename_input = request.name.clone();
                        self.rename_target =
                            Some(RenameTarget::Request(collection_index, request_index));
                    }
                }
                Action::None
            }
        }
    }
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

    container(text(method_text).size(10).color(text_color))
        .width(Length::Fixed(32.0))
        .align_x(iced::alignment::Horizontal::Right)
        .style(move |_theme| Style {
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
