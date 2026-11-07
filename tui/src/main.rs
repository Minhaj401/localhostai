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
use app::{App, DeviceRow, Msg, Overlay};
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
        "--status", "--once", "--help", "-h", "--version", "-V", "--headless", "--connect",
    ];
    for a in &args {
        if a.starts_with('-') && !KNOWN.contains(&a.as_str()) {
            bail!("unknown option {a}\n\nRun `privateml --help` for usage.");
        }
    }

    // Attach to a node that is already running instead of starting another.
    // Needed because one node can only own the port, and a UI on another
    // machine (or a second terminal) should still be able to drive it.
    if let Some(pos) = args.iter().position(|a| a == "--connect") {
        let url = match args.get(pos + 1) {
            Some(u) if u.starts_with("http") => u.clone(),
            _ => bail!("--connect needs a URL, e.g. --connect http://192.168.1.20:8765"),
        };
        let token = std::env::var("PRIVATELM_TOKEN").ok();
        let node = match token {
            Some(t) => Node::with_token(&url, Some(t)),
            None => Node::new(&url),
        };
        // Fail fast with an actionable message rather than an empty screen.
        match node.health().await {
            Ok(h) => {
                println!(
                    "Connected to {} ({}) · {} · {} model(s)",
                    h.name,
                    url,
                    h.backend,
                    h.models.len()
                );
            }
            Err(e) => bail!(
                "cannot reach a PrivateLM node at {url}: {e}\n\
                 Start one with `privateml` on that machine, or check the address/port."
            ),
        }
        return run_tui(url, None).await;
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
  --connect <URL>     attach the UI to a node that is already running\n\
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
    refresh_devices(&node, &mut app).await;
    refresh_cluster(&node, &mut app).await;

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

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(4));
    ticker.tick().await;

    loop {
        terminal.draw(|f| {
            ui::draw(f, &mut app);
            match app.overlay {
                Overlay::ModelPicker => {
                    let h = (app.models.len() as u16 + 4).min(14);
                    let area = ui::centered_rect(60, h, f.area());
                    ui::model_picker(f, &app, area);
                }
                Overlay::Devices => {
                    let h = (app.devices.len() as u16 + 8).min(18);
                    let area = ui::centered_rect(86, h, f.area());
                    ui::devices_panel(f, &app, area);
                }
                Overlay::Pairing => {
                    let h = (app.qr_lines.len() as u16 + 8).min(f.area().height);
                    let w = app
                        .qr_lines
                        .first()
                        .map(|l| l.chars().count() as u16 + 4)
                        .unwrap_or(46)
                        .max(46);
                    let area = ui::centered_rect(w + 4, h, f.area());
                    ui::pairing_panel(f, &app, area);
                }
                Overlay::None => {}
            }
        })?;

        let ev = tokio::select! {
            got = rx.recv() => match got {
                Some(e) => e,
                None => break,
            },
            _ = ticker.tick() => {
                // Periodic refresh so the pool and cluster state stay truthful.
                refresh_devices(&node, &mut app).await;
                refresh_cluster(&node, &mut app).await;
                continue;
            }
        };
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
                // ---- overlays close first
                KeyCode::Esc if app.overlay != Overlay::None => app.overlay = Overlay::None,
                KeyCode::Esc => break,
                KeyCode::Char('d') if app.overlay == Overlay::None && !app.busy => {
                    refresh_devices(&node, &mut app).await;
                    app.overlay = Overlay::Devices;
                }
                KeyCode::Char('p') if app.overlay == Overlay::None && !app.busy => {
                    load_qr(&node, &mut app).await;
                    app.overlay = Overlay::Pairing;
                }
                // ---- model picker movement
                KeyCode::Up if app.overlay == Overlay::ModelPicker => {
                    if let Some(i) = app.selected_model {
                        app.selected_model = Some(i.saturating_sub(1));
                    }
                }
                KeyCode::Down if app.overlay == Overlay::ModelPicker => {
                    if let Some(i) = app.selected_model {
                        if i + 1 < app.models.len() {
                            app.selected_model = Some(i + 1);
                        }
                    }
                }
                KeyCode::Enter if app.overlay == Overlay::ModelPicker => {
                    app.model = app.selected_model.and_then(|i| app.models.get(i).cloned());
                    app.overlay = Overlay::None;
                }
                KeyCode::Char('D') if app.overlay == Overlay::ModelPicker => {
                    let chosen = app.selected_model.and_then(|i| app.models.get(i).cloned());
                    app.overlay = Overlay::None;
                    if let Some(m) = chosen {
                        start_distributed(&node, &mut app, &m).await;
                    }
                }
                KeyCode::Up if app.overlay == Overlay::None && !app.busy => {
                    app.selected_model = Some(match app.selected_model {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    });
                    app.overlay = Overlay::ModelPicker;
                }
                KeyCode::Char('c')
                    if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    break
                }
                KeyCode::Backspace if app.overlay == Overlay::None => {
                    app.input.pop();
                }
                KeyCode::Enter if app.overlay == Overlay::None && !app.busy && !app.input.is_empty() => {
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
                KeyCode::Char(c) if app.overlay == Overlay::None => app.input.push(c),
                _ => {}
            },
        }
    }
    Ok(())
}

