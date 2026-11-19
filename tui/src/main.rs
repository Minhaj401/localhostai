//! localhost.ai — one binary: terminal chat and an embedded LAN node.

mod api;
mod app;
mod cli;
mod model_browser;
mod ui;
mod workspace;
mod workspace_ui;

use anyhow::{bail, Context, Result};
use api::{ApiEvent, Health, Node};
use app::{Action, App, CatalogRow, DeviceRow, Msg, Overlay};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event as CEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use localhostai_core::{config::Config, server, SplitPlan};
use ratatui::{backend::CrosstermBackend, Terminal};
use serde_json::{json, Value};
use std::io::{self, Write};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

enum UiEvent {
    ConversationReady {
        generation: u64,
        conversation: localhostai_core::conversations::Conversation,
    },
    Workspace(workspace::Update),
    Key(crossterm::event::KeyEvent),
    Paste(String),
    Api {
        generation: u64,
        event: ApiEvent,
    },
    Status(Box<Snapshot>),
    Pair(Result<String>),
    DeviceToggle(Result<String>),
    Browse {
        request_id: u64,
        result: Result<localhostai_core::model_hub::HubResults>,
    },
    Downloaded {
        operation_id: u64,
        result: Result<localhostai_core::ModelInfo>,
    },
    Preview {
        request_id: u64,
        model: String,
        ctx: u32,
        result: Result<SplitPlan>,
    },
    Installed {
        operation_id: u64,
        model: String,
        result: Result<()>,
    },
    Cluster {
        operation_id: u64,
        model: Option<String>,
        notice: String,
    },
    Stopped {
        operation_id: u64,
        result: Result<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let options = cli::Options::parse(std::env::args().skip(1))?;
    if options.help {
        print_help();
        return Ok(());
    }
    if options.version {
        println!(
            "{} {}",
            localhostai_core::APP_DISPLAY_NAME,
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    let (node, handle) = if let Some(url) = &options.connect {
        (
            Node::with_token(url, std::env::var("LOCALHOSTAI_TOKEN").ok()),
            None,
        )
    } else {
        let mut cfg = Config::load()?;
        if let Some(port) = options.port {
            cfg.port = port;
        }
        if let Some(name) = options.name {
            cfg.node_name = name;
        }
        if options.no_lan {
            cfg.lan_enabled = false;
        }
        if options.no_discovery {
            cfg.discovery = false;
        }
        if options.require_pairing {
            cfg.require_pairing = true;
        }
        let handle = server::start(cfg).await?;
        let node = Node::with_token(
            &format!("http://127.0.0.1:{}", handle.port()),
            Some(handle.pair_token().into()),
        );
        (node, Some(handle))
    };

    let interrupted_command_is_error = options.once.is_some() || options.status;
    let work = async {
        if options.status {
            let h = node.health().await?;
            println!("{} {}", localhostai_core::APP_DISPLAY_NAME, h.version);
            println!("node    : {}", h.name);
            println!("listen  : {}", node.base());
            println!("backend : {}", h.backend);
            println!("pairing : {}", if h.pairing { "required" } else { "off" });
            println!(
                "models  : {}",
                if h.models.is_empty() {
                    "(none)".into()
                } else {
                    h.models.join(", ")
                }
            );
            Ok(())
        } else if let Some(prompt) = options.once {
            once(node, prompt).await
        } else if options.headless {
            println!(
                "{} {} — node listening on {}",
                localhostai_core::APP_DISPLAY_NAME,
                env!("CARGO_PKG_VERSION"),
                node.base()
            );
            if let Some(handle) = &handle {
                println!("pair token: {}", handle.pair_token());
            }
            std::future::pending::<Result<()>>().await
        } else {
            node.health().await?;
            run_tui(node).await
        }
    };
    let result = tokio::select! {
        result = work => result,
        result = shutdown_signal() => result.and_then(|()| {
            if interrupted_command_is_error {
                bail!("command interrupted");
            }
            Ok(())
        }),
    };
    if let Some(handle) = handle {
        handle.shutdown();
        handle.cluster().stop().await?;
    }
    result
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate())?;
        let mut hangup = signal(SignalKind::hangup())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => {},
            _ = hangup.recv() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

fn print_help() {
    println!(
        "localhost.ai {} — private LLM chat across your laptop and phone\n\
\nUSAGE:\n  localhostai [OPTIONS]\n\
\nOPTIONS:\n\
  --once <PROMPT>     print one streamed reply and exit\n\
  --status           show node address, backend and models, then exit\n\
  --port <N>         preferred port (default 8765, next free if taken)\n\
  --name <NAME>      name shown on the phone\n\
  --no-lan           loopback only\n\
  --no-discovery     do not answer UDP discovery broadcasts\n\
  --require-pairing  require a token for chat and control requests\n\
  --headless         run the node without a UI\n\
  --connect <URL>    use an existing node (also with --once or --status)\n\
  -h, --help         show help\n\
  -V, --version      show version\n\
\nKEYS:\n  Enter send · ↑ models · F1 help · Ctrl+A activity · Ctrl+X stop\n\
  Ctrl+D devices · Ctrl+P pair · PageUp/PageDown chat · Esc quit\n\
\nFILES:\n  ~/.localhostai/config.toml   settings\n\
  ~/.localhostai/pair_token    shared secret for the phone\n\
  ~/.localhostai/models/       GGUF files\n\
\nSet LOCALHOSTAI_TOKEN when connecting to a node that requires pairing.",
        env!("CARGO_PKG_VERSION")
    );
}

async fn once(node: Node, prompt: String) -> Result<()> {
    let health = node.health().await?;
    if !health.ok {
        bail!("no model is loaded; run `localhostai`, choose a model in the TUI, and press Enter");
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    let model = health.models.first().cloned();
    let task = tokio::spawn(async move {
        node.chat_stream(model.as_deref(), vec![("user".into(), prompt)], None, tx)
            .await
    });
    while let Some(event) = rx.recv().await {
        match event {
            ApiEvent::Chunk(text) => {
                print!("{text}");
                io::stdout().flush()?;
            }
            ApiEvent::Done | ApiEvent::Failed(_) => break,
        }
    }
    println!();
    // A failed HTTP request, stream, or task must produce a nonzero exit code.
    task.await??;
    Ok(())
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
    }
}

async fn run_tui(node: Node) -> Result<()> {
    use std::io::IsTerminal;
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        bail!("localhost.ai's chat UI needs a terminal.\nTry: localhostai --status or localhostai --once \"hello\"");
    }
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    event_loop(&mut terminal, node).await
}

struct Snapshot {
    health: Option<Health>,
    devices: Option<Vec<DeviceRow>>,
    cluster: Option<Value>,
    models: Option<Vec<String>>,
    managed: Option<Value>,
}

async fn snapshot(node: &Node) -> Snapshot {
    let (health, devices, cluster, models, managed) = tokio::join!(
        node.health(),
        node.get_json("/peers"),
        node.get_json("/cluster"),
        node.get_json("/models"),
        node.get_json("/managed")
    );
    Snapshot {
        health: health.ok(),
        devices: devices.ok().and_then(|v| {
            v["devices"]
                .as_array()
                .map(|a| a.iter().map(DeviceRow::from_json).collect())
        }),
        cluster: cluster.ok(),
        models: models.ok().and_then(|v| {
            v["models"].as_array().map(|a| {
                a.iter()
                    .filter_map(|m| m["name"].as_str().map(String::from))
                    .collect()
            })
        }),
        managed: managed.ok(),
    }
}

fn apply_snapshot(app: &mut App, update: Snapshot) {
    if let Some(h) = update.health {
        app.node_label = h.name;
        app.backend_label = h.backend;
        app.servable_models = h.models;
        if !app
            .model
            .as_ref()
            .is_some_and(|m| app.servable_models.contains(m))
        {
            app.model = app.servable_models.first().cloned();
        }
    } else {
        app.backend_label = "unreachable".into();
    }
    if let Some(models) = update.models {
        app.plannable_models = models;
    }
    if let Some(managed) = update.managed {
        app.runtime_ready = managed["runtime_ready"].as_bool().unwrap_or(false);
        app.catalog = managed["catalog"]
            .as_array()
            .map(|models| models.iter().filter_map(CatalogRow::from_json).collect())
            .unwrap_or_default();
        if let Some(custom) = managed["custom"].as_array() {
            app.catalog
                .extend(custom.iter().filter_map(CatalogRow::from_json));
        }
        let status = &managed["status"];
        app.setup_phase = status["phase"].as_str().unwrap_or("idle").to_string();
        app.setup_model = status["model"].as_str().map(String::from);
        app.setup_job = status["job_id"].as_str().map(String::from);
        app.setup_message = status["message"].as_str().map(String::from);
        app.setup_downloaded = status["downloaded_bytes"].as_u64().unwrap_or(0);
        app.setup_total = status["total_bytes"].as_u64().unwrap_or(0);
        app.setup_busy = matches!(
            app.setup_phase.as_str(),
            "queued"
                | "preparing"
                | "resolving_model"
                | "downloading_runtime"
                | "installing_runtime"
                | "verifying_model"
                | "downloading_model"
        );
        if app.setup_busy {
            let progress = if app.setup_total > 0 {
                format!(
                    " {:.1}% ({:.0}/{:.0} MiB)",
                    app.setup_downloaded as f64 * 100.0 / app.setup_total as f64,
                    app.setup_downloaded as f64 / 1_048_576.0,
                    app.setup_total as f64 / 1_048_576.0,
                )
            } else {
                String::new()
            };
            app.notice = Some(format!(
                "{}{}",
                status["message"]
                    .as_str()
                    .unwrap_or("Preparing local model"),
                progress
            ));
            app.activity_message = app.notice.clone().unwrap_or_default();
            app.operation_busy = true;
        } else if matches!(app.setup_phase.as_str(), "failed" | "cancelled") {
            app.notice = status["message"].as_str().map(String::from);
            app.activity_message = app
                .notice
                .clone()
                .unwrap_or_else(|| "Installation failed".into());
            app.activity_error = true;
            app.operation_busy = false;
        }
    }
    let selected = app.selected_model.and_then(|i| app.models.get(i)).cloned();
    app.models = app.servable_models.clone();
    for model in &app.plannable_models {
        if !app.models.contains(model) {
            app.models.push(model.clone());
        }
    }
    for model in &app.catalog {
        if !app.models.contains(&model.id) {
            app.models.push(model.id.clone());
        }
    }
    app.selected_model = selected
        .as_ref()
        .and_then(|m| app.models.iter().position(|s| s == m))
        .or_else(|| (!app.models.is_empty()).then_some(0));
    if let Some(devices) = update.devices {
        let selected = app
            .selected_device
            .and_then(|index| app.devices.get(index))
            .map(|device| device.id.clone());
        app.devices = devices;
        app.selected_device = selected
            .as_ref()
            .and_then(|id| app.devices.iter().position(|device| &device.id == id))
            .or_else(|| (app.overlay == Overlay::Devices && !app.devices.is_empty()).then_some(0));
    } else {
        for device in &mut app.devices {
            device.usable = false;
            device.reachable = false;
        }
    }
    if let Some(cluster) = update.cluster {
        app.cluster_state = cluster["state"].as_str().unwrap_or("stopped").to_string();
        app.cluster_model = cluster["model"].as_str().map(String::from);
        app.cluster_error = cluster["error"].as_str().map(String::from);
        app.cluster_log = cluster["log"].as_str().map(String::from);
        app.cluster_assignments =
            serde_json::from_value(cluster["assignments"].clone()).unwrap_or_default();
        app.cluster_label = match app.cluster_state.as_str() {
            "running" => format!("serving {}", cluster["model"].as_str().unwrap_or("model")),
            "starting" => "starting…".into(),
            "failed" => "cluster failed".into(),
            _ => "local".into(),
        };
        if let Some(error) = cluster["error"].as_str() {
            app.notice = Some(error.into());
            app.activity_message = error.into();
            app.activity_error = true;
            app.operation_busy = false;
        }
    }
}

fn prepare_first_run(app: &mut App) {
    if app.servable_models.is_empty() && !app.models.is_empty() {
        let installed = app
            .catalog
            .iter()
            .find(|model| model.installed)
            .map(|model| model.id.as_str())
            .or_else(|| app.plannable_models.first().map(String::as_str));
        let recommended = app
            .catalog
            .iter()
            .find(|model| model.recommended)
            .map(|model| model.id.as_str());
        app.selected_model = installed
            .or(recommended)
            .and_then(|model| app.models.iter().position(|id| id == model))
            .or(Some(0));
        app.overlay = Overlay::ModelPicker;
        app.notice = Some(
            "Choose a model and press Enter, or B to find your own GGUF on Hugging Face or by URL."
                .into(),
        );
    }
}

fn request_snapshot(node: &Node, tx: &UnboundedSender<UiEvent>) {
    let node = node.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(UiEvent::Status(Box::new(snapshot(&node).await)));
    });
}

