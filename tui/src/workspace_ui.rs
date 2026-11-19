use crate::app::App;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use serde_json::Value;

pub fn conversations_panel(f: &mut Frame, app: &App, area: Rect) {
    let workspace = &app.workspace;
    let mut lines = vec![
        Line::from("Ctrl+N new · ↑/↓ select · Enter open · Delete remove selected"),
        Line::from("Ctrl+R rename current · Ctrl+E export Markdown · Ctrl+Y export JSON"),
        Line::from(format!(
            "{}: {}",
            if workspace.renaming {
                "New name (Enter to save)"
            } else {
                "Search names and messages"
            },
            workspace.query
        )),
        Line::from(if workspace.pending {
            "Saving…"
        } else if workspace.online {
            "Synced with coordinator"
        } else {
            "Offline — showing last snapshot"
        }),
        Line::default(),
    ];
    let page = area.height.saturating_sub(8).max(1) as usize;
    let start = workspace.selected.saturating_sub(page - 1);
    for (index, chat) in workspace.library.iter().enumerate().skip(start).take(page) {
        lines.push(Line::styled(
            format!(
                "{} {} · {} messages{}",
                if index == workspace.selected {
                    ">"
                } else {
                    " "
                },
                chat.title,
                chat.message_count,
                if chat.generating {
                    " · generating"
                } else {
                    ""
                }
            ),
            Style::default().fg(if index == workspace.selected {
                Color::Green
            } else {
                Color::White
            }),
        ));
    }
    if workspace.library.is_empty() {
        lines.push(Line::from(
            "No conversations found. Ctrl+N creates a named, shared chat.",
        ));
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Conversations · Esc back "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn number(value: &Value, unit: &str) -> String {
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .map(|n| format!("{n:.1}{unit}"))
        .unwrap_or_else(|| "not reported".into())
}
fn metrics(value: &Value) -> String {
    format!(
        "{} · TTFT {} · {} tokens · elapsed {}",
        number(&value["tokens_per_second"], " tok/s"),
        number(&value["ttft_ms"], " ms"),
        value["output_tokens"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "—".into()),
        number(&value["elapsed_ms"], " ms")
    )
}
pub fn performance_panel(f: &mut Frame, app: &App, area: Rect) {
    let state = &app.workspace;
    let data = &state.metrics;
    let benchmark = &data["benchmark"];
    let latest = if benchmark["state"] == "running" {
        &benchmark["current"]
    } else {
        &data["generations"][0]
    };
    let mut lines = vec![
        Line::from("Live measurements · refresh 1s · PgUp/PgDn scroll · Esc back"),
        Line::from(format!(
            "Coordinator HTTP round trip: {}",
            state
                .rtt_ms
                .map(|n| format!("{n:.1} ms"))
                .unwrap_or_else(|| "unavailable".into())
        )),
        Line::from(format!(
            "{} · {}",
            if state.online {
                "Connected"
            } else {
                "Offline / last snapshot"
            },
            latest["state"].as_str().unwrap_or("no replies yet")
        )),
        Line::from(metrics(latest)),
        Line::from(format!(
            "Token measurement: {}",
            latest["token_source"]
                .as_str()
                .unwrap_or("waiting for runtime timings or usage")
        )),
        Line::default(),
        Line::from("MEMORY AND LAYERS (RSS is measured; planned allocation is an estimate)"),
    ];
    if let Some(devices) = data["devices"].as_array() {
        for device in devices {
            lines.push(Line::from(format!(
                "{}: RSS {} · planned {} · {} layers · sample {}s old",
                device["name"].as_str().unwrap_or("Device"),
                number(&device["rss_mb"], " MiB"),
                number(&device["planned_mb"], " MiB"),
                device["assigned_layers"],
                device["sample_age_seconds"]
            )));
            if device["self"] == false {
                lines.push(Line::from(format!(
                    "  Phone → coordinator HTTP RTT: {}",
                    number(&device["coordinator_rtt_ms"], " ms")
                )));
            }
        }
    }
    lines.extend([
        Line::default(),
        Line::from("BENCHMARK · B start comparison · X cancel"),
        Line::from("Uses selected model/context, same prompt, seed 42, 128 output-token limit."),
        Line::from("Reloads + warms up each mode; model stops after completion. Chat waits."),
        Line::from(format!(
            "Status: {} · {}",
            benchmark["state"].as_str().unwrap_or("not run"),
            benchmark["phase"].as_str().unwrap_or("")
        )),
    ]);
    if let Some(error) = benchmark["error"].as_str() {
        lines.push(Line::styled(error, Style::default().fg(Color::Red)));
    }
    if let Some(results) = benchmark["results"].as_array() {
        for result in results {
            lines.push(Line::styled(
                format!(
                    "{}: {}",
                    result["mode"].as_str().unwrap_or(""),
                    result["state"].as_str().unwrap_or("")
                ),
                Style::default().fg(Color::Green),
            ));
            if result["state"] == "complete" {
                lines.push(Line::from(metrics(&result["metrics"])));
                lines.push(Line::from(format!(
                    "Peak laptop engine RSS {} · load {}",
                    number(&result["peak_engine_rss_mb"], " MiB"),
                    number(&result["load_ms"], " ms")
                )));
            } else {
                lines.push(Line::from(
                    result["reason"].as_str().unwrap_or("Unavailable"),
                ));
            }
        }
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Performance "),
            )
            .scroll((state.scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}
