//! App state shared between the event loop, UI and API tasks.

use crate::model_browser::{BrowserAction, ModelBrowser};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use localhostai_core::model_hub::ModelSource;
use localhostai_core::SplitPlan;
use ratatui::widgets::ListState;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

pub const CONTEXT_CHOICES: &[u32] = &[1024, 2048, 4096, 8192, 16384];

/// A device in the compute pool, as reported by the node.
#[derive(Debug, Clone, Default)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub ram_mb: u64,
    pub available_mb: u64,
    pub budget_mb: u64,
    pub cores: u32,
    pub threads: u32,
    pub rpc: Option<String>,
    pub is_self: bool,
    pub enabled: bool,
    pub stale: bool,
    pub reachable: bool,
    pub usable: bool,
    pub in_use: bool,
    pub reason: Option<String>,
    pub heartbeat_seconds: u64,
    pub assigned_layers: u32,
    pub assigned_memory_mb: u64,
    pub swap_used_mb: u64,
    pub pressure_full_avg10: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct CatalogRow {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub bytes: u64,
    pub license: String,
    pub installed: bool,
    pub recommended: bool,
    pub custom: bool,
}

impl CatalogRow {
    pub fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            id: value["id"].as_str()?.to_string(),
            display_name: value["display_name"].as_str()?.to_string(),
            description: value["description"].as_str().unwrap_or("").to_string(),
            bytes: value["bytes"].as_u64().unwrap_or(0),
            license: value["license"].as_str().unwrap_or("").to_string(),
            installed: value["installed"].as_bool().unwrap_or(false),
            recommended: value["recommended"].as_bool().unwrap_or(false),
            custom: value["custom"].as_bool().unwrap_or(false),
        })
    }
}

impl DeviceRow {
    pub fn from_json(v: &Value) -> Self {
        let is_self = v["self"].as_bool().unwrap_or(false);
        let reachable = v["reachable"].as_bool().unwrap_or(false);
        let budget_mb = v["budget_mb"].as_u64().unwrap_or(0);
        let in_use = v["in_use"].as_bool().unwrap_or(false);
        let enabled = v["enabled"].as_bool().unwrap_or(true);
        Self {
            id: v["id"].as_str().unwrap_or("").to_string(),
            name: v["name"].as_str().unwrap_or("?").to_string(),
            kind: v["kind"].as_str().unwrap_or("?").to_string(),
            ram_mb: v["ram_mb"].as_u64().unwrap_or(0),
            available_mb: v["available_mb"].as_u64().unwrap_or(0),
            budget_mb,
            cores: v["cores"].as_u64().unwrap_or(0) as u32,
            threads: v["threads"].as_u64().unwrap_or(0) as u32,
            rpc: v["rpc"].as_str().map(String::from),
            is_self,
            enabled,
            stale: v["stale"].as_bool().unwrap_or(false),
            reachable,
            usable: enabled
                && (budget_mb > 0 || in_use)
                && v["usable"].as_bool().unwrap_or(is_self || reachable),
            in_use,
            reason: v["reason"].as_str().map(String::from),
            heartbeat_seconds: v["heartbeat_seconds"].as_u64().unwrap_or(0),
            assigned_layers: v["assigned_layers"].as_u64().unwrap_or(0) as u32,
            assigned_memory_mb: v["assigned_memory_mb"].as_u64().unwrap_or(0),
            swap_used_mb: v["memory"]["swap_used_mb"].as_u64().unwrap_or(0),
            pressure_full_avg10: v["memory"]["pressure_full_avg10"].as_f64(),
        }
    }
}

/// Which overlay is open, if any.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Overlay {
    #[default]
    None,
    ModelPicker,
    ModelBrowser,
    Devices,
    Pairing,
    Activity,
    Help,
}

#[derive(Debug, Clone)]
pub struct Msg {
    pub role: String,
    pub content: String,
    pub tag: Option<String>,
}