/// Pull the device pool from the node. Only reachable devices are counted,
/// so the number shown here is memory we can genuinely use.
async fn refresh_devices(node: &Node, app: &mut App) {
    if let Ok(v) = node.get_json("/peers").await {
        if let Some(arr) = v["devices"].as_array() {
            app.devices = arr.iter().map(DeviceRow::from_json).collect();
        }
    }
}

/// Reflect whether we are serving locally or from a split model.
async fn refresh_cluster(node: &Node, app: &mut App) {
    let Ok(v) = node.get_json("/cluster").await else { return };
    let state = v["state"].as_str().unwrap_or("stopped");
    app.cluster_label = match state {
        "running" => format!(
            "distributed {}/{}L",
            v["layers_on_self"].as_u64().unwrap_or(0),
            v["total_layers"].as_u64().unwrap_or(0)
        ),
        "starting" => "starting…".into(),
        "failed" => "cluster failed".into(),
        _ => "local".into(),
    };
    if let Some(err) = v["error"].as_str() {
        app.notice = Some(err.to_string());
    }
}

/// Build the pairing QR from the node's /pair payload.
async fn load_qr(node: &Node, app: &mut App) {
    let Ok(v) = node.get_json("/pair").await else {
        app.notice = Some("could not read pairing info".into());
        return;
    };
    let uri = v["uri"].as_str().unwrap_or("").to_string();
    if uri.is_empty() {
        app.notice = Some("node did not return a pairing URI".into());
        return;
    }
    match privateml_core::qr::encode(&uri) {
        Some(grid) => {
            app.qr_lines = privateml_core::qr::render_terminal(&grid, 2);
            app.pair_uri = uri;
        }
        None => app.notice = Some("pairing payload too large to encode".into()),
    }
}

/// Plan a split for `model` across the pool, then ask the node to serve it.
async fn start_distributed(node: &Node, app: &mut App, model: &str) {
    // Register the model if the node does not know it yet (Ollama names may
    // not have a GGUF path, which distributed mode requires).
    let plan_req = serde_json::json!({ "model": model, "ctx": 4096 });
    match node.post_json("/cluster/plan", &plan_req).await {
        Ok(v) => {
            let summary = v["plan"]["summary"].as_str().unwrap_or("").to_string();
            let fits = !v["plan"]["over_budget"].as_bool().unwrap_or(true);
            if !fits {
                app.notice = Some(summary);
                return;
            }
            app.notice = Some(format!("starting: {summary}"));
            match node
                .post_json("/cluster/start", &serde_json::json!({ "model": model, "ctx": 4096 }))
                .await
            {
                Ok(r) => {
                    if r["ok"].as_bool().unwrap_or(false) {
                        app.notice = Some(format!("distributed: {summary}"));
                        refresh_cluster(node, app).await;
                    } else {
                        let e = r["error"]
                            .as_str()
                            .or_else(|| r["status"]["error"].as_str())
                            .unwrap_or("cluster failed");
                        app.notice = Some(e.to_string());
                    }
                }
                Err(e) => app.notice = Some(format!("start failed: {e}")),
            }
        }
        Err(e) => {
            // 404 means the model has no GGUF path registered — say so plainly.
            app.notice = Some(format!(
                "cannot plan '{model}': {e}. Distributed mode needs a GGUF file \
                 (Ollama-only models must be exported first)."
            ));
        }
    }
}

async fn forward(mut from: UnboundedReceiver<ApiEvent>, to: UnboundedSender<UiEvent>) {
    while let Some(ev) = from.recv().await {
        if to.send(UiEvent::Api(ev)).is_err() {
            return;
        }
    }
}
