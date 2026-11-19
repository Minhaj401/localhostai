use crate::app::{App, Msg, Overlay};
use crate::{Node, UiEvent};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use localhostai_core::conversations::{Conversation, Summary};
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Default)]
pub struct Workspace {
    pub current: Option<Conversation>,
    pub library: Vec<Summary>,
    pub query: String,
    pub selected: usize,
    pub renaming: bool,
    pub supported: bool,
    pub pending: bool,
    pub refreshing: bool,
    pub epoch: u64,
    pub metrics: Value,
    pub rtt_ms: Option<f64>,
    pub online: bool,
    pub scroll: u16,
    pub draft: String,
    pub drafts: std::collections::HashMap<String, String>,
}
impl Workspace {
    pub fn invalidate(&mut self) {
        self.epoch += 1;
        self.refreshing = false;
    }
    pub fn generating(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|c| c.active_request.is_some())
    }
    pub fn benchmarking(&self) -> bool {
        self.metrics["benchmark"]["state"] == "running"
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
        if self.pending {
            return Action::None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('n') if ctrl => Action::New,
            KeyCode::Char('r') if ctrl => {
                self.renaming = true;
                self.query = self
                    .current
                    .as_ref()
                    .map(|c| c.title.clone())
                    .unwrap_or_default();
                Action::None
            }
            KeyCode::Char('e') if ctrl => Action::Export("markdown".into()),
            KeyCode::Char('y') if ctrl => Action::Export("json".into()),
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.library.len().saturating_sub(1));
                Action::None
            }
            KeyCode::Backspace => {
                self.query.pop();
                Action::None
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.selected = 0;
                Action::None
            }
            KeyCode::Enter if self.renaming => Action::Rename(self.query.clone()),
            KeyCode::Enter => self
                .library
                .get(self.selected)
                .map(|c| Action::Open(c.id.clone()))
                .unwrap_or(Action::None),
            KeyCode::Delete => self
                .library
                .get(self.selected)
                .map(|c| Action::Delete(c.id.clone(), c.revision))
                .unwrap_or(Action::None),
            _ => Action::None,
        }
    }
}

pub async fn initialize(app: &mut App, node: &Node) {
    app.workspace.supported = true;
    let result = snapshot(node, None, "").await;
    if result
        .as_ref()
        .err()
        .and_then(|e| e.downcast_ref::<reqwest::Error>())
        .and_then(|e| e.status())
        == Some(reqwest::StatusCode::NOT_FOUND)
    {
        app.workspace.supported = false;
    }
    apply(
        app,
        Update {
            epoch: 0,
            result,
            command: false,
        },
    );
}

#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    New,
    Open(String),
    Rename(String),
    Delete(String, u64),
    Export(String),
    Benchmark,
    CancelBenchmark,
}
pub struct Update {
    pub epoch: u64,
    pub result: Result<Snapshot>,
    pub command: bool,
}
pub struct Snapshot {
    pub library: Vec<Summary>,
    pub current: Option<Conversation>,
    pub metrics: Value,
    pub rtt_ms: Option<f64>,
    pub notice: Option<String>,
}

pub fn refresh(app: &mut App, node: &Node, tx: &UnboundedSender<UiEvent>) {
    if app.workspace.refreshing || app.workspace.pending {
        return;
    }
    app.workspace.refreshing = true;
    let epoch = app.workspace.epoch;
    let current = app.workspace.current.as_ref().map(|c| c.id.clone());
    let query = if app.workspace.renaming {
        String::new()
    } else {
        app.workspace.query.clone()
    };
    let node = node.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = snapshot(&node, current, &query).await;
        let _ = tx.send(UiEvent::Workspace(Update {
            epoch,
            result,
            command: false,
        }));
    });
}

