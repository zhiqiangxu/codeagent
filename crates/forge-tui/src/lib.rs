pub mod input;
pub mod display;
pub mod prompter;

pub use input::InputBuffer;
pub use display::DisplayMessage;
pub use prompter::TuiRuntimePrompter;

/// TUI 应用状态，状态与渲染分离。
pub struct AppState {
    pub input: InputBuffer,
    pub messages: Vec<DisplayMessage>,
    pub scroll: usize,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            input: InputBuffer::new(),
            messages: Vec::new(),
            scroll: 0,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
