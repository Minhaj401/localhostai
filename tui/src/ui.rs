//! Ratatui drawing for the PrivateLM-LAN chat client.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{App, Msg};

pub fn draw(f: &mut Frame, app: &mut App) {
    let [status_area, chat_area, input_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(3),
    ])
    .areas(f.area());

    draw_status(f, app, status_area);
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
        Span::raw("  ↑↓ model · Enter send · Esc quit"),
    ];
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    for msg in &app.messages {
        items.push(bubble(msg, f.area().width));
    }
    if !app.streaming.is_empty() {
        let streaming_msg = Msg {
            role: "assistant".into(),
            content: app.streaming.clone(),
            tag: Some("streaming…".into()),
        };
        items.push(bubble(&streaming_msg, f.area().width));
    }
    if app.messages.is_empty() && app.streaming.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " Empty — type a message below. Inference runs on the laptop node.",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" chat "))
        .highlight_symbol("");
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn bubble(msg: &Msg, width: u16) -> ListItem<'static> {
    let (role, color) = match msg.role.as_str() {
        "user" => ("you", Color::Blue),
        _ => ("assistant", Color::Green),
    };
    let mut lines = vec![Line::from(Span::styled(
        format!("{role}"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))];
    for raw in msg.content.split('\n') {
        lines.push(wrap(raw, width.saturating_sub(6) as usize));
    }
    if let Some(tag) = &msg.tag {
        lines.push(Line::from(Span::styled(
            format!(" {tag}"),
            Style::default().fg(Color::DarkGray),
        )));
    }
    ListItem::new(lines)
}

/// Naive word wrap at `max` chars (TUI paragraphs don't auto-wrap List items well).
fn wrap(text: &str, max: usize) -> Line<'static> {
    if text.len() <= max {
        return Line::from(text.to_string());
    }
    let mut out = String::new();
    let mut line_len = 0;
    for word in text.split_whitespace() {
        let w = word.len();
        if line_len > 0 && line_len + 1 + w > max {
            out.push('\n');
            line_len = 0;
        } else if line_len > 0 {
            out.push(' ');
            line_len += 1;
        }
        out.push_str(word);
        line_len += w;
    }
    Line::from(out)
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.busy {
        " input (generating…) "
    } else {
        " input "
    };
    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(input, area);
    if !app.busy {
        // cursor at end of input
        let x = area.x + 1 + app.input.chars().count() as u16;
        if x < area.x + area.width - 1 {
            f.set_cursor_position((x, area.y + 1));
        }
    }
}

pub fn model_picker(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .models
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let marker = if Some(i) == app.selected_model {
                "▸ "
            } else {
                "  "
            };
            ListItem::new(format!("{marker}{m}"))
        })
        .collect();
    let popup = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" select model (Enter/Esc) ")
                .style(Style::default().bg(Color::Black)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray));
    let mut state = ListState::default();
    state.select(Some(app.selected_model.unwrap_or(0)));
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_stateful_widget(popup, area, &mut state);
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vert = Layout::vertical([Constraint::Percentage(100)]).split(area);
    let _ = Alignment::Center;
    let v = vert[0];
    let x = v.x + (v.width.saturating_sub(width)) / 2;
    let y = v.y + (v.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(v.width), height.min(v.height))
}