async fn snapshot(node: &Node, current: Option<String>, query: &str) -> Result<Snapshot> {
    let mut url = reqwest::Url::parse("http://localhost/conversations")?;
    url.query_pairs_mut().append_pair("query", query);
    let library = node
        .get_json(&format!("/conversations?{}", url.query().unwrap_or("")))
        .await?;
    let library: Vec<Summary> = serde_json::from_value(library["conversations"].clone())?;
    let current = match current.or_else(|| library.first().map(|c| c.id.clone())) {
        Some(id) => match node.get_json(&format!("/conversations/{id}")).await {
            Ok(value) => Some(serde_json::from_value(value["conversation"].clone())?),
            Err(error)
                if error
                    .downcast_ref::<reqwest::Error>()
                    .and_then(|e| e.status())
                    == Some(reqwest::StatusCode::NOT_FOUND) =>
            {
                None
            }
            Err(error) => return Err(error),
        },
        None => None,
    };
    let at = std::time::Instant::now();
    let rtt_ms = node
        .get_json("/ping")
        .await
        .ok()
        .map(|_| at.elapsed().as_secs_f64() * 1000.0);
    let metrics = node.get_json("/performance").await.unwrap_or(Value::Null);
    Ok(Snapshot {
        library,
        current,
        metrics,
        rtt_ms,
        notice: None,
    })
}

pub fn launch(app: &mut App, action: Action, node: &Node, tx: &UnboundedSender<UiEvent>) {
    if matches!(action, Action::None) || app.workspace.pending {
        return;
    }
    if app.busy && !matches!(action, Action::Export(_)) {
        app.notice = Some("Stop the current reply before changing conversations".into());
        return;
    }
    app.workspace.pending = true;
    app.workspace.epoch += 1;
    let epoch = app.workspace.epoch;
    let current = app.workspace.current.clone();
    let model = app
        .selected_model
        .and_then(|i| app.models.get(i).cloned())
        .or_else(|| app.model.clone())
        .or_else(|| app.cluster_model.clone());
    let ctx = app.context_tokens;
    let node = node.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = command(&node, action, current, model, ctx).await;
        let _ = tx.send(UiEvent::Workspace(Update {
            epoch,
            result,
            command: true,
        }));
    });
}

async fn command(
    node: &Node,
    action: Action,
    current: Option<Conversation>,
    model: Option<String>,
    ctx: u32,
) -> Result<Snapshot> {
    let mut id = current.as_ref().map(|c| c.id.clone());
    let mut notice = None;
    match action {
        Action::New => {
            let value = node.post_json("/conversations", &json!({})).await?;
            id = value["conversation"]["id"].as_str().map(String::from);
        }
        Action::Open(selected) => id = Some(selected),
        Action::Rename(title) => {
            let c = current.context("Open a conversation first")?;
            node.post_json(
                &format!("/conversations/{}", c.id),
                &json!({"revision":c.revision,"title":title}),
            )
            .await?;
        }
        Action::Delete(selected, revision) => {
            node.post_json(
                &format!("/conversations/{selected}/delete"),
                &json!({"revision":revision}),
            )
            .await?;
            if id.as_deref() == Some(&selected) {
                id = None;
            }
        }
        Action::Export(format) => {
            let id = id.as_ref().context("Open a conversation first")?;
            let exported = node
                .get_json(&format!("/conversations/{id}/export?format={format}"))
                .await?;
            let content = exported["content"]
                .as_str()
                .context("invalid export")?
                .to_string();
            let extension = if format == "json" { "json" } else { "md" };
            let directory = localhostai_core::config::home().join("exports");
            let path = directory.join(format!(
                "chat-{id}-{}.{extension}",
                localhostai_core::conversations::now_ms()
            ));
            let path = tokio::task::spawn_blocking(move || -> Result<_> {
                use std::io::Write;
                std::fs::create_dir_all(&directory)?;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&path)?;
                file.write_all(content.as_bytes())?;
                file.sync_all()?;
                Ok(path)
            })
            .await??;
            notice = Some(format!("Exported {}", path.display()));
        }
        Action::Benchmark => {
            node.post_json(
                "/benchmarks/start",
                &json!({"model":model.context("Choose an installed model first")?,"ctx":ctx}),
            )
            .await?;
            notice = Some(
                "Benchmark started: two loads, same prompt and settings; model stops afterward"
                    .into(),
            );
        }
        Action::CancelBenchmark => {
            node.post_json("/benchmarks/cancel", &json!({})).await?;
            notice = Some("Cancelling benchmark and releasing model memory".into());
        }
        Action::None => {}
    }
    let mut result = snapshot(node, id, "").await?;
    result.notice = notice;
    Ok(result)
}

