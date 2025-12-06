use crate::ui::undoable_input::UndoHistory;
use beam::ui::undoable_editor;
use beam::ui::undoable_input;
use iced::widget::{column, container, text};
use iced::{Element, Task};
use iced_widget::text_editor;

pub fn main() -> iced::Result {
    iced::application(
        || (UndoTest::default(), Task::none()),
        UndoTest::update,
        UndoTest::view,
    )
    .title("Undo/Redo Test")
    .run()
}

struct UndoTest {
    input: undoable_input::UndoableInput,
    editor: undoable_editor::UndoableEditor,
    url_undo_history: UndoHistory,
    body_undo_history: UndoHistory,
    content: text_editor::Content,
}

#[derive(Debug, Clone)]
enum Message {
    InputMessage(undoable_input::Message),
    EditorMessage(undoable_editor::EditorMessage),
}

impl Default for UndoTest {
    fn default() -> Self {
        let url_undo_history = UndoHistory::new();
        let body_undo_history = UndoHistory::new();

        Self {
            input: undoable_input::UndoableInput::new(String::new(), url_undo_history, "Type here...".to_string()),
            editor: undoable_editor::UndoableEditor::new(body_undo_history)
                .height(iced::Length::Fixed(200.0)),
            url_undo_history: UndoHistory::new(),
            body_undo_history: UndoHistory::new(),
            content: text_editor::Content::with_text("==="),
        }
    }
}

impl UndoTest {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputMessage(msg) => {
                if let Some(new_value) = self.input.update(msg) {
                    println!("Input changed to: {:?}", new_value);
                }
            }
            Message::EditorMessage(msg) => {
                if let Some(new_text) = self.editor.update(msg) {
                    println!("Editor changed to: {:?}", new_text);
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<Message> {
        let content = column![
            text("Undo/Redo Test").size(24),
            text("Text Input:").size(16),
            self.input.view().map(Message::InputMessage),
            text(format!("Current value: {:?}", self.input.value())).size(12),
            text("Text Editor:").size(16),
            self.editor.view(self.content).map(Message::EditorMessage),
            text(format!("Current text length: {}", self.editor.text().len())).size(12),
        ]
        .spacing(20)
        .padding(40);

        container(content).into()
    }
}
