//! PrivateLM-LAN TUI — terminal chat client for the laptop node.
//!
//! Usage:
//!   privatelm-tui [node-url]            interactive TUI (default http://127.0.0.1:8765)
//!   privatelm-tui --once "prompt"       single-shot: print streamed reply, exit

mod api;
mod app;
mod ui;

use anyhow::{bail, Result};
use api::{ApiEvent, Node};
use app::{App, Msg};
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEventKind},
    execute,
    terminal::{enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

enum UiEvent {
    Key(crossterm::event::KeyEvent),
    Api(ApiEvent),
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let base = std::env::var("PRIVATELM_NODE")
        .unwrap_or_else(|_| "http://127.0.0.1:8765".into());

    if let Some(pos) = args.iter().position(|a| a == "--once") {
        let prompt = args.get(pos + 1).cloned();
        let url = args.iter().find(|a| a.starts_with("http")).cloned().unwrap_or(base);
        return once(&url, prompt).await;
    }

    let url = args.first().cloned().unwrap_or(base);
    run_tui(url).await
}

/// Single-shot mode — testable without a TTY.
async fn once(url: &str, prompt: Option<String>) -> Result<()> {
    let prompt = match prompt {
        Some(p) if !p.is_empty() => p,
        _ => bail!("--once needs a prompt: privatelm-tui --once \"hello\""),
    };
    let node = Node::new(url);
    let health = node.health().await?;
    if !health.ok {
        bail!("node reports no backend (backend: {})", health.backend);
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    let node2 = node.clone();
    let model = health.models.first().cloned();
    let handle = tokio::spawn(async move {
        node2
            .chat_stream(
                model.as_deref(),
                vec![("user".into(), prompt)],
                None,
                tx,
            )
            .await
    });
    while let Some(ev) = rx.recv().await {
        match ev {
            ApiEvent::Chunk(s) => print!("{s}"),
            ApiEvent::Failed(e) => eprintln!("\n[node error] {e}"),
            ApiEvent::Done => break,
        }
    }
    println!();
    let _ = handle.await;
    Ok(())
}

async fn run_tui(url: String) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let res = event_loop(&mut terminal, url).await;

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    res
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    url: String,
) -> Result<()> {
    let node = Node::new(&url);
    let mut app = App::new(url.clone(), String::new());

    // Health + model list on startup.
    if let Ok(h) = node.health().await {
        app.node_label = h.name;
        app.backend_label = h.backend;
        app.models = h.models;
        app.model = app.models.first().cloned();
        app.selected_model = if app.models.is_empty() { None } else { Some(0) };
    }

    // Keyboard events come from a blocking thread; API events from tasks.
    let (key_tx, mut rx): (UnboundedSender<UiEvent>, UnboundedReceiver<UiEvent>) =
        mpsc::unbounded_channel();
    let key_tx_events = key_tx.clone();
    tokio::task::spawn_blocking(move || loop {
        match event::read() {
            Ok(CEvent::Key(k)) if k.kind == KeyEventKind::Press => {
                if key_tx_events.send(UiEvent::Key(k)).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => return,
        }
    });

    loop {
        terminal.draw(|f| {
            ui::draw(f, &mut app);
            if app.picker_open {
                let area = ui::centered_rect(50, (app.models.len() as u16 + 2).min(12), f.area());
                ui::model_picker(f, &app, area);
            }
        })?;

        let Some(ev) = rx.recv().await else { break };
        match ev {
            UiEvent::Api(ApiEvent::Chunk(s)) => {
                app.streaming.push_str(&s);
                app.scroll_to_bottom();
            }
            UiEvent::Api(ApiEvent::Done) => {
                let content = std::mem::take(&mut app.streaming);
                app.messages.push(Msg {
                    role: "assistant".into(),
                    content,
                    tag: Some(app.model.clone().unwrap_or_default()),
                });
                app.busy = false;
                app.scroll_to_bottom();
            }
            UiEvent::Api(ApiEvent::Failed(e)) => {
                app.streaming.clear();
                app.messages.push(Msg {
                    role: "assistant".into(),
                    content: format!("[error] {e}"),
                    tag: None,
                });
                app.busy = false;
            }
            UiEvent::Key(key) => match key.code {
                KeyCode::Esc if app.picker_open => app.picker_open = false,
                KeyCode::Esc => break,
                KeyCode::Up if app.picker_open => {
                    if let Some(i) = app.selected_model {
                        app.selected_model = Some(i.saturating_sub(1));
                    }
                }
                KeyCode::Down if app.picker_open => {
                    if let Some(i) = app.selected_model {
                        if i + 1 < app.models.len() {
                            app.selected_model = Some(i + 1);
                        }
                    }
                }
                KeyCode::Enter if app.picker_open => {
                    app.model = app.selected_model.and_then(|i| app.models.get(i).cloned());
                    app.picker_open = false;
                }
                KeyCode::Up if !app.busy => {
                    app.selected_model = Some(match app.selected_model {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    });
                    app.picker_open = true;
                }
                KeyCode::Char('c')
                    if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    break
                }
                KeyCode::Backspace if !app.picker_open => {
                    app.input.pop();
                }
                KeyCode::Enter if !app.picker_open && !app.busy && !app.input.is_empty() => {
                    let prompt = std::mem::take(&mut app.input);
                    app.messages.push(Msg {
                        role: "user".into(),
                        content: prompt.clone(),
                        tag: None,
                    });
                    app.busy = true;
                    app.streaming.clear();
                    app.scroll_to_bottom();
                    let node = node.clone();
                    let history = app.history();
                    let model = app.model.clone();
                    let system = app.system_prompt.clone();
                    let (tx, rx) = mpsc::unbounded_channel();
                    let tx_err = tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = node
                            .chat_stream(
                                model.as_deref(),
                                history,
                                if system.is_empty() { None } else { Some(&system) },
                                tx,
                            )
                            .await
                        {
                            let _ = tx_err.send(ApiEvent::Failed(e.to_string()));
                        }
                    });
                    // splice the API receiver into the main receiver
                    tokio::spawn(forward(rx, key_tx.clone()));
                }
                KeyCode::Char(c) if !app.picker_open => app.input.push(c),
                _ => {}
            },
        }
    }
    Ok(())
}

async fn forward(mut from: UnboundedReceiver<ApiEvent>, to: UnboundedSender<UiEvent>) {
    while let Some(ev) = from.recv().await {
        if to.send(UiEvent::Api(ev)).is_err() {
            return;
        }
    }
}