fn request_model_preview(
    node: &Node,
    tx: &UnboundedSender<UiEvent>,
    request_id: u64,
    model: String,
    ctx: u32,
) {
    let node = node.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = preview_model(&node, &model, ctx).await;
        let _ = tx.send(UiEvent::Preview {
            request_id,
            model,
            ctx,
            result,
        });
    });
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    node: Node,
) -> Result<()> {
    let mut app = App::new(node.base().into(), String::new());
    apply_snapshot(&mut app, snapshot(&node).await);
    workspace::initialize(&mut app, &node).await;
    prepare_first_run(&mut app);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let key_tx = tx.clone();
    tokio::task::spawn_blocking(move || {
        // A permanently blocking event::read prevented Tokio from exiting even
        // after Esc. Polling lets the thread notice its receiver was dropped.
        while !key_tx.is_closed() {
            match event::poll(std::time::Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(CEvent::Key(k)) if k.kind == KeyEventKind::Press => {
                        if key_tx.send(UiEvent::Key(k)).is_err() {
                            break;
                        }
                    }
                    Ok(CEvent::Paste(text)) => {
                        if key_tx.send(UiEvent::Paste(text)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                    _ => {}
                },
                Err(_) => break,
                _ => {}
            }
        }
    });
    if app.overlay == Overlay::ModelPicker {
        if let Action::Preview {
            request_id,
            model,
            ctx,
        } = app.request_preview()
        {
            request_model_preview(&node, &tx, request_id, model, ctx);
        }
    }
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(4));
    let mut workspace_ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    ticker.tick().await;
    let mut refreshing = false;
    let mut chat_generation = 0u64;
    let mut chat_task: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        terminal.draw(|f| {
            ui::draw(f, &mut app);
            match app.overlay {
                Overlay::ModelPicker => {
                    let area = ui::centered_rect(
                        96,
                        (app.models.len().saturating_add(14).min(28)) as u16,
                        f.area(),
                    );
                    ui::model_picker(f, &app, area);
                }
                Overlay::ModelBrowser => {
                    let area = ui::centered_rect(110, 28, f.area());
                    ui::model_browser(f, &app.browser, area);
                }
                Overlay::Devices => {
                    let area = ui::centered_rect(
                        112,
                        (app.devices
                            .len()
                            .saturating_mul(2)
                            .saturating_add(8)
                            .min(24)) as u16,
                        f.area(),
                    );
                    ui::devices_panel(f, &app, area);
                }
                Overlay::Pairing => {
                    let h = (app.qr_lines.len() + 8).min(u16::MAX as usize) as u16;
                    let w = app
                        .qr_lines
                        .first()
                        .map(|l| l.chars().count() + 8)
                        .unwrap_or(50)
                        .max(50)
                        .min(u16::MAX as usize) as u16;
                    let area = ui::centered_rect(w, h, f.area());
                    ui::pairing_panel(f, &app, area);
                }
                Overlay::Activity => {
                    let area = ui::centered_rect(100, 24, f.area());
                    ui::activity_panel(f, &app, area);
                }
                Overlay::Help => {
                    let area = ui::centered_rect(88, 25, f.area());
                    ui::help_panel(f, area);
                }
                Overlay::Conversations => {
                    let area = ui::centered_rect(100, 26, f.area());
                    ui::conversations_panel(f, &app, area);
                }
                Overlay::Performance => {
                    let area = ui::centered_rect(112, 30, f.area());
                    ui::performance_panel(f, &app, area);
                }
                Overlay::None => {}
            }
        })?;
        let event = tokio::select! {
            _ = workspace_ticker.tick() => { workspace::refresh(&mut app,&node,&tx); continue; }
            event = rx.recv() => match event { Some(event) => event, None => break },
            _ = ticker.tick() => {
                if !refreshing { request_snapshot(&node, &tx); refreshing = true; }
                continue;
            }
        };
        match event {
            UiEvent::ConversationReady {
                generation,
                conversation,
            } => {
                if generation == chat_generation {
                    app.workspace.current = Some(conversation);
                }
            }
            UiEvent::Workspace(update) => workspace::apply(&mut app, update),
            UiEvent::Paste(text) => app.paste(&text),
            UiEvent::Browse { request_id, result } => app.browser.apply(request_id, result),
            UiEvent::Downloaded {
                operation_id,
                result,
            } if operation_id == app.operation_id => {
                app.operation_busy = false;
                app.setup_busy = false;
                match result {
                    Ok(model) => {
                        app.activity_message =
                            "Download complete. Select the model to plan and load it.".into();
                        app.activity_error = false;
                        if !app.models.contains(&model.name) {
                            app.models.push(model.name.clone());
                        }
                        if !app.plannable_models.contains(&model.name) {
                            app.plannable_models.push(model.name.clone());
                        }
                        app.selected_model = app.models.iter().position(|id| id == &model.name);
                        if app.overlay == Overlay::Activity {
                            app.overlay = Overlay::ModelPicker;
                        }
                        if let Action::Preview {
                            request_id,
                            model,
                            ctx,
                        } = app.request_preview()
                        {
                            request_model_preview(&node, &tx, request_id, model, ctx);
                        }
                    }
                    Err(error) => {
                        app.activity_message = format!("Download failed: {error:#}");
                        app.activity_error = true;
                    }
                }
                app.notice = Some(app.activity_message.clone());
                if !refreshing {
                    request_snapshot(&node, &tx);
                    refreshing = true;
                }
            }
            UiEvent::Status(update) => {
                apply_snapshot(&mut app, *update);
                refreshing = false;
            }
            UiEvent::Pair(result) => match result {
                Ok(uri) => match localhostai_core::qr::encode(&uri) {
                    Some(grid) => {
                        app.qr_lines = localhostai_core::qr::render_terminal(&grid, 4);
                        app.pair_uri = uri;
                    }
                    None => app.notice = Some("Pairing payload is too large to encode".into()),
                },
                Err(e) => {
                    app.notice = Some(format!("Could not read pairing info: {e}"));
                    app.overlay = Overlay::None;
                }
            },
            UiEvent::DeviceToggle(result) => {
                app.notice = Some(match result {
                    Ok(message) => message,
                    Err(error) => format!("Could not update device: {error}"),
                });
                if !refreshing {
                    request_snapshot(&node, &tx);
                    refreshing = true;
                }
            }
            UiEvent::Preview {
                request_id,
                model,
                ctx,
                result,
            } => {
                if request_id == app.preview_request_id
                    && app
                        .selected_model
                        .and_then(|index| app.models.get(index))
                        .is_some_and(|selected| selected == &model)
                    && app.context_tokens == ctx
                {
                    app.preview_pending = false;
                    match result {
                        Ok(plan) => {
                            app.preview = Some(plan);
                            app.preview_error = None;
                        }
                        Err(error) => {
                            app.preview = None;
                            app.preview_error = Some(format!("{error:#}"));
                        }
                    }
                }
            }
            UiEvent::Installed {
                operation_id,
                model,
                result,
            } if operation_id == app.operation_id => {
                app.operation_busy = false;
                app.setup_busy = false;
                match result {
                    Ok(()) => {
                        app.activity_message = format!("Installed and verified {model}");
                        app.activity_error = false;
                        app.notice = Some(app.activity_message.clone());
                    }
                    Err(error) => {
                        app.activity_message = format!("Install failed: {error:#}");
                        app.activity_error = true;
                        app.notice = Some(app.activity_message.clone());
                    }
                }
                if !refreshing {
                    request_snapshot(&node, &tx);
                    refreshing = true;
                }
            }
            UiEvent::Cluster {
                operation_id,
                model,
                notice,
            } if operation_id == app.operation_id => {
                app.cluster_starting = false;
                app.setup_busy = false;
                app.operation_busy = false;
                app.activity_error = model.is_none();
                app.activity_message = notice.clone();
                app.notice = Some(notice);
                if let Some(model) = model {
                    app.model = Some(model);
                }
                if !refreshing {
                    request_snapshot(&node, &tx);
                    refreshing = true;
                }
            }
            UiEvent::Stopped {
                operation_id,
                result,
            } if operation_id == app.operation_id => {
                app.operation_busy = false;
                app.cluster_starting = false;
                match result {
                    Ok(message) => {
                        app.activity_message = message.clone();
                        app.activity_error = false;
                        app.notice = Some(message);
                    }
                    Err(error) => {
                        app.activity_message = format!("Could not stop model: {error:#}");
                        app.activity_error = true;
                        app.notice = Some(app.activity_message.clone());
                    }
                }
                if !refreshing {
                    request_snapshot(&node, &tx);
                    refreshing = true;
                }
            }
            UiEvent::Api { generation, event } if generation == chat_generation => match event {
                ApiEvent::Chunk(text) => app.streaming.push_str(&text),
                ApiEvent::Done => {
                    app.workspace.invalidate();
                    app.workspace.draft.clear();
                    app.messages.push(Msg {
                        role: "assistant".into(),
                        content: std::mem::take(&mut app.streaming),
                        tag: app.model.clone(),
                    });
                    app.busy = false;
                    app.activity_message = "Response complete".into();
                    app.activity_error = false;
                    chat_task = None;
                }
                ApiEvent::Failed(error) => {
                    app.workspace.invalidate();
                    if app.input.is_empty() {
                        app.input = std::mem::take(&mut app.workspace.draft);
                        app.input_cursor = app.input.len();
                    }
                    if !app.streaming.is_empty() {
                        app.messages.push(Msg {
                            role: "assistant".into(),
                            content: std::mem::take(&mut app.streaming),
                            tag: Some("interrupted".into()),
                        });
                    }
                    app.notice = Some(error.clone());
                    app.busy = false;
                    app.activity_message = error;
                    app.activity_error = true;
                    chat_task = None;
                }
            },
            UiEvent::Api { .. } => {
                // A cancelled generation may still have queued terminal events.
            }
            UiEvent::Installed { .. }
            | UiEvent::Cluster { .. }
            | UiEvent::Stopped { .. }
            | UiEvent::Downloaded { .. } => {
                // A newer operation superseded this result.
            }
            UiEvent::Key(key) => {
                match app.key(key) {
                    Action::Workspace(action) => workspace::launch(&mut app, action, &node, &tx),
                    Action::Quit => break,
                    Action::None => {}
                    Action::Refresh => {
                        if !refreshing {
                            request_snapshot(&node, &tx);
                            refreshing = true;
                        }
                    }
                    Action::Pair => {
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result = node.get_json("/pair").await.and_then(|v| {
                                let uri = v["uri"].as_str().unwrap_or("").to_string();
                                if uri.is_empty() {
                                    bail!("node did not return a pairing URI");
                                }
                                Ok(uri)
                            });
                            let _ = tx.send(UiEvent::Pair(result));
                        });
                    }
                    Action::Preview {
                        request_id,
                        model,
                        ctx,
                    } => request_model_preview(&node, &tx, request_id, model, ctx),
                    Action::Browse { request_id, query } => {
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result = node
                                .post_json("/models/browse", &json!({"query":query}))
                                .await
                                .and_then(|value| {
                                    serde_json::from_value(value)
                                        .context("invalid model search response")
                                });
                            let _ = tx.send(UiEvent::Browse { request_id, result });
                        });
                    }
                    Action::Download { source, name } => {
                        app.operation_id = app.operation_id.wrapping_add(1);
                        let operation_id = app.operation_id;
                        app.operation_busy = true;
                        app.setup_busy = true;
                        app.activity_error = false;
                        app.activity_message = format!("Downloading {name}");
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result =
                                install_job(&node, "/managed/download", json!({"source":source}))
                                    .await
                                    .and_then(|value| {
                                        serde_json::from_value(value["result"].clone())
                                            .context("invalid downloaded model response")
                                    });
                            let _ = tx.send(UiEvent::Downloaded {
                                operation_id,
                                result,
                            });
                        });
                    }
                    Action::InstallOnly(model) => {
                        app.operation_id = app.operation_id.wrapping_add(1);
                        let operation_id = app.operation_id;
                        app.operation_busy = true;
                        app.activity_error = false;
                        app.activity_message = format!("Installing {model}");
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result =
                                install_job(&node, "/managed/install", json!({"model": &model}))
                                    .await
                                    .map(|_| ());
                            let _ = tx.send(UiEvent::Installed {
                                operation_id,
                                model,
                                result,
                            });
                        });
                    }
                    Action::ToggleDevice { id, enabled } => {
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result = node
                                .post_json("/peers/toggle", &json!({"id": id, "enabled": enabled}))
                                .await
                                .map(|_| {
                                    if enabled {
                                        "Device included in future model loads".to_string()
                                    } else {
                                        "Device excluded from future model loads".to_string()
                                    }
                                });
                            let _ = tx.send(UiEvent::DeviceToggle(result));
                        });
                    }
                    Action::Start { model, ctx } => {
                        app.operation_id = app.operation_id.wrapping_add(1);
                        let operation_id = app.operation_id;
                        app.cluster_starting = true;
                        app.operation_busy = true;
                        app.activity_error = false;
                        app.setup_busy = app
                            .catalog_model(&model)
                            .is_some_and(|entry| !entry.installed || !app.runtime_ready);
                        app.notice = Some(if app.setup_busy {
                            format!("Preparing {model} for first use…")
                        } else {
                            format!("Planning and loading {model} at {ctx} tokens…")
                        });
                        app.activity_message = app.notice.clone().unwrap_or_default();
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result = start_distributed(&node, &model, ctx).await;
                            let (model, notice) = match result {
                                Ok(summary) => (Some(model), summary),
                                Err(e) => (None, format!("Could not start: {e}")),
                            };
                            let _ = tx.send(UiEvent::Cluster {
                                operation_id,
                                model,
                                notice,
                            });
                        });
                    }
                    Action::StopCluster => {
                        app.operation_id = app.operation_id.wrapping_add(1);
                        let operation_id = app.operation_id;
                        app.overlay = Overlay::Activity;
                        app.operation_busy = true;
                        app.activity_error = false;
                        app.activity_message = "Stopping the active model…".into();
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result = stop_cluster(&node).await;
                            let _ = tx.send(UiEvent::Stopped {
                                operation_id,
                                result,
                            });
                        });
                    }
                    Action::CancelChat => {
                        if let Some(chat) = &app.workspace.current {
                            if let Some(request_id) = &chat.active_request {
                                let node = node.clone();
                                let id = chat.id.clone();
                                let request_id = request_id.clone();
                                tokio::spawn(async move {
                                    let _ = node
                                        .post_json(
                                            &format!("/conversations/{id}/cancel"),
                                            &json!({"request_id":request_id}),
                                        )
                                        .await;
                                });
                            }
                        }
                        chat_generation = chat_generation.wrapping_add(1);
                        if let Some(task) = chat_task.take() {
                            task.abort();
                        }
                        if !app.streaming.is_empty() {
                            app.messages.push(Msg {
                                role: "assistant".into(),
                                content: std::mem::take(&mut app.streaming),
                                tag: Some("interrupted".into()),
                            });
                        }
                        app.busy = false;
                        app.notice = Some("Generation stopped".into());
                        app.activity_message = "Generation stopped".into();
                        app.activity_error = false;
                    }
                    Action::CancelInstall(job_id) => {
                        app.activity_message = "Cancelling installation...".into();
                        let operation_id = app.operation_id;
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result = node
                                .post_json("/managed/cancel", &json!({"job_id":job_id}))
                                .await
                                .and_then(|value| {
                                    if value["ok"].as_bool() == Some(true) {
                                        Ok("Installation cancelled; partial data was retained"
                                            .into())
                                    } else {
                                        bail!("installation was not cancelled")
                                    }
                                });
                            let _ = tx.send(UiEvent::Stopped {
                                operation_id,
                                result,
                            });
                        });
                    }
                    Action::DeleteModel(model) => {
                        app.operation_id = app.operation_id.wrapping_add(1);
                        let operation_id = app.operation_id;
                        app.operation_busy = true;
                        app.activity_message = format!("Deleting {model}...");
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result = node
                                .post_json("/managed/delete", &json!({"model":model}))
                                .await
                                .map(|_| "Model deleted from this laptop".into());
                            let _ = tx.send(UiEvent::Stopped {
                                operation_id,
                                result,
                            });
                        });
                    }
                    Action::ResetRuntime => {
                        app.operation_id = app.operation_id.wrapping_add(1);
                        let operation_id = app.operation_id;
                        app.operation_busy = true;
                        app.activity_message = "Resetting the managed runtime...".into();
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let result = node.post_json("/managed/runtime/reset", &json!({})).await
                            .map(|_| "Runtime reset; the next model load installs a fresh pinned copy".into());
                            let _ = tx.send(UiEvent::Stopped {
                                operation_id,
                                result,
                            });
                        });
                    }
                    Action::Send => {
                        let content = app.input.clone();
                        app.workspace.invalidate();
                        app.workspace.draft = content.clone();
                        let supported = app.workspace.supported;
                        let mut conversation = app.workspace.current.clone();
                        if let Some(chat) = &mut conversation {
                            chat.model = app.model.clone();
                            chat.ctx = app.context_tokens;
                            chat.system_prompt = app.system_prompt.clone();
                        }
                        app.messages.push(Msg {
                            role: "user".into(),
                            content: std::mem::take(&mut app.input),
                            tag: None,
                        });
                        app.input_cursor = 0;
                        app.busy = true;
                        app.streaming.clear();
                        app.notice = None;
                        app.activity_message = "Generating response…".into();
                        app.activity_error = false;
                        app.scroll_to_bottom();
                        chat_generation = chat_generation.wrapping_add(1);
                        let generation = chat_generation;
                        let node = node.clone();
                        let history = app.history();
                        let model = app.model.clone();
                        let system = app.system_prompt.clone();
                        let ctx = app.context_tokens;
                        let ready_tx = tx.clone();
                        let (api_tx, api_rx) = mpsc::unbounded_channel();
                        chat_task = Some(tokio::spawn(async move {
                            if supported {
                                let result: Result<()> = async {
                                    let mut chat = match conversation {
                                        Some(chat) => chat,
                                        None => serde_json::from_value::<
                                            localhostai_core::conversations::Conversation,
                                        >(
                                            node.post_json("/conversations", &json!({})).await?
                                                ["conversation"]
                                                .clone(),
                                        )?,
                                    };
                                    chat.model = model.clone();
                                    chat.ctx = ctx;
                                    chat.system_prompt = system.clone();
                                    let _ = ready_tx.send(UiEvent::ConversationReady {
                                        generation,
                                        conversation: chat.clone(),
                                    });
                                    let _ = node
                                        .conversation_stream(&chat, &content, api_tx.clone())
                                        .await;
                                    Ok(())
                                }
                                .await;
                                if let Err(error) = result {
                                    let _ = api_tx.send(ApiEvent::Failed(format!("{error:#}")));
                                }
                                return;
                            }
                            // The core client emits exactly one Failed event for
                            // every error, including transport failures.
                            let _ = node
                                .chat_stream(model.as_deref(), history, Some(&system), api_tx)
                                .await;
                        }));
                        tokio::spawn(forward(api_rx, tx.clone(), generation));
                    }
                }
            }
        }
    }
    if let Some(task) = chat_task {
        task.abort();
    }
    Ok(())
}

