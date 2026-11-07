//! PrivateLM core — everything the laptop needs to be a node, embedded.
//!
//! No Python, no separate service, no install step. The TUI binary links this
//! crate and the node *is* the TUI process.
//!
//! Modules:
//!   config     — `~/.privateml/config.toml`, directories, port selection
//!   discovery  — UDP broadcast responder + client so the APK finds the node
//!   engine     — Ollama detection / llama.cpp supervision (self-contained mode)
//!   server     — HTTP: /health, /v1/models, /v1/chat/completions (SSE), /pair
//!   api        — client used by the TUI (and testable standalone)
//!   models     — GGUF catalog + download with progress

pub mod api;
pub mod cluster;
pub mod config;
pub mod discovery;
pub mod hardware;
pub mod plan;
pub mod qr;
pub mod server;

pub use api::{ApiEvent, Node};
pub use cluster::{Cluster, ClusterState, ClusterStatus, Toolchain};
pub use config::Config;
pub use hardware::Hardware;
pub use plan::{ModelInfo, SplitPlan};
pub use server::{NodeHandle, NodeRuntime, ServerStatus};

/// Product identity, single source of truth.
pub const APP_NAME: &str = "privateml";
pub const APP_DISPLAY_NAME: &str = "PrivateLM";
pub const DISCOVERY_MAGIC: &str = "PRIVATELM_DISCOVER";
pub const DISCOVERY_PORT: u16 = 50897;
pub const DEFAULT_PORT: u16 = 8765;
pub const RPC_PORT: u16 = 50052;