pub struct App {
    pub browser: ModelBrowser,
    pub base_url: String,
    pub node_label: String,
    pub backend_label: String,
    pub messages: Vec<Msg>,
    pub streaming: String,
    pub input: String,
    pub input_cursor: usize,
    pub busy: bool,
    pub models: Vec<String>,
    pub model: Option<String>,
    pub selected_model: Option<usize>,
    pub context_tokens: u32,
    pub preview: Option<SplitPlan>,
    pub preview_pending: bool,
    pub preview_error: Option<String>,
    pub preview_request_id: u64,
    pub servable_models: Vec<String>,
    pub plannable_models: Vec<String>,
    pub cluster_starting: bool,
    pub setup_busy: bool,
    pub setup_phase: String,
    pub setup_downloaded: u64,
    pub setup_total: u64,
    pub setup_model: Option<String>,
    pub setup_job: Option<String>,
    pub setup_message: Option<String>,
    pub runtime_ready: bool,
    pub catalog: Vec<CatalogRow>,
    pub system_prompt: String,
    pub list_state: ListState,
    pub chat_follow: bool,
    pub chat_rows: usize,
    pub chat_page_rows: usize,

    // --- product surface ---------------------------------------------------
    /// Which overlay is showing.
    pub overlay: Overlay,
    /// Devices in the compute pool.
    pub devices: Vec<DeviceRow>,
    pub selected_device: Option<usize>,
    /// One-line cluster state for the status bar ("local" / "distributed 19/36L").
    pub cluster_label: String,
    pub cluster_state: String,
    pub cluster_model: Option<String>,
    pub cluster_assignments: Vec<localhostai_core::plan::SplitAssignment>,
    pub cluster_error: Option<String>,
    pub cluster_log: Option<String>,
    /// Pairing URI rendered as a QR in the Pairing overlay.
    pub qr_lines: Vec<String>,
    pub pair_uri: String,
    /// Transient message shown under the status bar.
    pub notice: Option<String>,
    pub activity_message: String,
    pub activity_error: bool,
    pub operation_busy: bool,
    pub operation_id: u64,
}

impl App {
    pub fn new(base_url: String, system_prompt: String) -> Self {
        Self {
            browser: ModelBrowser {
                editing: true,
                ..Default::default()
            },
            base_url,
            node_label: "connecting…".into(),
            backend_label: "-".into(),
            messages: Vec::new(),
            streaming: String::new(),
            input: String::new(),
            input_cursor: 0,
            busy: false,
            models: Vec::new(),
            model: None,
            selected_model: None,
            context_tokens: 4096,
            preview: None,
            preview_pending: false,
            preview_error: None,
            preview_request_id: 0,
            servable_models: Vec::new(),
            plannable_models: Vec::new(),
            cluster_starting: false,
            setup_busy: false,
            setup_phase: "idle".into(),
            setup_downloaded: 0,
            setup_total: 0,
            setup_model: None,
            setup_job: None,
            setup_message: None,
            runtime_ready: false,
            catalog: Vec::new(),
            system_prompt,
            list_state: ListState::default(),
            chat_follow: true,
            chat_rows: 0,
            chat_page_rows: 1,
            overlay: Overlay::None,
            devices: Vec::new(),
            selected_device: None,
            cluster_label: "local".into(),
            cluster_state: "stopped".into(),
            cluster_model: None,
            cluster_assignments: Vec::new(),
            cluster_error: None,
            cluster_log: None,
            qr_lines: Vec::new(),
            pair_uri: String::new(),
            notice: None,
            activity_message: "No operations yet".into(),
            activity_error: false,
            operation_busy: false,
            operation_id: 0,
        }
    }

    pub fn history(&self) -> Vec<(String, String)> {
        let budget = (self.context_tokens as usize * 3 * 3 / 4).clamp(1024, 3 * 1024 * 1024);
        let mut used: usize = 0;
        let mut history = Vec::new();
        for message in self.messages.iter().rev() {
            let cost = message.content.len().saturating_add(32);
            if used.saturating_add(cost) > budget {
                if history.is_empty() {
                    history.push((
                        message.role.clone(),
                        message.content.chars().take(budget).collect(),
                    ));
                }
                break;
            }
            used = used.saturating_add(cost);
            history.push((message.role.clone(), message.content.clone()));
        }
        history.reverse();
        history
    }

    /// Total memory the pool can lend, counting only usable devices.
    pub fn pool_mb(&self) -> u64 {
        self.devices
            .iter()
            .filter(|d| d.usable)
            .map(|d| d.budget_mb)
            .fold(0, u64::saturating_add)
    }

    pub fn scroll_to_bottom(&mut self) {
        self.chat_follow = true;
        if self.chat_rows > 0 {
            self.list_state.select(Some(self.chat_rows - 1));
        }
    }