async fn preview_model(node: &Node, model: &str, ctx: u32) -> Result<SplitPlan> {
    let value = node
        .post_json("/cluster/plan", &json!({"model": model, "ctx": ctx}))
        .await?;
    serde_json::from_value(value["plan"].clone()).context("node returned an invalid split plan")
}

async fn start_distributed(node: &Node, model: &str, ctx: u32) -> Result<String> {
    let preview = preview_model(node, model, ctx).await?;
    if !preview.fits() {
        bail!(preview.summary);
    }
    let managed = node.get_json("/managed").await?;
    let catalog_entry = managed["catalog"].as_array().and_then(|catalog| {
        catalog
            .iter()
            .find(|entry| entry["id"].as_str() == Some(model))
    });
    let custom_entry = managed["custom"].as_array().and_then(|custom| {
        custom
            .iter()
            .find(|entry| entry["id"].as_str() == Some(model))
    });
    if catalog_entry.is_some() || custom_entry.is_some() {
        install_job(node, "/managed/install", json!({"model": model})).await?;
    }
    let request = json!({"model": model, "ctx": ctx});
    let plan = node.post_json("/cluster/plan", &request).await?;
    let summary = plan["plan"]["summary"]
        .as_str()
        .unwrap_or("No plan returned");
    if plan["plan"]["over_budget"].as_bool() != Some(false) {
        bail!("{summary}");
    }
    let result = node.post_json("/cluster/start", &request).await?;
    if result["ok"].as_bool() != Some(true) {
        bail!(
            "{}",
            result["status"]["error"]
                .as_str()
                .or_else(|| result["error"].as_str())
                .unwrap_or("cluster failed")
        );
    }
    Ok(summary.into())
}

