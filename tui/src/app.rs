//! App state shared between the event loop, UI and API tasks.

use ratatui::widgets::ListState;
use serde_json::Value;

/// A device in the compute pool, as reported by the node.
#[derive(Debug, Clone, Default)]
pub struct DeviceRow {
    pub name: String,
    pub kind: String,
    pub budget_mb: u64,
    pub threads: u32,
    pub is_self: bool,
    pub stale: bool,
    pub reachable: bool,
}

impl DeviceRow {
    pub fn from_json(v: &Value) -> Self {
        Self {
            name: v["name"].as_str().unwrap_or("?").to_string(),
            kind: v["kind"].as_str().unwrap_or("?").to_string(),
            budget_mb: v["budget_mb"].as_u64().unwrap_or(0),
            threads: v["threads"].as_u64().unwrap_or(0) as u32,
            is_self: v["self"].as_bool().unwrap_or(false),
            stale: v["stale"].as_bool().unwrap_or(false),
            reachable: v["reachable"]
                .as_bool()
                .or_else(|| v["rpc"].as_str().map(|r| !r.is_empty()))
                .unwrap_or(false),
        }
    }
}

/// Which overlay is open, if any.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Overlay {
    #[default]
    None,
    ModelPicker,
    Devices,
    Pairing,
}

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

    // --- product surface ---------------------------------------------------
    /// Which overlay is showing.
    pub overlay: Overlay,
    /// Devices in the compute pool.
    pub devices: Vec<DeviceRow>,
    /// One-line cluster state for the status bar ("local" / "distributed 19/36L").
    pub cluster_label: String,
    /// Pairing URI rendered as a QR in the Pairing overlay.
    pub qr_lines: Vec<String>,
    pub pair_uri: String,
    /// Transient message shown under the status bar.
    pub notice: Option<String>,
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
            overlay: Overlay::None,
            devices: Vec::new(),
            cluster_label: "local".into(),
            qr_lines: Vec::new(),
            pair_uri: String::new(),
            notice: None,
        }
    }

    pub fn history(&self) -> Vec<(String, String)> {
        self.messages
            .iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect()
    }

    /// Total memory the pool can lend, counting only usable devices.
    pub fn pool_mb(&self) -> u64 {
        self.devices
            .iter()
            .filter(|d| d.is_self || d.reachable)
            .map(|d| d.budget_mb)
            .sum()
    }

    pub fn scroll_to_bottom(&mut self) {
        let len = self.messages.len() + if self.streaming.is_empty() { 0 } else { 1 };
        if len > 0 {
            self.list_state.select(Some(len - 1));
        }
    }
}