    fn selected_model_id(&self) -> Option<String> {
        self.selected_model
            .and_then(|index| self.models.get(index).cloned())
    }

    pub fn request_preview(&mut self) -> Action {
        let Some(model) = self.selected_model_id() else {
            return Action::None;
        };
        self.preview_request_id = self.preview_request_id.wrapping_add(1);
        self.preview = None;
        self.preview_error = None;
        if self
            .catalog_model(&model)
            .is_some_and(|entry| entry.custom && !entry.installed)
        {
            self.preview_pending = false;
            self.preview_error =
                Some("Download this GGUF before planning its layers (I to download/resume)".into());
            return Action::None;
        }
        self.preview_pending = true;
        Action::Preview {
            request_id: self.preview_request_id,
            model,
            ctx: self.context_tokens,
        }
    }

    fn preview_fits(&mut self, model: &str) -> bool {
        let Some(plan) = self
            .preview
            .as_ref()
            .filter(|plan| plan.model == model && plan.ctx == self.context_tokens)
        else {
            // The load task repeats the same authoritative plan request before
            // any download, so fast Enter presses remain safe.
            return true;
        };
        if !plan.fits() {
            self.notice = Some(plan.summary.clone());
            return false;
        }
        true
    }

    fn move_input_left(&mut self) {
        self.input_cursor = self.input[..self.input_cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0);
    }

    fn move_input_right(&mut self) {
        if let Some(grapheme) = self.input[self.input_cursor..].graphemes(true).next() {
            self.input_cursor += grapheme.len();
        }
    }

    fn delete_input_before_cursor(&mut self) {
        let end = self.input_cursor;
        self.move_input_left();
        if self.input_cursor < end {
            self.input.drain(self.input_cursor..end);
        }
    }

    fn delete_input_at_cursor(&mut self) {
        let Some(grapheme) = self.input[self.input_cursor..].graphemes(true).next() else {
            return;
        };
        self.input
            .drain(self.input_cursor..self.input_cursor + grapheme.len());
    }

    fn delete_input_word(&mut self) {
        while self.input_cursor > 0 {
            let before = &self.input[..self.input_cursor];
            let Some(grapheme) = before.graphemes(true).next_back() else {
                break;
            };
            if !grapheme.chars().all(char::is_whitespace) {
                break;
            }
            self.delete_input_before_cursor();
        }
        while self.input_cursor > 0 {
            let before = &self.input[..self.input_cursor];
            let Some(grapheme) = before.graphemes(true).next_back() else {
                break;
            };
            if grapheme.chars().all(char::is_whitespace) {
                break;
            }
            self.delete_input_before_cursor();
        }
    }