async fn install_job(node: &Node, path: &str, mut body: Value) -> Result<Value> {
    body["async"] = json!(true);
    let accepted = node.post_json(path, &body).await?;
    let id = accepted["job"]["id"]
        .as_str()
        .context("node did not return an installation job")?
        .to_string();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(7200);
    loop {
        if tokio::time::Instant::now() >= deadline {
            bail!("installation exceeded its two-hour operation deadline; retry to resume");
        }
        let value = node.get_json(&format!("/managed/jobs/{id}")).await?;
        match value["job"]["state"].as_str().unwrap_or("") {
            "succeeded" => return Ok(value["job"].clone()),
            "failed" | "cancelled" => bail!(
                "{}",
                value["job"]["error"]
                    .as_str()
                    .unwrap_or("installation failed")
            ),
            _ => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
        }
    }
}

async fn stop_cluster(node: &Node) -> Result<String> {
    let value = node.post_json("/cluster/stop", &json!({})).await?;
    if value["ok"].as_bool() != Some(true) {
        bail!("node did not confirm model shutdown");
    }
    Ok("Active model unloaded".into())
}

async fn forward(
    mut from: UnboundedReceiver<ApiEvent>,
    to: UnboundedSender<UiEvent>,
    generation: u64,
) {
    while let Some(event) = from.recv().await {
        if to.send(UiEvent::Api { generation, event }).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_catalog_populates_first_run_picker_and_progress() {
        let mut app = App::new("http://node".into(), String::new());
        apply_snapshot(
            &mut app,
            Snapshot {
                health: Some(Health {
                    name: "node".into(),
                    backend: "none".into(),
                    ..Default::default()
                }),
                devices: None,
                cluster: None,
                models: Some(vec![]),
                managed: Some(json!({
                    "status": {
                        "phase": "downloading_model",
                        "downloaded_bytes": 50,
                        "total_bytes": 100,
                        "message": "Downloading model"
                    },
                    "catalog": [{
                        "id": "small-model",
                        "display_name": "Small model",
                        "description": "For first use",
                        "bytes": 100,
                        "license": "Apache-2.0",
                        "installed": false,
                        "recommended": true
                    }]
                })),
            },
        );
        assert_eq!(app.models, vec!["small-model"]);
        assert!(app.setup_busy);
        assert!(app.notice.as_deref().unwrap().contains("50.0%"));
        prepare_first_run(&mut app);
        assert_eq!(app.overlay, Overlay::ModelPicker);
        assert_eq!(app.selected_model, Some(0));
    }

    #[test]
    fn snapshot_retains_activity_and_cluster_diagnostics() {
        let mut app = App::new("http://node".into(), String::new());
        apply_snapshot(
            &mut app,
            Snapshot {
                health: None,
                devices: None,
                models: None,
                managed: Some(json!({
                    "runtime_ready": true,
                    "status": {
                        "phase": "downloading_model",
                        "model": "large",
                        "message": "Downloading large",
                        "downloaded_bytes": 25,
                        "total_bytes": 100
                    },
                    "catalog": []
                })),
                cluster: Some(json!({
                    "state": "starting",
                    "model": "large",
                    "error": null,
                    "log": "/tmp/cluster.log",
                    "assignments": [{
                        "device_id": "laptop",
                        "device_name": "Laptop",
                        "kind": "laptop",
                        "layers": 25,
                        "memory_mb": 1000,
                        "budget_mb": 1200,
                        "rpc": null
                    }]
                })),
            },
        );
        assert_eq!(app.setup_model.as_deref(), Some("large"));
        assert_eq!(app.setup_message.as_deref(), Some("Downloading large"));
        assert_eq!(app.cluster_state, "starting");
        assert_eq!(app.cluster_model.as_deref(), Some("large"));
        assert_eq!(app.cluster_assignments[0].layers, 25);
        assert_eq!(app.cluster_log.as_deref(), Some("/tmp/cluster.log"));
        assert!(app.operation_busy);
    }
}
