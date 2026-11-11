//! Ratatui drawing for the PrivateLM-LAN chat client.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Msg};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn draw(f: &mut Frame, app: &mut App) {
    let [status_area, notice_area, chat_area, input_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(if app.notice.is_some() { 3 } else { 0 }),
        Constraint::Min(3),
        Constraint::Length(3),
    ])
    .areas(f.area());

    draw_status(f, app, status_area);
    if let Some(notice) = &app.notice {
        f.render_widget(
            Paragraph::new(notice.as_str())
                .style(Style::default().fg(Color::Yellow))
                .wrap(Wrap { trim: false }),
            notice_area,
        );
    }
    draw_chat(f, app, chat_area);
    draw_input(f, app, input_area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let model = app.model.clone().unwrap_or_else(|| "no model".into());
    let spans = vec![
        Span::styled(
            format!(" {} ", app.node_label),
            Style::default().fg(Color::Black).bg(Color::Green),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{} · {}", app.backend_label, model),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(
            format!(" {} ", app.cluster_label),
            if app.cluster_label.starts_with("serving") {
                Style::default().fg(Color::Black).bg(Color::Magenta)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::raw("  ↑ models · F1 help · Ctrl+A activity"),
    ];
    let activity_style = if app.activity_error {
        Style::default().fg(Color::Red)
    } else if app.operation_busy || app.setup_busy || app.cluster_starting || app.busy {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(spans),
            Line::from(vec![
                Span::styled(" activity ", activity_style.add_modifier(Modifier::BOLD)),
                Span::styled(
                    truncate(
                        &app.activity_message.replace('\n', " "),
                        (area.width as usize).saturating_sub(12),
                    ),
                    activity_style,
                ),
            ]),
        ]),
        area,
    );
}

