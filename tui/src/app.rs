//! App state shared between the event loop, UI and API tasks.

use ratatui::widgets::ListState;

#[derive(Debug, Clone)]
pub struct Msg {
    pub role: String,
    pub content: String,
    pub tag: Option<String>,
}

pub struct App {
    pub base_url: String,
    pub node_label: String,
    pub backend_label: String,
    pub messages: Vec<Msg>,
    pub streaming: String,
    pub input: String,
    pub busy: bool,
    pub models: Vec<String>,
    pub model: Option<String>,
    pub selected_model: Option<usize>,
    pub picker_open: bool,
    pub system_prompt: String,
    pub list_state: ListState,
}

impl App {
    pub fn new(base_url: String, system_prompt: String) -> Self {
        Self {
            base_url,
            node_label: "connecting…".into(),
            backend_label: "-".into(),
            messages: Vec::new(),
            streaming: String::new(),
            input: String::new(),
            busy: false,
            models: Vec::new(),
            model: None,
            selected_model: None,
            picker_open: false,
            system_prompt,
            list_state: ListState::default(),
        }
    }

    pub fn history(&self) -> Vec<(String, String)> {
        self.messages
            .iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect()
    }

    pub fn scroll_to_bottom(&mut self) {
        let len = self.messages.len() + if self.streaming.is_empty() { 0 } else { 1 };
        if len > 0 {
            self.list_state.select(Some(len - 1));
        }
    }
}
