mod types;
mod http;
mod ui;

use types::*;
use http::*;
use ui::*;

use iced::widget::pane_grid::{self, PaneGrid, Axis};
use iced::widget::{
    button, column, container, row, text, text_input, text_editor, pick_list, scrollable,
    mouse_area, stack
};
use iced::{Element, Fill, Length, Size, Theme, Color, Task};
use iced_aw::ContextMenu;

pub fn main() -> iced::Result {
    iced::application(
            |_state: &PostmanApp| String::from("Beam"),
            PostmanApp::update,
            PostmanApp::view,
        )
        .window_size(Size::new(1200.0, 800.0))
        .run()
}

impl Default for PostmanApp {
    fn default() -> Self {
        let (mut panes, collections_pane) = pane_grid::State::new(PaneContent::Collections);

        // Split vertically to create request config pane (middle panel)
        let (request_pane, first_split) = panes.split(
            Axis::Vertical,
            collections_pane,
            PaneContent::RequestConfig,
        ).unwrap();

        // Split vertically again to create response pane (right panel)
        let (_, second_split) = panes.split(
            Axis::Vertical,
            request_pane,
            PaneContent::Response,
        ).unwrap();

        // Set three-panel horizontal layout ratios
        // Collections: 25%, Request Config: 40%, Response: 35%
        panes.resize(first_split, 0.25);
        panes.resize(second_split, 0.533); // 40/(40+35) = 0.533

        let collections = vec![
            RequestCollection {
                name: "My Collection".to_string(),
                requests: vec![
                    SavedRequest {
                        name: "Get Users".to_string(),
                        method: HttpMethod::GET,
                        url: "https://jsonplaceholder.typicode.com/users".to_string(),
                    },
                    SavedRequest {
                        name: "Create User".to_string(),
                        method: HttpMethod::POST,
                        url: "https://jsonplaceholder.typicode.com/users".to_string(),
                    },
                ],
                expanded: true,
            },
            RequestCollection {
                name: "API Tests".to_string(),
                requests: vec![
                    SavedRequest {
                        name: "Health Check".to_string(),
                        method: HttpMethod::GET,
                        url: "https://httpbin.org/status/200".to_string(),
                    },
                ],
                expanded: false,
            },
        ];

        Self {
            panes,
            collections,
            current_request: RequestConfig {
                method: HttpMethod::GET,
                url: String::new(),
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                params: vec![],
                body: text_editor::Content::new(),
                content_type: "application/json".to_string(),
                auth_type: AuthType::None,
                selected_tab: RequestTab::Body,
            },
            response: None,
            response_body_content: text_editor::Content::new(),
            selected_response_tab: ResponseTab::Body,

            is_loading: false,
        }
    }
}