fn draw_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    for msg in &app.messages {
        items.extend(
            bubble(msg, area.width.saturating_sub(2))
                .into_iter()
                .map(ListItem::new),
        );
    }
    if !app.streaming.is_empty() {
        let streaming_msg = Msg {
            role: "assistant".into(),
            content: app.streaming.clone(),
            tag: Some("streaming…".into()),
        };
        items.extend(
            bubble(&streaming_msg, area.width.saturating_sub(2))
                .into_iter()
                .map(ListItem::new),
        );
    }
    if app.messages.is_empty() && app.streaming.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " Empty — type a message below. Inference runs on the laptop node.",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    app.chat_rows = items.len();
    app.chat_page_rows = area.height.saturating_sub(2).max(1) as usize;
    if app.chat_follow {
        app.list_state.select(items.len().checked_sub(1));
    } else if let Some(selected) = app.list_state.selected() {
        app.list_state
            .select(Some(selected.min(items.len().saturating_sub(1))));
    }
    let scroll = if app.chat_follow {
        ""
    } else {
        " · scroll paused · PgDn latest"
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" chat · {}{scroll} ", app.base_url)),
        )
        .highlight_symbol("");
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn bubble(msg: &Msg, width: u16) -> Vec<Line<'static>> {
    let (role, color) = match msg.role.as_str() {
        "user" => ("you", Color::Blue),
        _ => ("assistant", Color::Green),
    };
    let mut lines = vec![Line::from(Span::styled(
        role.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))];
    for raw in msg.content.split('\n') {
        lines.extend(wrap(raw, width as usize));
    }
    if let Some(tag) = &msg.tag {
        lines.push(Line::from(Span::styled(
            format!(" {tag}"),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

/// List items contain actual lines, not embedded newlines inside one Line.
/// Grapheme widths preserve Unicode and long unbroken tokens in narrow terminals.
fn wrap(text: &str, max: usize) -> Vec<Line<'static>> {
    let max = max.max(1);
    let mut lines = Vec::new();
    let mut out = String::new();
    let mut line_len = 0;
    for grapheme in text.graphemes(true) {
        let width = grapheme.width();
        if line_len > 0 && line_len + width > max {
            lines.push(Line::from(std::mem::take(&mut out)));
            line_len = 0;
        }
        out.push_str(grapheme);
        line_len += width;
    }
    lines.push(Line::from(out));
    lines
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.setup_busy {
        " input (installing local model…) "
    } else if app.cluster_starting {
        " input (loading model…) "
    } else if app.busy {
        " input (generating…) "
    } else {
        " input "
    };
    let available = area.width.saturating_sub(3) as usize;
    let (visible, cursor_column) = input_window(&app.input, app.input_cursor, available);
    let input =
        Paragraph::new(visible.as_str()).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(input, area);
    if app.overlay == crate::app::Overlay::None
        && !app.busy
        && !app.cluster_starting
        && !app.setup_busy
        && area.width >= 3
        && area.height >= 2
    {
        f.set_cursor_position((area.x + 1 + cursor_column as u16, area.y + 1));
    }
}

fn input_window(input: &str, cursor: usize, max_width: usize) -> (String, usize) {
    if max_width == 0 {
        return (String::new(), 0);
    }
    let mut cursor = cursor.min(input.len());
    while cursor > 0 && !input.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let right_exists = cursor < input.len();
    let left_budget = if right_exists {
        max_width.saturating_sub((max_width / 3).max(1))
    } else {
        max_width
    };
    let mut start = cursor;
    let mut left_width = 0;
    for (index, grapheme) in input[..cursor].grapheme_indices(true).rev() {
        let width = grapheme.width();
        if left_width + width > left_budget {
            break;
        }
        start = index;
        left_width += width;
    }
    let mut visible = String::new();
    let mut width = 0;
    for grapheme in input[start..].graphemes(true) {
        let grapheme_width = grapheme.width();
        if width + grapheme_width > max_width {
            break;
        }
        visible.push_str(grapheme);
        width += grapheme_width;
    }
    (visible, input[start..cursor].width())
}

pub fn model_picker(f: &mut Frame, app: &App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            format!(
                " {} device(s) · {:.1} GiB shared ",
                app.devices.iter().filter(|d| d.usable).count(),
                app.pool_mb() as f64 / 1024.0
            ),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::styled(
            format!("  context {} tokens  ←/→", app.context_tokens),
            Style::default().fg(Color::Yellow),
        ),
    ])));
    for (i, m) in app.models.iter().enumerate() {
        let marker = if Some(i) == app.selected_model {
            "▸ "
        } else {
            "  "
        };
        let style = if Some(i) == app.selected_model {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
        };
        let (label, hint) = if app.servable_models.contains(m) {
            (m.as_str(), " (ready)".to_string())
        } else if let Some(model) = app.catalog_model(m) {
            let size = model.bytes as f64 / 1_073_741_824.0;
            let hint = if model.installed {
                " (installed · Enter to load)".to_string()
            } else if model.recommended {
                format!(" ({size:.2} GiB · {} · recommended)", model.license)
            } else {
                format!(" ({size:.2} GiB · {})", model.license)
            };
            (model.display_name.as_str(), hint)
        } else {
            (m.as_str(), " (local GGUF · Enter to load)".to_string())
        };
        items.push(ListItem::new(Line::from(Span::styled(
            format!("{marker}{label}{hint}"),
            style,
        ))));
    }
    if let Some(model) = app
        .selected_model
        .and_then(|index| app.models.get(index))
        .and_then(|id| app.catalog_model(id))
    {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  {}", model.description),
            Style::default().fg(Color::DarkGray),
        ))));
    }
    items.push(ListItem::new(Line::from("")));
    let selected = app.selected_model.and_then(|index| app.models.get(index));
    if app.preview_pending {
        items.push(ListItem::new(Line::from(Span::styled(
            "  calculating placement…",
            Style::default().fg(Color::Yellow),
        ))));
    } else if let Some(plan) = app.preview.as_ref().filter(|plan| {
        selected.is_some_and(|model| model == &plan.model) && plan.ctx == app.context_tokens
    }) {
        items.push(ListItem::new(Line::from(Span::styled(
            if plan.fits() {
                "  FITS CURRENT POOL"
            } else {
                "  DOES NOT FIT"
            },
            Style::default()
                .fg(if plan.fits() {
                    Color::Green
                } else {
                    Color::Red
                })
                .add_modifier(Modifier::BOLD),
        ))));
        for line in wrap(
            &format!("  {}", plan.summary),
            area.width.saturating_sub(4) as usize,
        ) {
            items.push(ListItem::new(
                line.style(Style::default().fg(Color::DarkGray)),
            ));
        }
        for assignment in &plan.assignments {
            let allocation = if assignment.layers == 0 {
                "idle".to_string()
            } else {
                format!(
                    "{}/{}L · {:.2}/{:.2} GiB",
                    assignment.layers,
                    plan.total_layers,
                    assignment.memory_mb as f64 / 1024.0,
                    assignment.budget_mb as f64 / 1024.0,
                )
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {:<22}", truncate(&assignment.device_name, 22)),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(allocation),
            ])));
        }
    } else if let Some(error) = &app.preview_error {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  preview unavailable: {}", truncate(error, 62)),
            Style::default().fg(Color::Yellow),
        ))));
    }
    items.push(ListItem::new(Line::from(Span::styled(
        "  ↑/↓ model · ←/→ context · Enter load · D reload · I install only · B browse · Esc close",
        Style::default().fg(Color::DarkGray),
    ))));

    let popup = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" models ")
            .style(Style::default().bg(Color::Black)),
    );
    f.render_widget(ratatui::widgets::Clear, area);
    let mut state = ListState::default().with_selected(app.selected_model.map(|i| i + 1));
    f.render_stateful_widget(popup, area, &mut state);
}