    pub fn paste(&mut self, text: &str) {
        if self.overlay == Overlay::ModelBrowser {
            self.browser.paste(text);
            return;
        }
        if self.overlay != Overlay::None || self.busy {
            return;
        }
        let text = text.replace(['\r', '\n'], " ");
        self.input.insert_str(self.input_cursor, &text);
        self.input_cursor += text.len();
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.overlay == Overlay::ModelBrowser && !(control && key.code == KeyCode::Char('c')) {
            if key.code == KeyCode::Esc {
                self.browser.request_id = self.browser.request_id.wrapping_add(1);
                self.browser.pending = false;
                self.overlay = Overlay::ModelPicker;
                return self.request_preview();
            }
            return match self.browser.key(
                key,
                !self.setup_busy && !self.operation_busy && !self.cluster_starting,
            ) {
                BrowserAction::None => Action::None,
                BrowserAction::Search { request_id, query } => Action::Browse { request_id, query },
                BrowserAction::Download { source, name } => {
                    self.overlay = Overlay::Activity;
                    Action::Download { source, name }
                }
            };
        }
        match key.code {
            KeyCode::Char('c') if control => return Action::Quit,
            KeyCode::Esc if self.overlay != Overlay::None => self.overlay = Overlay::None,
            KeyCode::Esc => return Action::Quit,
            KeyCode::F(1) => self.overlay = Overlay::Help,
            KeyCode::Char('a') if control && self.overlay == Overlay::None => {
                self.overlay = Overlay::Activity
            }
            KeyCode::Char('x')
                if control && matches!(self.overlay, Overlay::None | Overlay::Activity) =>
            {
                if self.busy {
                    return Action::CancelChat;
                }
                if self.setup_busy && self.setup_job.is_some() {
                    return Action::CancelInstall(self.setup_job.clone().unwrap());
                }
                if (self.cluster_starting && !self.setup_busy)
                    || matches!(self.cluster_state.as_str(), "starting" | "running")
                {
                    return Action::StopCluster;
                }
            }
            KeyCode::Char('d') if control && self.overlay == Overlay::None => {
                self.selected_device = (!self.devices.is_empty()).then_some(0);
                self.overlay = Overlay::Devices;
                return Action::Refresh;
            }
            KeyCode::Char('p') if control && self.overlay == Overlay::None => {
                self.overlay = Overlay::Pairing;
                return Action::Pair;
            }
            KeyCode::Up if self.overlay == Overlay::Devices => {
                self.selected_device = self.selected_device.map(|index| index.saturating_sub(1));
            }
            KeyCode::Down if self.overlay == Overlay::Devices => {
                self.selected_device = self
                    .selected_device
                    .map(|index| (index + 1).min(self.devices.len().saturating_sub(1)));
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.overlay == Overlay::Devices => {
                if let Some(device) = self
                    .selected_device
                    .and_then(|index| self.devices.get(index))
                {
                    return Action::ToggleDevice {
                        id: device.id.clone(),
                        enabled: !device.enabled,
                    };
                }
            }
            KeyCode::Up if self.overlay == Overlay::ModelPicker => {
                self.selected_model = self.selected_model.map(|i| i.saturating_sub(1));
                return self.request_preview();
            }
            KeyCode::Char('b') | KeyCode::Char('B') if self.overlay == Overlay::ModelPicker => {
                self.overlay = Overlay::ModelBrowser;
                self.browser.editing = true;
            }
            KeyCode::Down if self.overlay == Overlay::ModelPicker => {
                self.selected_model = self
                    .selected_model
                    .map(|i| (i + 1).min(self.models.len().saturating_sub(1)));
                return self.request_preview();
            }
            KeyCode::Left | KeyCode::Right if self.overlay == Overlay::ModelPicker => {
                let index = CONTEXT_CHOICES
                    .iter()
                    .position(|ctx| *ctx == self.context_tokens)
                    .unwrap_or(2);
                let next = if key.code == KeyCode::Left {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(CONTEXT_CHOICES.len() - 1)
                };
                self.context_tokens = CONTEXT_CHOICES[next];
                return self.request_preview();
            }
            KeyCode::Char('i') | KeyCode::Char('I') if self.overlay == Overlay::ModelPicker => {
                if self.setup_busy || self.operation_busy {
                    return Action::None;
                }
                if let Some(model) = self.selected_model_id() {
                    if self.catalog_model(&model).is_none() {
                        self.notice = Some("This GGUF is already a local file".into());
                        return Action::None;
                    }
                    self.overlay = Overlay::Activity;
                    return Action::InstallOnly(model);
                }
            }
            KeyCode::Delete if self.overlay == Overlay::ModelPicker => {
                if self.setup_busy || self.operation_busy {
                    return Action::None;
                }
                if let Some(model) = self.selected_model_id() {
                    if self.model.as_deref() == Some(&model)
                        || self.cluster_model.as_deref() == Some(&model)
                    {
                        self.notice = Some("Stop the active model before deleting it".into());
                    } else if self
                        .catalog_model(&model)
                        .is_some_and(|entry| entry.installed)
                    {
                        self.overlay = Overlay::Activity;
                        return Action::DeleteModel(model);
                    }
                }
            }
            KeyCode::Char('u') | KeyCode::Char('U') if self.overlay == Overlay::ModelPicker => {
                if !self.setup_busy && !self.operation_busy && self.cluster_state != "running" {
                    self.overlay = Overlay::Activity;
                    return Action::ResetRuntime;
                }
            }
            KeyCode::Enter | KeyCode::Char('D') if self.overlay == Overlay::ModelPicker => {
                if self.cluster_starting || self.setup_busy {
                    return Action::None;
                }
                let chosen = self
                    .selected_model
                    .and_then(|i| self.models.get(i).cloned());
                self.overlay = Overlay::None;
                if let Some(model) = chosen {
                    if self
                        .catalog_model(&model)
                        .is_some_and(|entry| entry.custom && !entry.installed)
                    {
                        self.overlay = Overlay::Activity;
                        return Action::InstallOnly(model);
                    }
                    if key.code == KeyCode::Char('D') || !self.servable_models.contains(&model) {
                        if self.preview_fits(&model) {
                            self.overlay = Overlay::Activity;
                            return Action::Start {
                                model,
                                ctx: self.context_tokens,
                            };
                        }
                        self.overlay = Overlay::ModelPicker;
                        return Action::None;
                    }
                    self.model = Some(model);
                }
            }
            KeyCode::Up if self.overlay == Overlay::None && !self.busy => {
                self.selected_model = self
                    .model
                    .as_ref()
                    .and_then(|m| self.models.iter().position(|s| s == m))
                    .or_else(|| (!self.models.is_empty()).then_some(0));
                self.overlay = Overlay::ModelPicker;
                return self.request_preview();
            }
            KeyCode::PageUp if self.overlay == Overlay::None => {
                let current = self
                    .list_state
                    .selected()
                    .unwrap_or_else(|| self.chat_rows.saturating_sub(1));
                self.list_state
                    .select(Some(current.saturating_sub(self.chat_page_rows.max(1))));
                self.chat_follow = false;
            }
            KeyCode::PageDown if self.overlay == Overlay::None => {
                let last = self.chat_rows.saturating_sub(1);
                let current = self.list_state.selected().unwrap_or(last);
                let next = current.saturating_add(self.chat_page_rows.max(1)).min(last);
                self.list_state.select(Some(next));
                self.chat_follow = next == last;
            }
            KeyCode::Left if self.overlay == Overlay::None => self.move_input_left(),
            KeyCode::Right if self.overlay == Overlay::None => self.move_input_right(),
            KeyCode::Home if self.overlay == Overlay::None => self.input_cursor = 0,
            KeyCode::End if self.overlay == Overlay::None => self.input_cursor = self.input.len(),
            KeyCode::Backspace if self.overlay == Overlay::None => {
                self.delete_input_before_cursor()
            }
            KeyCode::Delete if self.overlay == Overlay::None => self.delete_input_at_cursor(),
            KeyCode::Char('w') if control && self.overlay == Overlay::None => {
                self.delete_input_word()
            }
            KeyCode::Enter
                if self.overlay == Overlay::None
                    && !self.busy
                    && !self.cluster_starting
                    && !self.setup_busy
                    && !self.input.trim().is_empty() =>
            {
                return Action::Send
            }
            KeyCode::Char(c)
                if self.overlay == Overlay::None
                    && !control
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.input.insert(self.input_cursor, c);
                self.input_cursor += c.len_utf8();
            }
            _ => {}
        }
        Action::None
    }