impl PostmanApp {
    fn create_response_content(body: &str) -> text_editor::Content {
        text_editor::Content::with_text(body)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PaneResized(event) => {
                self.panes.resize(event.split, event.ratio);
                Task::none()
            }
            Message::UrlChanged(url) => {
                self.current_request.url = url;
                Task::none()
            }
            Message::MethodChanged(method) => {
                self.current_request.method = method;
                Task::none()
            }
            Message::SendRequest => {
                self.is_loading = true;
                let config = self.current_request.clone();
                Task::perform(send_request(config), Message::RequestCompleted)
            }
            Message::CancelRequest => {
                self.is_loading = false;
                Task::none()
            }
            Message::RequestCompleted(result) => {
                self.is_loading = false;
                match result {
                    Ok(response) => {
                        self.response_body_content = Self::create_response_content(&response.body);
                        self.response = Some(response);
                    }
                    Err(error) => {
                        self.response_body_content = Self::create_response_content(&error);
                        self.response = Some(ResponseData {
                            status: 0,
                            status_text: "Error".to_string(),
                            headers: vec![],
                            body: error,
                            size: 0,
                            time: 0,
                        });
                    }
                }
                Task::none()
            }
            Message::CollectionToggled(index) => {
                if let Some(collection) = self.collections.get_mut(index) {
                    collection.expanded = !collection.expanded;
                }
                Task::none()
            }
            Message::RequestSelected(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        self.current_request.method = request.method.clone();
                        self.current_request.url = request.url.clone();
                    }
                }
                Task::none()
            }
            Message::TabSelected(tab) => {
                self.current_request.selected_tab = tab;
                Task::none()
            }
            Message::ResponseTabSelected(tab) => {
                self.selected_response_tab = tab;
                Task::none()
            }
            Message::HeaderKeyChanged(index, key) => {
                if let Some(header) = self.current_request.headers.get_mut(index) {
                    header.0 = key;
                }
                Task::none()
            }
            Message::HeaderValueChanged(index, value) => {
                if let Some(header) = self.current_request.headers.get_mut(index) {
                    header.1 = value;
                }
                Task::none()
            }
            Message::AddHeader => {
                self.current_request.headers.push((String::new(), String::new()));
                Task::none()
            }
            Message::RemoveHeader(index) => {
                if index < self.current_request.headers.len() {
                    self.current_request.headers.remove(index);
                }
                Task::none()
            }
            Message::ParamKeyChanged(index, key) => {
                if let Some(param) = self.current_request.params.get_mut(index) {
                    param.0 = key;
                }
                Task::none()
            }
            Message::ParamValueChanged(index, value) => {
                if let Some(param) = self.current_request.params.get_mut(index) {
                    param.1 = value;
                }
                Task::none()
            }
            Message::AddParam => {
                self.current_request.params.push((String::new(), String::new()));
                Task::none()
            }
            Message::RemoveParam(index) => {
                if index < self.current_request.params.len() {
                    self.current_request.params.remove(index);
                }
                Task::none()
            }
            Message::BodyChanged(action) => {
                self.current_request.body.perform(action);
                Task::none()
            }
            Message::ResponseBodyAction(action) => {
                self.response_body_content.perform(action);
                Task::none()
            }
            Message::AuthTypeChanged(auth_type) => {
                self.current_request.auth_type = auth_type;
                Task::none()
            }

            Message::AddHttpRequest(collection_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    collection.requests.push(SavedRequest {
                        name: format!("New Request {}", collection.requests.len() + 1),
                        method: HttpMethod::GET,
                        url: String::new(),
                    });
                }

                Task::none()
            }
            Message::DeleteFolder(collection_index) => {
                if collection_index < self.collections.len() {
                    self.collections.remove(collection_index);
                }

                Task::none()
            }
            Message::AddFolder(_collection_index) => {
                self.collections.push(RequestCollection {
                    name: format!("New Collection {}", self.collections.len() + 1),
                    requests: vec![],
                    expanded: true,
                });

                Task::none()
            }
            Message::RenameFolder(collection_index) => {
                // For now, just add a number to the name as a placeholder
                // In a real app, this would open a dialog or text input
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    collection.name = format!("{} (Renamed)", collection.name);
                }

                Task::none()
            }
            Message::SendRequestFromMenu(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        self.current_request.method = request.method.clone();
                        self.current_request.url = request.url.clone();
                        self.is_loading = true;
                        let config = self.current_request.clone();

                        return Task::perform(send_request(config), Message::RequestCompleted);
                    }
                }

                Task::none()
            }
            Message::CopyRequestAsCurl(collection_index, request_index) => {
                if let Some(collection) = self.collections.get(collection_index) {
                    if let Some(request) = collection.requests.get(request_index) {
                        let mut temp_config = self.current_request.clone();
                        temp_config.method = request.method.clone();
                        temp_config.url = request.url.clone();
                        let curl_command = generate_curl_command(&temp_config);
                        // In a real app, you'd copy to clipboard here
                        println!("Curl command: {}", curl_command);
                    }
                }
                Task::none()
            }
            Message::RenameRequest(_collection_index, _request_index) => {
                // TODO: Implement rename functionality
                Task::none()
            }
            Message::DuplicateRequest(collection_index, request_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    if let Some(request) = collection.requests.get(request_index).cloned() {
                        let mut new_request = request;
                        new_request.name = format!("{} (Copy)", new_request.name);
                        collection.requests.push(new_request);
                    }
                }
                Task::none()
            }
            Message::DeleteRequest(collection_index, request_index) => {
                if let Some(collection) = self.collections.get_mut(collection_index) {
                    if request_index < collection.requests.len() {
                        collection.requests.remove(request_index);
                    }
                }
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let pane_grid = PaneGrid::new(&self.panes, |_id, pane, _is_maximized| {
            let content = match pane {
                PaneContent::Collections => self.collections_view(),
                PaneContent::RequestConfig => self.request_config_view(),
                PaneContent::Response => self.response_view(),
            };

            container(content)
                .width(Fill)
                .height(Fill)
                .padding(0)
                .into()
        })
        .on_resize(10, Message::PaneResized)
        .spacing(1)
        .style(|_theme| pane_grid::Style {
            hovered_region: pane_grid::Highlight {
                background: iced::Background::Color(iced::Color::TRANSPARENT),
                border: iced::Border::default(),
            },
            hovered_split: pane_grid::Line {
                color: iced::Color::from_rgb(0.7, 0.7, 0.7), // Gray
                width: 1.0,
            },
            picked_split: pane_grid::Line {
                color: iced::Color::from_rgb(0.7, 0.7, 0.7), // Gray
                width: 1.0,
            },
        });

        pane_grid.into()
    }

    fn collections_view(&self) -> Element<'_, Message> {
        collections_panel(&self.collections)
    }

    fn request_config_view(&self) -> Element<'_, Message> {
        request_panel(&self.current_request, self.is_loading)
    }

    fn response_view(&self) -> Element<'_, Message> {
        response_panel(
            &self.response,
            &self.response_body_content,
            self.selected_response_tab.clone(),
            self.is_loading,
        )
    }
}