pub fn model_browser(f: &mut Frame, browser: &crate::model_browser::ModelBrowser, area: Rect) {
    f.render_widget(ratatui::widgets::Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" download models · Hugging Face / GGUF link ")
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(inner);
    let (input, cursor) = input_window(
        &browser.query,
        browser.cursor,
        rows[0].width.saturating_sub(3) as usize,
    );
    f.render_widget(
        Paragraph::new(input).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" search, owner/repo, or direct GGUF URL ")
                .border_style(Style::default().fg(if browser.editing {
                    Color::Cyan
                } else {
                    Color::DarkGray
                })),
        ),
        rows[0],
    );
    if browser.editing && rows[0].width > 2 && rows[0].height > 2 {
        f.set_cursor_position((rows[0].x + 1 + cursor as u16, rows[0].y + 1));
    }
    let hint = browser
        .results
        .repository
        .as_deref()
        .unwrap_or("Search any GGUF model. Empty search shows popular repositories.");
    f.render_widget(
        Paragraph::new(hint)
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: true }),
        rows[1],
    );
    if browser.pending {
        f.render_widget(
            Paragraph::new("Searching model host…").style(Style::default().fg(Color::Yellow)),
            rows[2],
        );
    } else if let Some(error) = &browser.error {
        f.render_widget(
            Paragraph::new(error.as_str())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: true }),
            rows[2],
        );
    } else if browser.rows() == 0 {
        f.render_widget(
            Paragraph::new("No results yet. Enter a search, repository, or GGUF file link.")
                .wrap(Wrap { trim: true }),
            rows[2],
        );
    } else {
        let mut items = Vec::new();
        for repo in &browser.results.repositories {
            items.push(ListItem::new(format!(
                "{} · {} downloads",
                repo.id, repo.downloads
            )));
        }
        for model in &browser.results.models {
            items.push(ListItem::new(format!(
                "{} · {:.2} GiB · {} file(s)",
                model.display_name,
                model.bytes as f64 / 1_073_741_824.0,
                model.files.len()
            )));
        }
        let list = List::new(items)
            .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan))
            .highlight_symbol("▸ ");
        let mut state = ListState::default().with_selected(browser.selected);
        f.render_stateful_widget(list, rows[2], &mut state);
    }
    f.render_widget(Paragraph::new("Enter search/open/download · ↑/↓ choose · Tab edit search · Ctrl+U clear · Esc library\nDownloads stay on the laptop. GGUF metadata and memory fit are checked before loading.")
        .style(Style::default().fg(Color::DarkGray)).wrap(Wrap { trim: true }), rows[3]);
}

