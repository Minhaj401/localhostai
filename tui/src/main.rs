//! PrivateLM — one binary. Chatting UI *and* the LAN node itself.
//!
//! Usage:
//!   privatelm                       start the node + interactive chat
//!   privatelm --once "prompt"       single-shot: print streamed reply, exit
//!   privatelm --port 8765           force the listen port
//!   privatelm --no-lan              loopback only (no phone access)
//!   privatelm --status              print node info and exit
//!
//! The laptop node runs as a task inside this process: embedded HTTP API,
//! UDP discovery, Ollama/llama.cpp detection. Nothing else to install.

mod api;
mod app;
mod ui;

use privateml_core::{config::Config, server, NodeHandle};

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

    // ---- flags that do not start a UI
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!(
            "{} {}",
            privateml_core::APP_DISPLAY_NAME,
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    // ---- config + node startup (the node IS this process)
    let mut cfg = Config::load()?;
    for (i, a) in args.iter().enumerate() {
        match a.as_str() {
            "--port" => {
                if let Some(p) = args.get(i + 1).and_then(|p| p.parse().ok()) {
                    cfg.port = p;
                }
            }
            "--headless" => {} // handled below
            "--no-lan" => cfg.lan_enabled = false,
            "--no-discovery" => cfg.discovery = false,
            "--require-pairing" => cfg.require_pairing = true,
            "--name" => {
                if let Some(n) = args.get(i + 1) {
                    cfg.node_name = n.clone();
                }
            }
            _ => {}
        }
    }

    // Reject unknown flags early instead of dropping into the TUI, which needs a TTY.
    const KNOWN: &[&str] = &[
        "--port", "--no-lan", "--no-discovery", "--require-pairing", "--name",
        "--status", "--once", "--help", "-h", "--version", "-V", "--headless",
    ];
    for a in &args {
        if a.starts_with('-') && !KNOWN.contains(&a.as_str()) {
            bail!("unknown option {a}\n\nRun `privateml --help` for usage.");
        }
    }

    let node_handle = server::start(cfg.clone()).await?;
    let base = format!("http://127.0.0.1:{}", node_handle.port());

    if args.iter().any(|a| a == "--status") {
        let n = Node::with_token(&base, Some(node_handle.pair_token().into()));
        let h = n.health().await?;
        println!("{} {}", privateml_core::APP_DISPLAY_NAME, h.version);
        println!("node    : {}", h.name);
        println!("listen  : {}", base);
        println!("backend : {}", if h.backend.is_empty() { "none" } else { &h.backend });
        println!("pairing : {}", if h.pairing { "required" } else { "off" });
        println!("models  : {}", if h.models.is_empty() { "(none)".into() } else { h.models.join(", ") });
        node_handle.shutdown();
        return Ok(());
    }

    if let Some(pos) = args.iter().position(|a| a == "--once") {
        let prompt = args.get(pos + 1).cloned();
        let r = once(&base, prompt).await;
        node_handle.shutdown();
        return r;
    }

    // Headless: node only, no UI. Useful for SSH, systemd, and testing.
    if args.iter().any(|a| a == "--headless") {
        let h = node_handle.clone_ref();
        println!("{} {} — node listening on {base}", privateml_core::APP_DISPLAY_NAME, env!("CARGO_PKG_VERSION"));
        println!("pair token: {}", h.pair_token());
        tokio::signal::ctrl_c().await.ok();
        node_handle.shutdown();
        return Ok(());
    }

    // Interactive: hand the node handle to the UI so the status bar can show it.
    let r = run_tui(base, Some(node_handle.clone_ref())).await;
    node_handle.shutdown();
    r
}

fn print_help() {
    println!(
        "{name} {ver} — private LLM chat across your laptop and phone\n\
\n\
USAGE:\n  privatelm [OPTIONS]\n\
\n\
OPTIONS:\n\
  --once <PROMPT>     print one streamed reply and exit\n\
  --status            show node address, backend and models, then exit\n\
  --port <N>          listen port (default 8765, next free if taken)\n\
  --name <NAME>       name shown on the phone\n\
  --no-lan            loopback only; the phone cannot connect\n\
  --no-discovery      do not answer UDP discovery broadcasts\n\
  --require-pairing   demand the pair token on every request\n\
  --headless          run the node with no UI (SSH, systemd, testing)\n\
  -h, --help          show this help\n\
  -V, --version       show version\n\
\n\
FILES:\n  ~/.privateml/config.toml   settings\n\
  ~/.privateml/pair_token    shared secret for the phone\n\
  ~/.privateml/models/       downloaded GGUF models\n",
        name = privateml_core::APP_DISPLAY_NAME,
        ver = env!("CARGO_PKG_VERSION")
    );
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

async fn run_tui(url: String, handle: Option<NodeHandle>) -> Result<()> {
    use std::io::IsTerminal;
    if !io::stdout().is_terminal() {
        bail!(
            "PrivateLM's chat UI needs a terminal.\n\
             Try: privateml --status   (node info)\n\
             or:  privateml --once \"hello\"   (one reply)"
        );
    }
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let res = event_loop(&mut terminal, url, handle).await;

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    res
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    url: String,
    handle: Option<privateml_core::NodeHandle>,
) -> Result<()> {
    let node = match &handle {
        Some(h) => Node::with_token(&url, Some(h.pair_token().to_string())),
        None => Node::new(&url),
    };
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