pub fn apply(app: &mut App, update: Update) {
    if update.epoch != app.workspace.epoch {
        return;
    }
    app.workspace.refreshing = false;
    if update.command {
        app.workspace.pending = false;
    }
    match update.result {
        Ok(snapshot) => {
            app.workspace.supported = true;
            app.workspace.online = true;
            app.workspace.library = snapshot.library;
            app.workspace.selected = app
                .workspace
                .selected
                .min(app.workspace.library.len().saturating_sub(1));
            if !snapshot.metrics.is_null() {
                app.workspace.metrics = snapshot.metrics;
            }
            app.workspace.rtt_ms = snapshot.rtt_ms;
            if !app.busy {
                let switched = app.workspace.current.as_ref().map(|c| &c.id)
                    != snapshot.current.as_ref().map(|c| &c.id);
                if switched {
                    let old = app
                        .workspace
                        .current
                        .as_ref()
                        .map(|c| c.id.clone())
                        .unwrap_or_default();
                    app.workspace
                        .drafts
                        .insert(old, std::mem::take(&mut app.input));
                    let new = snapshot
                        .current
                        .as_ref()
                        .map(|c| c.id.as_str())
                        .unwrap_or("");
                    app.input = app.workspace.drafts.remove(new).unwrap_or_default();
                    app.input_cursor = app.input.len();
                }
                app.workspace.current = snapshot.current;
                if let Some(chat) = &app.workspace.current {
                    app.messages = chat
                        .messages
                        .iter()
                        .map(|m| Msg {
                            role: m.role.clone(),
                            content: m.content.clone(),
                            tag: if m.interrupted {
                                Some(
                                    if chat.active_request.is_some()
                                        && chat.messages.last().is_some_and(|last| last.id == m.id)
                                    {
                                        "generating on coordinator"
                                    } else {
                                        "interrupted"
                                    }
                                    .into(),
                                )
                            } else {
                                m.model.clone()
                            },
                        })
                        .collect();
                    app.streaming.clear();
                    if switched {
                        app.system_prompt = chat.system_prompt.clone();
                        app.context_tokens = chat.ctx;
                    }
                } else {
                    app.messages.clear();
                    app.streaming.clear();
                }
                if switched {
                    app.scroll_to_bottom();
                }
            }
            if update.command {
                app.workspace.query.clear();
                app.workspace.renaming = false;
                if app.overlay == Overlay::Conversations {
                    app.overlay = Overlay::None;
                }
            }
            if snapshot.notice.is_some() {
                app.notice = snapshot.notice;
            }
        }
        Err(error) => {
            app.workspace.online = false;
            if update.command || app.workspace.supported {
                app.notice = Some(format!("Conversation sync: {error:#}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_updates_do_not_replace_the_selected_chat_or_pending_draft() {
        let mut app = App::new("node".into(), String::new());
        app.workspace.epoch = 2;
        app.input = "keep this draft".into();
        apply(
            &mut app,
            Update {
                epoch: 1,
                command: true,
                result: Ok(Snapshot {
                    library: vec![],
                    current: None,
                    metrics: Value::Null,
                    rtt_ms: None,
                    notice: None,
                }),
            },
        );
        assert_eq!(app.input, "keep this draft");
        assert!(!app.workspace.supported);
        let action = app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(action, crate::app::Action::None);
        assert_eq!(app.overlay, Overlay::Conversations);
        app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(app.workspace.query, "n");
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)),
            crate::app::Action::Workspace(Action::Export("markdown".into()))
        );
    }
}