/// Device pool: what each machine can contribute, and whether it really can.
pub fn devices_panel(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    let allocated_mb: u64 = app
        .devices
        .iter()
        .map(|device| device.assigned_memory_mb)
        .sum();
    let allocated_layers: u32 = app
        .devices
        .iter()
        .map(|device| device.assigned_layers)
        .sum();
    lines.push(Line::from(Span::styled(
        format!(
            "pool {:.1} GiB shared across {} device(s) · active {}L / {:.1} GiB",
            app.pool_mb() as f64 / 1024.0,
            app.devices.iter().filter(|d| d.usable).count(),
            allocated_layers,
            allocated_mb as f64 / 1024.0,
        ),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for (index, d) in app.devices.iter().enumerate() {
        // Layers go where a worker answers. A stale heartbeat is a warning,
        // not a disqualification — the worker may still be listening.
        let usable = d.usable;
        let selected = app.selected_device == Some(index);
        let (status, colour) = if !d.enabled {
            ("excluded", Color::DarkGray)
        } else if usable && d.in_use {
            ("in use by current model", Color::Green)
        } else if usable && d.stale {
            ("ready · heartbeat stale", Color::Yellow)
        } else if usable {
            ("ready", Color::Green)
        } else if d.stale {
            ("offline", Color::Red)
        } else if let Some(reason) = &d.reason {
            (reason.as_str(), Color::Yellow)
        } else if d.reachable && d.budget_mb == 0 {
            ("no free memory budget", Color::Yellow)
        } else {
            ("no reachable worker", Color::Yellow)
        };
        let allocation = if d.assigned_layers > 0 {
            format!(
                "{}L / {:.1} GiB",
                d.assigned_layers,
                d.assigned_memory_mb as f64 / 1024.0
            )
        } else {
            "idle".into()
        };
        let heartbeat = if d.is_self {
            "local".to_string()
        } else {
            format!(
                "seen {}s ago · {}",
                d.heartbeat_seconds,
                truncate(d.rpc.as_deref().unwrap_or("worker unavailable"), 28)
            )
        };
        lines.push(Line::from(vec![
            Span::styled(
                if selected { " ▸ " } else { "   " },
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                if d.enabled { "[x] " } else { "[ ] " },
                Style::default().fg(if d.enabled {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{:<18}", truncate(&d.name, 18)),
                Style::default()
                    .fg(if d.enabled {
                        Color::White
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                format!("{:<8} ", d.kind),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(status, Style::default().fg(colour)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("       "),
            Span::styled(
                format!(
                    "RAM {:.1}/{:.1} GiB free · shared cap {:.1} GiB · CPU {}t/{}c · allocation {} · {}",
                    d.available_mb as f64 / 1024.0,
                    d.ram_mb as f64 / 1024.0,
                    d.budget_mb as f64 / 1024.0,
                    d.threads,
                    d.cores,
                    allocation,
                    heartbeat,
                ),
                Style::default().fg(if d.enabled {
                    Color::Cyan
                } else {
                    Color::DarkGray
                }),
            ),
        ]));
    }

    if app.devices.len() <= 1 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " No phone has joined yet. On the Android app, scan the pairing code (Ctrl+P).",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " ↑/↓ select · Space include/exclude · changes apply on the next model load · Esc close",
        Style::default().fg(Color::DarkGray),
    )));

    let popup = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" compute pool · resource controls ")
            .style(Style::default().bg(Color::Black)),
    );
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(popup, area);
}

/// Pairing: the QR a phone scans to join.
pub fn pairing_panel(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    if app.qr_lines.is_empty() {
        lines.push(Line::from(Span::styled(
            " generating pairing code…",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for l in &app.qr_lines {
            lines.push(Line::from(Span::styled(
                l.clone(),
                Style::default().fg(Color::Black).bg(Color::White),
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            " {} ",
            if app.pair_uri.is_empty() {
                "—"
            } else {
                &app.pair_uri
            }
        ),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        " Scan in the PrivateLM app → Add node → Scan QR.",
        Style::default().fg(Color::Cyan),
    )));

    let popup = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" pair this phone · Esc to close ")
            .style(Style::default().bg(Color::Black)),
    );
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(popup, area);
}

pub fn activity_panel(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" activity · Ctrl+X stops generation/model · Esc close ")
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(area);
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(block, area);
    let show_progress = app.setup_total > 0 && app.setup_busy;
    let [body, progress] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(if show_progress { 2 } else { 0 }),
    ])
    .areas(inner);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                if app.activity_error {
                    " FAILED "
                } else if app.operation_busy {
                    " WORKING "
                } else {
                    " READY "
                },
                Style::default()
                    .fg(Color::Black)
                    .bg(if app.activity_error {
                        Color::Red
                    } else if app.operation_busy {
                        Color::Yellow
                    } else {
                        Color::Green
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}", app.activity_message)),
        ]),
        Line::from(""),
        Line::from(format!(
            " managed  {}{}",
            app.setup_phase,
            app.setup_model
                .as_ref()
                .map(|model| format!(" · {model}"))
                .unwrap_or_default()
        )),
        Line::from(format!(
            " cluster  {}{}",
            app.cluster_state,
            app.cluster_model
                .as_ref()
                .map(|model| format!(" · {model}"))
                .unwrap_or_default()
        )),
    ];
    if let Some(message) = &app.setup_message {
        lines.push(Line::from(format!(" setup    {message}")));
    }
    if app.cluster_assignments.is_empty() {
        lines.push(Line::from(" layers   none assigned"));
    } else {
        for assignment in &app.cluster_assignments {
            lines.push(Line::from(format!(
                " layers   {:<20} {:>3}L · {:.2} GiB",
                truncate(&assignment.device_name, 20),
                assignment.layers,
                assignment.memory_mb as f64 / 1024.0,
            )));
        }
    }
    if let Some(error) = &app.cluster_error {
        lines.push(Line::from(Span::styled(
            format!(" error    {error}"),
            Style::default().fg(Color::Red),
        )));
    }
    if let Some(log) = &app.cluster_log {
        lines.push(Line::from(Span::styled(
            format!(" log      {log}"),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(format!(
        " chat     {}",
        if app.busy { "generating" } else { "idle" }
    )));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
    if show_progress {
        let ratio = app.setup_downloaded as f64 / app.setup_total as f64;
        f.render_widget(
            Gauge::default()
                .ratio(ratio.clamp(0.0, 1.0))
                .label(format!(
                    "{:.1}% · {:.0}/{:.0} MiB",
                    ratio * 100.0,
                    app.setup_downloaded as f64 / 1_048_576.0,
                    app.setup_total as f64 / 1_048_576.0,
                ))
                .gauge_style(Style::default().fg(Color::Cyan)),
            progress,
        );
    }
}

pub fn help_panel(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            "MAIN",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Enter send · ↑ models · Ctrl+D devices · Ctrl+P pair"),
        Line::from("  Ctrl+A activity · Ctrl+X stop generation/model · Esc quit/close"),
        Line::from("  PageUp/PageDown scroll conversation"),
        Line::from(""),
        Line::from(Span::styled(
            "INPUT",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  ←/→ cursor · Home/End · Backspace/Delete · Ctrl+W delete word"),
        Line::from("  Bracketed paste is accepted; pasted newlines become spaces"),
        Line::from(""),
        Line::from(Span::styled(
            "MODEL PICKER",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑/↓ model · ←/→ context · Enter load · D reload"),
        Line::from("  I install/verify without loading · B browse more models"),
        Line::from("  Browser: search, owner/repo, or GGUF link · Tab edit · Enter choose"),
        Line::from(""),
        Line::from(Span::styled(
            "DEVICES",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑/↓ device · Space/Enter include or exclude"),
    ];
    let popup = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" PrivateLM help · F1 · Esc to close ")
            .style(Style::default().bg(Color::Black)),
    );
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(popup, area);
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vert = Layout::vertical([Constraint::Percentage(100)]).split(area);
    let _ = Alignment::Center;
    let v = vert[0];
    let x = v.x + (v.width.saturating_sub(width)) / 2;
    let y = v.y + (v.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(v.width), height.min(v.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn wrapped_unicode_and_long_words_keep_all_their_text() {
        let text = "你好🙂abcdefghijk";
        let lines = wrap(text, 4);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.width() <= 4));
        assert_eq!(
            lines.iter().map(ToString::to_string).collect::<String>(),
            text
        );
    }

    #[test]
    fn input_window_keeps_a_middle_unicode_cursor_visible() {
        let (visible, cursor) = input_window("ab🙂cd", 2, 4);
        assert_eq!(visible, "ab🙂");
        assert_eq!(cursor, 2);
        assert_eq!(input_window("", 99, 4), (String::new(), 0));
    }

    #[test]
    fn long_replies_scroll_to_their_end_and_errors_are_visible() {
        let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
        let mut app = App::new("http://localhost:8765".into(), String::new());
        app.messages.push(Msg {
            role: "assistant".into(),
            content: format!("{}LAST TOKEN", "long line\n".repeat(40)),
            tag: None,
        });
        app.notice = Some("Engine failed to load".into());
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("LAST TOKEN"));
        assert!(rendered.contains("Engine failed to load"));
    }

    #[test]
    fn device_panel_shows_controls_capacity_and_active_allocation() {
        let mut terminal = Terminal::new(TestBackend::new(120, 16)).unwrap();
        let mut app = App::new("http://localhost".into(), String::new());
        app.devices.push(crate::app::DeviceRow {
            id: "phone".into(),
            name: "Pixel".into(),
            kind: "phone".into(),
            ram_mb: 8000,
            available_mb: 4200,
            budget_mb: 7100,
            cores: 8,
            threads: 4,
            rpc: Some("192.168.1.5:50052".into()),
            enabled: true,
            reachable: true,
            usable: true,
            heartbeat_seconds: 4,
            assigned_layers: 12,
            assigned_memory_mb: 900,
            ..Default::default()
        });
        app.selected_device = Some(0);
        terminal
            .draw(|frame| devices_panel(frame, &app, frame.area()))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        for expected in ["[x]", "shared cap", "12L / 0.9 GiB", "seen 4s ago"] {
            assert!(
                rendered.contains(expected),
                "missing {expected}: {rendered}"
            );
        }
    }

    #[test]
    fn paused_chat_does_not_jump_when_more_rows_arrive() {
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let mut app = App::new("http://localhost".into(), String::new());
        app.messages.push(Msg {
            role: "assistant".into(),
            content: "line\n".repeat(30),
            tag: None,
        });
        app.chat_follow = false;
        app.list_state.select(Some(2));
        terminal
            .draw(|frame| draw_chat(frame, &mut app, frame.area()))
            .unwrap();
        assert_eq!(app.list_state.selected(), Some(2));
    }

    #[test]
    fn model_picker_shows_context_fit_and_assignments() {
        let mut terminal = Terminal::new(TestBackend::new(100, 22)).unwrap();
        let mut app = App::new("http://localhost".into(), String::new());
        app.models = vec!["model".into()];
        app.selected_model = Some(0);
        app.context_tokens = 8192;
        app.preview = Some(privateml_core::SplitPlan {
            model: "model".into(),
            total_layers: 28,
            ctx: 8192,
            assignments: vec![privateml_core::plan::SplitAssignment {
                worker_session: None,
                device_id: "laptop".into(),
                device_name: "Laptop".into(),
                kind: "laptop".into(),
                layers: 25,
                memory_mb: 1053,
                budget_mb: 1082,
                rpc: None,
            }],
            over_budget: false,
            shortfall_mb: 0,
            summary: "Fits on Laptop".into(),
        });
        terminal
            .draw(|frame| model_picker(frame, &app, frame.area()))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        for expected in [
            "context 8192",
            "FITS CURRENT POOL",
            "25/28L",
            "install only",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected}: {rendered}"
            );
        }
    }

    #[test]
    fn help_and_activity_panels_expose_recovery_controls() {
        let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
        let mut app = App::new("http://localhost".into(), String::new());
        app.activity_message = "Model load failed".into();
        app.activity_error = true;
        app.cluster_log = Some("/tmp/cluster.log".into());
        terminal
            .draw(|frame| activity_panel(frame, &app, frame.area()))
            .unwrap();
        let activity = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(activity.contains("Model load failed"));
        assert!(activity.contains("/tmp/cluster.log"));
        terminal
            .draw(|frame| help_panel(frame, frame.area()))
            .unwrap();
        let help = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(help.contains("Ctrl+X stop generation/model"));
        assert!(help.contains("I install/verify"));
    }
}