    pub fn catalog_model(&self, id: &str) -> Option<&CatalogRow> {
        self.catalog.iter().find(|model| model.id == id)
    }
}

#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    Send,
    Refresh,
    Pair,
    Preview {
        request_id: u64,
        model: String,
        ctx: u32,
    },
    InstallOnly(String),
    Browse {
        request_id: u64,
        query: String,
    },
    Download {
        source: ModelSource,
        name: String,
    },
    ToggleDevice {
        id: String,
        enabled: bool,
    },
    Start {
        model: String,
        ctx: u32,
    },
    StopCluster,
    CancelChat,
    CancelInstall(String),
    DeleteModel(String),
    ResetRuntime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_letters_are_text_and_shortcuts_require_control() {
        let mut app = App::new(String::new(), String::new());
        for c in "deep prompt".chars() {
            assert_eq!(
                app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
                Action::None
            );
        }
        assert_eq!(app.input, "deep prompt");
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
            Action::Pair
        );
    }

    #[test]
    fn opening_the_picker_keeps_the_current_model_selected() {
        let mut app = App::new(String::new(), String::new());
        app.models = vec!["a".into(), "b".into()];
        app.servable_models = app.models.clone();
        app.model = Some("b".into());
        app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.selected_model, Some(1));
    }

    #[test]
    fn an_rpc_address_alone_does_not_count_as_reachable_memory() {
        let device = DeviceRow::from_json(&serde_json::json!({
            "id":"phone", "rpc":"10.0.0.1:50052", "ram_mb":8000,
            "available_mb":4200, "budget_mb":7100, "cores":8, "threads":4,
            "enabled":false, "assigned_layers":12, "assigned_memory_mb":900
        }));
        assert!(!device.usable);
        assert!(!device.reachable);
        assert!(!device.enabled);
        assert_eq!(device.available_mb, 4200);
        assert_eq!(device.assigned_layers, 12);
    }

    #[test]
    fn device_panel_can_include_and_exclude_the_selected_device() {
        let mut app = App::new(String::new(), String::new());
        app.devices = vec![DeviceRow {
            id: "phone".into(),
            name: "phone".into(),
            enabled: true,
            ..Default::default()
        }];
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Action::Refresh
        );
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
            Action::ToggleDevice {
                id: "phone".into(),
                enabled: false,
            }
        );
    }

    #[test]
    fn input_editing_uses_unicode_grapheme_boundaries() {
        let mut app = App::new(String::new(), String::new());
        for c in "a🙂b".chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.input_cursor, app.input.len());
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
        assert_eq!(app.input, "aX🙂b");
        app.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.input, "aXb");
        app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.input, "Xb");
        app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        app.paste(" one\ntwo");
        assert_eq!(app.input, "Xb one two");
        app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.input, "Xb one ");
    }

    #[test]
    fn help_does_not_steal_question_marks_from_the_prompt() {
        let mut app = App::new(String::new(), String::new());
        app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert_eq!(app.input, "?");
        app.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert_eq!(app.overlay, Overlay::Help);
    }

    #[test]
    fn model_picker_previews_context_and_blocks_an_oversized_load() {
        let mut app = App::new(String::new(), String::new());
        app.models = vec!["large".into()];
        app.catalog = vec![CatalogRow {
            id: "large".into(),
            ..Default::default()
        }];
        let preview = app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(
            preview,
            Action::Preview {
                request_id: 1,
                model: "large".into(),
                ctx: 4096,
            }
        );
        assert_eq!(app.overlay, Overlay::ModelPicker);
        let next = app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.context_tokens, 8192);
        assert_eq!(
            next,
            Action::Preview {
                request_id: 2,
                model: "large".into(),
                ctx: 8192,
            }
        );
        app.preview_pending = false;
        app.preview = Some(SplitPlan {
            model: "large".into(),
            total_layers: 28,
            ctx: 8192,
            assignments: vec![],
            over_budget: true,
            shortfall_mb: 500,
            summary: "500 MB short".into(),
        });
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Action::None
        );
        assert_eq!(app.overlay, Overlay::ModelPicker);
        assert_eq!(app.notice.as_deref(), Some("500 MB short"));
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE)),
            Action::InstallOnly("large".into())
        );
    }

    #[test]
    fn fitting_preview_starts_with_the_selected_context() {
        let mut app = App::new(String::new(), String::new());
        app.models = vec!["model".into()];
        app.selected_model = Some(0);
        app.overlay = Overlay::ModelPicker;
        app.context_tokens = 2048;
        app.preview = Some(SplitPlan {
            model: "model".into(),
            total_layers: 24,
            ctx: 2048,
            assignments: vec![],
            over_budget: false,
            shortfall_mb: 0,
            summary: "fits".into(),
        });
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Start {
                model: "model".into(),
                ctx: 2048,
            }
        );
        assert_eq!(app.overlay, Overlay::Activity);
    }

    #[test]
    fn page_navigation_pauses_and_resumes_chat_following() {
        let mut app = App::new(String::new(), String::new());
        app.chat_rows = 100;
        app.chat_page_rows = 10;
        app.list_state.select(Some(99));
        app.key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.list_state.selected(), Some(89));
        assert!(!app.chat_follow);
        app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.list_state.selected(), Some(99));
        assert!(app.chat_follow);
    }

    #[test]
    fn stop_shortcut_targets_the_active_operation() {
        let mut app = App::new(String::new(), String::new());
        app.busy = true;
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Action::CancelChat
        );
        app.busy = false;
        app.cluster_starting = true;
        app.overlay = Overlay::Activity;
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Action::StopCluster
        );
    }

    #[test]
    fn newest_history_message_is_truncated_to_the_context_budget() {
        let mut app = App::new(String::new(), String::new());
        app.context_tokens = 1024;
        app.messages.push(Msg {
            role: "user".into(),
            content: "x".repeat(10_000),
            tag: None,
        });
        let history = app.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].1.len(), 2304);
    }
}
