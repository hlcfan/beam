use beam::ui::undoable_editor;
use beam::ui::undoable_input;
use iced::widget::{column, container, text, text_editor};
use iced::{Element, Task};

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
    input_value: String,
    editor_content: text_editor::Content,
    input: undoable_input::UndoableInput,
    editor: undoable_editor::UndoableEditor,
}

#[derive(Debug, Clone)]
enum Message {
    InputMessage(undoable_input::Message),
    EditorMessage(undoable_editor::Message),
}

impl Default for UndoTest {
    fn default() -> Self {
        Self {
            input_value: "test".to_string(),
            editor_content: text_editor::Content::with_text("test"),
            input: undoable_input::UndoableInput::new(String::new(), "Type here...".to_string())
                .size(20.0)
                .padding(10.0),
            editor: undoable_editor::UndoableEditor::new("test".to_string()) // Init history
                .height(iced::Length::Fixed(200.0)),
        }
    }
}

impl UndoTest {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputMessage(msg) => {
                if let Some(new_value) = self.input.update(msg) {
                    println!("Input changed to: {:?}", new_value);
                    self.input_value = new_value;
                }
            }
            Message::EditorMessage(msg) => {
                if let Some(new_text) = self.editor.update(msg, &mut self.editor_content) {
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
            self.input
                .view(self.input_value.as_str())
                .map(Message::InputMessage),
            text(format!("Current value: {:?}", self.input.value())).size(12),
            text("Text Editor:").size(16),
            self.editor
                .view(&self.editor_content)
                .map(Message::EditorMessage),
            text(format!(
                "Current text length: {}",
                self.editor_content.text().len()
            ))
            .size(12),
        ]
        .spacing(20)
        .padding(40);

        container(content).into()
    }
}
