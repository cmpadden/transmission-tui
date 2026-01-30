use std::{
    io::{self, Stdout},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::{
    config::AppConfig,
    model::{format_bytes, format_eta, format_progress, format_speed, Snapshot, TorrentSummary},
    rpc::{RpcResult, TransmissionClient},
};

type Backend = ratatui::backend::CrosstermBackend<Stdout>;

pub fn run(config: AppConfig) -> Result<()> {
    let client = TransmissionClient::new(config.rpc.clone())
        .context("failed to construct Transmission RPC client")?;
    let mut terminal = setup_terminal()?;
    let (event_tx, event_rx) = unbounded();
    let (rpc_tx, rpc_rx) = unbounded();

    let input_handle = spawn_input_thread(event_tx.clone());
    let worker_handle = spawn_rpc_worker(client, rpc_rx, event_tx.clone(), config.poll_interval);

    let mut app = App::new(&config);
    app.set_status(StatusUpdate::info("Connecting to transmission…"));

    if rpc_tx.send(RpcCommand::Refresh).is_err() {
        app.set_status(StatusUpdate::error(
            "RPC worker not available; shutting down",
        ));
    }

    let loop_result = run_loop(&mut terminal, &mut app, event_rx, rpc_tx.clone());

    drop(rpc_tx);
    drop(event_tx);

    restore_terminal(&mut terminal)?;
    input_handle.join().ok();
    worker_handle.join().ok();

    loop_result
}

fn run_loop(
    terminal: &mut Terminal<Backend>,
    app: &mut App,
    events: Receiver<AppEvent>,
    rpc_tx: Sender<RpcCommand>,
) -> Result<()> {
    terminal.draw(|f| app.render(f))?;
    loop {
        let event = match events.recv() {
            Ok(event) => event,
            Err(_) => break,
        };
        if app.process_event(event, &rpc_tx)? {
            break;
        }
        terminal.draw(|f| app.render(f))?;
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn setup_terminal() -> Result<Terminal<Backend>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<Backend>) -> Result<()> {
    disable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, DisableBracketedPaste, LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn spawn_input_thread(tx: Sender<AppEvent>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let tick_rate = Duration::from_millis(250);
        loop {
            match event::poll(tick_rate) {
                Ok(true) => match event::read() {
                    Ok(evt) => {
                        if tx.send(AppEvent::Input(evt)).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(AppEvent::Status(StatusUpdate::error(format!(
                            "Input error: {err}"
                        ))));
                    }
                },
                Ok(false) => {
                    if tx.send(AppEvent::Tick).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    if tx.send(AppEvent::Tick).is_err() {
                        break;
                    }
                }
            }
        }
    })
}

fn spawn_rpc_worker(
    client: TransmissionClient,
    rx: Receiver<RpcCommand>,
    tx: Sender<AppEvent>,
    poll_interval: Duration,
) -> thread::JoinHandle<()> {
    thread::spawn(move || rpc_worker_loop(client, rx, tx, poll_interval))
}

fn rpc_worker_loop(
    client: TransmissionClient,
    rx: Receiver<RpcCommand>,
    tx: Sender<AppEvent>,
    poll_interval: Duration,
) {
    let poll_enabled = poll_interval > Duration::ZERO;
    if !poll_enabled {
        while let Ok(cmd) = rx.recv() {
            handle_command(&client, cmd, &tx);
        }
        return;
    }
    loop {
        match rx.recv_timeout(poll_interval) {
            Ok(cmd) => handle_command(&client, cmd, &tx),
            Err(RecvTimeoutError::Timeout) => send_snapshot(&client, &tx),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle_command(client: &TransmissionClient, cmd: RpcCommand, tx: &Sender<AppEvent>) {
    match cmd {
        RpcCommand::Refresh => send_snapshot(client, tx),
        RpcCommand::AddMagnet(magnet) => handle_add(client, magnet, tx),
        RpcCommand::RemoveTorrent {
            id,
            name,
            delete_data,
        } => handle_remove(client, id, name, delete_data, tx),
        RpcCommand::ResumeTorrent { id, name } => handle_resume(client, id, name, tx),
        RpcCommand::PauseTorrent { id, name } => handle_pause(client, id, name, tx),
    }
}

fn send_snapshot(client: &TransmissionClient, tx: &Sender<AppEvent>) {
    let result = client.fetch_snapshot();
    let _ = tx.send(AppEvent::Snapshot(result));
}

fn handle_add(client: &TransmissionClient, magnet: String, tx: &Sender<AppEvent>) {
    let trimmed = magnet.trim();
    if trimmed.is_empty() {
        let _ = tx.send(AppEvent::Status(StatusUpdate::info(
            "Ignoring empty magnet input",
        )));
        return;
    }
    match client.add_magnet(trimmed) {
        Ok(outcome) => {
            let label = outcome
                .name
                .clone()
                .unwrap_or_else(|| "torrent".to_string());
            let status = if outcome.duplicate {
                StatusUpdate::warning(format!("Magnet already present ({label})"))
            } else if outcome.added {
                StatusUpdate::success(format!("Magnet queued ({label})"))
            } else {
                StatusUpdate::success(format!("Magnet processed ({label})"))
            };
            let _ = tx.send(AppEvent::Status(status));
            if let Some(id) = outcome.torrent_id {
                let _ = tx.send(AppEvent::FocusTorrent(Some(id)));
            }
            send_snapshot(client, tx);
        }
        Err(err) => {
            let _ = tx.send(AppEvent::Status(StatusUpdate::error(format!(
                "Add failed: {err}"
            ))));
        }
    }
}

fn handle_remove(
    client: &TransmissionClient,
    id: i64,
    name: String,
    delete_data: bool,
    tx: &Sender<AppEvent>,
) {
    match client.remove_torrents(&[id], delete_data) {
        Ok(()) => {
            let _ = tx.send(AppEvent::Status(StatusUpdate::success(format!(
                "Removed {name}"
            ))));
            send_snapshot(client, tx);
        }
        Err(err) => {
            let _ = tx.send(AppEvent::Status(StatusUpdate::error(format!(
                "Remove failed: {err}"
            ))));
        }
    }
}

fn handle_resume(client: &TransmissionClient, id: i64, name: String, tx: &Sender<AppEvent>) {
    match client.start_torrents(&[id]) {
        Ok(()) => {
            let _ = tx.send(AppEvent::Status(StatusUpdate::success(format!(
                "Resumed {name}"
            ))));
            send_snapshot(client, tx);
        }
        Err(err) => {
            let _ = tx.send(AppEvent::Status(StatusUpdate::error(format!(
                "Resume failed: {err}"
            ))));
        }
    }
}

fn handle_pause(client: &TransmissionClient, id: i64, name: String, tx: &Sender<AppEvent>) {
    match client.stop_torrents(&[id]) {
        Ok(()) => {
            let _ = tx.send(AppEvent::Status(StatusUpdate::success(format!(
                "Paused {name}"
            ))));
            send_snapshot(client, tx);
        }
        Err(err) => {
            let _ = tx.send(AppEvent::Status(StatusUpdate::error(format!(
                "Pause failed: {err}"
            ))));
        }
    }
}

enum AppEvent {
    Input(Event),
    Tick,
    Snapshot(RpcResult<Snapshot>),
    Status(StatusUpdate),
    FocusTorrent(Option<i64>),
}

#[derive(Clone)]
struct StatusUpdate {
    text: String,
    level: StatusLevel,
}

impl StatusUpdate {
    fn info(message: impl Into<String>) -> Self {
        Self {
            text: message.into(),
            level: StatusLevel::Info,
        }
    }

    fn success(message: impl Into<String>) -> Self {
        Self {
            text: message.into(),
            level: StatusLevel::Success,
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            text: message.into(),
            level: StatusLevel::Warning,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            text: message.into(),
            level: StatusLevel::Error,
        }
    }
}

#[derive(Clone, Copy)]
enum StatusLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone)]
struct StatusMessage {
    text: String,
    level: StatusLevel,
    expires_at: Option<Instant>,
}

impl StatusMessage {
    fn from_update(update: StatusUpdate) -> Self {
        let duration = match update.level {
            StatusLevel::Info => Duration::from_secs(4),
            StatusLevel::Success => Duration::from_secs(5),
            StatusLevel::Warning => Duration::from_secs(6),
            StatusLevel::Error => Duration::from_secs(8),
        };
        Self {
            text: update.text,
            level: update.level,
            expires_at: Some(Instant::now() + duration),
        }
    }
}

struct App {
    connection_label: String,
    snapshot: Option<Snapshot>,
    list_state: ListState,
    filtered_indices: Vec<usize>,
    filter_text: String,
    filter_lower: String,
    pending_focus: Option<i64>,
    selected_id: Option<i64>,
    status: Option<StatusMessage>,
    toast: Option<StatusMessage>,
    mode: InputMode,
    should_quit: bool,
    pending_manual_refresh: bool,
    delete_armed: bool,
    delete_armed_until: Option<Instant>,
}

impl App {
    fn new(config: &AppConfig) -> Self {
        Self {
            connection_label: config.rpc.endpoint(),
            snapshot: None,
            list_state: ListState::default(),
            filtered_indices: Vec::new(),
            filter_text: String::new(),
            filter_lower: String::new(),
            pending_focus: None,
            selected_id: None,
            status: None,
            toast: None,
            mode: InputMode::Normal,
            should_quit: false,
            pending_manual_refresh: false,
            delete_armed: false,
            delete_armed_until: None,
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(frame.size());
        self.render_header(frame, chunks[0]);
        self.render_body(frame, chunks[1]);
        self.render_footer(frame, chunks[2]);
        self.render_toast(frame);
        match &self.mode {
            InputMode::Prompt(prompt) => {
                let area = centered_rect(60, 30, frame.size());
                let block = Block::default()
                    .title(Span::raw(format!(" {} ", prompt.title)))
                    .borders(Borders::ALL);
                let text = vec![
                    Line::from("Enter a magnet URL and press Enter (Esc to cancel)"),
                    Line::from(format!("> {}", prompt.buffer)),
                ];
                let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
                frame.render_widget(Clear, area);
                frame.render_widget(paragraph, area);
            }
            InputMode::Confirm(confirm) => {
                let area = centered_rect(50, 30, frame.size());
                let block = Block::default().title(confirm.title).borders(Borders::ALL);
                let text = vec![
                    Line::from(confirm.message.clone()),
                    Line::from(Span::styled(
                        "Press y to confirm, n or Esc to cancel",
                        Style::default().fg(Color::Yellow),
                    )),
                ];
                let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
                frame.render_widget(Clear, area);
                frame.render_widget(paragraph, area);
            }
            InputMode::Help => {
                let area = centered_rect(70, 70, frame.size());
                let block = Block::default().title("Key Bindings").borders(Borders::ALL);
                let lines = help_lines();
                let paragraph = Paragraph::new(lines)
                    .block(block)
                    .wrap(Wrap { trim: false });
                frame.render_widget(Clear, area);
                frame.render_widget(paragraph, area);
            }
            _ => {}
        }
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled(
                "Transmission",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  |  "),
            Span::raw(&self.connection_label),
        ]));
        if let Some(snapshot) = &self.snapshot {
            lines.push(Line::from(vec![Span::raw(format!(
                "DL {}  UL {}  | Active {}  Paused {}  Total {}  | Version {}",
                format_speed(snapshot.download_speed),
                format_speed(snapshot.upload_speed),
                snapshot.active_torrents,
                snapshot.paused_torrents,
                snapshot.total_torrents,
                snapshot.version
            ))]));
        } else {
            lines.push(Line::from("Waiting for session stats…"));
        }
        if let Some(status) = &self.status {
            lines.push(Line::from(Span::styled(
                status.text.clone(),
                status_style(status.level),
            )));
        }
        let paragraph = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::raw(" Session ")),
        );
        frame.render_widget(paragraph, area);
    }

    fn render_body(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);
        self.render_list(frame, chunks[0]);
        self.render_detail(frame, chunks[1]);
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let mut items = self
            .filtered_indices
            .iter()
            .filter_map(|&idx| self.snapshot.as_ref()?.torrents.get(idx))
            .map(|torrent| ListItem::new(Line::from(summary_line(torrent))))
            .collect::<Vec<_>>();
        if items.is_empty() {
            items.push(ListItem::new(Line::from("No torrents loaded")));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::raw(" Torrents "));
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().fg(Color::Yellow))
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn render_detail(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::raw(" Details "));
        if let Some(torrent) = self.current_torrent() {
            let content = vec![
                Line::from(Span::styled(
                    torrent.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(format!("Status: {}", torrent.status)),
                Line::from(format!(
                    "Progress: {}  ETA {}",
                    format_progress(torrent.percent_done),
                    format_eta(torrent.eta)
                )),
                Line::from(format!(
                    "Size: {} (remaining {})",
                    format_bytes(torrent.size_when_done),
                    format_bytes(torrent.left_until_done)
                )),
                Line::from(format!(
                    "Rates: DL {}  UL {}",
                    format_speed(torrent.rate_download),
                    format_speed(torrent.rate_upload)
                )),
                Line::from(format!("Ratio: {:.2}", torrent.upload_ratio)),
                Line::from(format!(
                    "Peers: sending {} | receiving {} | connected {}",
                    torrent.peers_sending, torrent.peers_receiving, torrent.peers_connected
                )),
                Line::from(format!("Path: {}", torrent.download_dir)),
            ];
            let mut lines = content;
            if let Some(error) = &torrent.error {
                lines.push(Line::from(Span::styled(
                    format!("Error: {error}"),
                    Style::default().fg(Color::Red),
                )));
            }
            let paragraph = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(paragraph, area);
        } else {
            let paragraph = Paragraph::new("No torrent selected")
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(paragraph, area);
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let mode_label = match &self.mode {
            InputMode::Normal => "NORMAL",
            InputMode::Filter { .. } => "FILTER",
            InputMode::Prompt(_) => "PROMPT",
            InputMode::Confirm(_) => "CONFIRM",
            InputMode::Help => "HELP",
        };
        let filter_display = match &self.mode {
            InputMode::Filter { buffer } => format!("/{}", buffer),
            _ => {
                if self.filter_text.is_empty() {
                    "(no filter)".to_string()
                } else {
                    format!("/{}", self.filter_text)
                }
            }
        };
        let summary = Line::from(format!("Mode {mode_label} | Filter {filter_display}"));
        let sections = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(14)])
            .split(area);
        let left = Paragraph::new(summary).wrap(Wrap { trim: true });
        frame.render_widget(left, sections[0]);
        let help_label =
            Paragraph::new(Line::from(Span::raw("Help [?]"))).alignment(Alignment::Right);
        frame.render_widget(help_label, sections[1]);
    }

    fn render_toast(&self, frame: &mut Frame) {
        if !matches!(self.mode, InputMode::Normal | InputMode::Filter { .. }) {
            return;
        }
        let Some(toast) = &self.toast else {
            return;
        };
        let frame_area = frame.size();
        if frame_area.width < 20 || frame_area.height < 5 {
            return;
        }
        let padding = 2;
        let max_width = frame_area.width.saturating_sub(padding * 2);
        let width = max_width.clamp(20, 60);
        let height = 3;
        let x = frame_area
            .x
            .saturating_add(frame_area.width.saturating_sub(width + padding));
        let y = frame_area
            .y
            .saturating_add(frame_area.height.saturating_sub(height + padding));
        let area = Rect::new(x, y, width, height);
        let text = Line::from(Span::styled(toast.text.clone(), status_style(toast.level)));
        let paragraph = Paragraph::new(text).alignment(Alignment::Center).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::raw(" Notice ")),
        );
        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }

    fn process_event(&mut self, event: AppEvent, rpc_tx: &Sender<RpcCommand>) -> Result<bool> {
        match event {
            AppEvent::Input(event) => self.handle_input(event, rpc_tx),
            AppEvent::Tick => {
                self.expire_status();
                Ok(false)
            }
            AppEvent::Snapshot(result) => {
                self.apply_snapshot(result);
                Ok(false)
            }
            AppEvent::Status(update) => {
                self.set_status(update);
                Ok(false)
            }
            AppEvent::FocusTorrent(target) => {
                self.pending_focus = target;
                Ok(false)
            }
        }
    }

    fn handle_input(&mut self, event: Event, rpc_tx: &Sender<RpcCommand>) -> Result<bool> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    self.should_quit = true;
                    return Ok(true);
                }
                if matches!(self.mode, InputMode::Normal) {
                    return self.handle_normal_key(key, rpc_tx);
                }
                match &mut self.mode {
                    InputMode::Filter { buffer } => {
                        let mut action = FilterAction::None;
                        match key.code {
                            KeyCode::Enter => {
                                let value = buffer.trim().to_string();
                                action = FilterAction::Apply(value);
                            }
                            KeyCode::Esc => {
                                action = FilterAction::Cancel;
                            }
                            KeyCode::Backspace => {
                                buffer.pop();
                            }
                            KeyCode::Char(c) => {
                                buffer.push(c);
                            }
                            _ => {}
                        }
                        match action {
                            FilterAction::Apply(value) => {
                                self.mode = InputMode::Normal;
                                self.apply_filter_text(value);
                            }
                            FilterAction::Cancel => {
                                self.mode = InputMode::Normal;
                            }
                            FilterAction::None => {}
                        }
                        Ok(false)
                    }
                    InputMode::Prompt(prompt) => {
                        let mut action = PromptAction::None;
                        match key.code {
                            KeyCode::Enter => {
                                let value = prompt.buffer.trim().to_string();
                                action = if value.is_empty() {
                                    PromptAction::Cancel
                                } else {
                                    PromptAction::Submit(value)
                                };
                            }
                            KeyCode::Esc => {
                                action = PromptAction::Cancel;
                            }
                            KeyCode::Backspace => {
                                prompt.buffer.pop();
                            }
                            KeyCode::Char(c) => {
                                prompt.buffer.push(c);
                            }
                            _ => {}
                        }
                        match action {
                            PromptAction::Submit(value) => {
                                self.mode = InputMode::Normal;
                                self.set_status(StatusUpdate::info("Submitting magnet…"));
                                if rpc_tx.send(RpcCommand::AddMagnet(value)).is_err() {
                                    self.set_status(StatusUpdate::error(
                                        "Failed to queue magnet add",
                                    ));
                                }
                            }
                            PromptAction::Cancel => {
                                self.mode = InputMode::Normal;
                            }
                            PromptAction::None => {}
                        }
                        Ok(false)
                    }
                    InputMode::Confirm(confirm) => {
                        let mut action = ConfirmAction::None;
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Enter => {
                                action = ConfirmAction::Accept;
                            }
                            KeyCode::Char('n') | KeyCode::Esc => {
                                action = ConfirmAction::Cancel;
                            }
                            _ => {}
                        }
                        match action {
                            ConfirmAction::Accept => {
                                let info = format!("Removing {}…", confirm.target_name);
                                let id = confirm.target_id;
                                let name = confirm.target_name.clone();
                                let delete_data = confirm.delete_data;
                                self.mode = InputMode::Normal;
                                self.set_status(StatusUpdate::info(info));
                                if rpc_tx
                                    .send(RpcCommand::RemoveTorrent {
                                        id,
                                        name,
                                        delete_data,
                                    })
                                    .is_err()
                                {
                                    self.set_status(StatusUpdate::error(
                                        "Failed to queue deletion",
                                    ));
                                }
                            }
                            ConfirmAction::Cancel => {
                                self.mode = InputMode::Normal;
                                self.set_status(StatusUpdate::info("Deletion cancelled"));
                            }
                            ConfirmAction::None => {}
                        }
                        Ok(false)
                    }
                    InputMode::Help => {
                        match key.code {
                            KeyCode::Char('?')
                            | KeyCode::Esc
                            | KeyCode::Enter
                            | KeyCode::Char('q') => {
                                self.mode = InputMode::Normal;
                            }
                            _ => {}
                        }
                        Ok(false)
                    }
                    InputMode::Normal => Ok(false),
                }
            }
            Event::Paste(data) => self.handle_paste(data, rpc_tx),
            _ => Ok(false),
        }
    }

    fn handle_paste(&mut self, data: String, _rpc_tx: &Sender<RpcCommand>) -> Result<bool> {
        match &mut self.mode {
            InputMode::Filter { buffer } => {
                buffer.push_str(&data);
                Ok(false)
            }
            InputMode::Prompt(prompt) => {
                prompt.buffer.push_str(&data);
                Ok(false)
            }
            _ => {
                let mut prompt = PromptState::new("Add magnet");
                prompt.buffer.push_str(&data);
                self.mode = InputMode::Prompt(prompt);
                Ok(false)
            }
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent, rpc_tx: &Sender<RpcCommand>) -> Result<bool> {
        let plain_d = matches!(key.code, KeyCode::Char('d')) && key.modifiers.is_empty();
        if !plain_d {
            self.disarm_delete();
        }
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                Ok(true)
            }
            KeyCode::Char('r') => {
                self.disarm_delete();
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.queue_refresh(rpc_tx);
                } else {
                    self.resume_selected_torrent(rpc_tx);
                }
                Ok(false)
            }
            KeyCode::Char('R') => {
                self.disarm_delete();
                self.queue_refresh(rpc_tx);
                Ok(false)
            }
            KeyCode::Char('p') => {
                self.disarm_delete();
                self.pause_selected_torrent(rpc_tx);
                Ok(false)
            }
            KeyCode::Char('a') => {
                self.disarm_delete();
                self.mode = InputMode::Prompt(PromptState::new("Add magnet"));
                Ok(false)
            }
            KeyCode::Char('/') => {
                self.disarm_delete();
                self.mode = InputMode::Filter {
                    buffer: self.filter_text.clone(),
                };
                Ok(false)
            }
            KeyCode::Char('j') => {
                self.move_selection(1);
                Ok(false)
            }
            KeyCode::Char('k') => {
                self.move_selection(-1);
                Ok(false)
            }
            KeyCode::Char('g') => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.goto_bottom();
                } else {
                    self.goto_top();
                }
                Ok(false)
            }
            KeyCode::Char('G') => {
                self.goto_bottom();
                Ok(false)
            }
            KeyCode::Char('?') => {
                self.disarm_delete();
                self.mode = InputMode::Help;
                Ok(false)
            }
            KeyCode::Char('d') if plain_d => {
                if self.delete_armed {
                    self.disarm_delete();
                    self.prompt_delete_current();
                } else {
                    self.arm_delete();
                }
                Ok(false)
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(5);
                Ok(false)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(-5);
                Ok(false)
            }
            KeyCode::Esc => {
                self.clear_filter();
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let max_index = self.filtered_indices.len() as isize - 1;
        let current = self.list_state.selected().unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, max_index) as usize;
        self.list_state.select(Some(next));
        self.update_selected_id();
    }

    fn goto_top(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        self.list_state.select(Some(0));
        self.update_selected_id();
    }

    fn goto_bottom(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let index = self.filtered_indices.len() - 1;
        self.list_state.select(Some(index));
        self.update_selected_id();
    }

    fn update_selected_id(&mut self) {
        self.selected_id = self.current_torrent().map(|t| t.torrent_id);
    }

    fn current_torrent(&self) -> Option<&TorrentSummary> {
        let snapshot = self.snapshot.as_ref()?;
        let selected = self.list_state.selected()?;
        let torrent_index = *self.filtered_indices.get(selected)?;
        snapshot.torrents.get(torrent_index)
    }

    fn clear_filter(&mut self) {
        if self.filter_text.is_empty() {
            return;
        }
        self.filter_text.clear();
        self.filter_lower.clear();
        self.rebuild_indices();
    }

    fn apply_filter_text(&mut self, value: String) {
        self.filter_text = value.clone();
        self.filter_lower = value.to_lowercase();
        self.rebuild_indices();
    }

    fn rebuild_indices(&mut self) {
        self.filtered_indices.clear();
        if let Some(snapshot) = &self.snapshot {
            for (idx, torrent) in snapshot.torrents.iter().enumerate() {
                if self.matches_filter(torrent) {
                    self.filtered_indices.push(idx);
                }
            }
        }
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
            self.selected_id = None;
            return;
        }
        if let Some(target) = self.pending_focus.take().or(self.selected_id) {
            if let Some(pos) = self
                .filtered_indices
                .iter()
                .position(|&idx| self.snapshot.as_ref().unwrap().torrents[idx].torrent_id == target)
            {
                self.list_state.select(Some(pos));
                self.selected_id = Some(target);
                return;
            }
        }
        let selected = self.list_state.selected().unwrap_or(0);
        let selected = selected.min(self.filtered_indices.len() - 1);
        self.list_state.select(Some(selected));
        self.update_selected_id();
    }

    fn matches_filter(&self, torrent: &TorrentSummary) -> bool {
        if self.filter_lower.is_empty() {
            return true;
        }
        torrent.name.to_lowercase().contains(&self.filter_lower)
    }

    fn expire_status(&mut self) {
        if let Some(status) = &self.status {
            if let Some(expiry) = status.expires_at {
                if Instant::now() >= expiry {
                    self.status = None;
                }
            }
        }
        if let Some(toast) = &self.toast {
            if let Some(expiry) = toast.expires_at {
                if Instant::now() >= expiry {
                    self.toast = None;
                }
            }
        }
        if self.delete_armed {
            if let Some(deadline) = self.delete_armed_until {
                if Instant::now() >= deadline {
                    self.disarm_delete();
                }
            }
        }
    }

    fn set_status(&mut self, update: StatusUpdate) {
        let message = StatusMessage::from_update(update.clone());
        if matches!(update.level, StatusLevel::Warning | StatusLevel::Error) {
            self.toast = Some(message.clone());
        }
        self.status = Some(message);
    }

    fn disarm_delete(&mut self) {
        self.delete_armed = false;
        self.delete_armed_until = None;
    }

    fn queue_refresh(&mut self, rpc_tx: &Sender<RpcCommand>) {
        self.pending_manual_refresh = true;
        self.set_status(StatusUpdate::info("Refreshing…"));
        if rpc_tx.send(RpcCommand::Refresh).is_err() {
            self.set_status(StatusUpdate::error("Failed to queue refresh"));
        }
    }

    fn arm_delete(&mut self) {
        self.delete_armed = true;
        self.delete_armed_until = Some(Instant::now() + Duration::from_secs(2));
        self.set_status(StatusUpdate::info(
            "Press d again to delete the selected torrent",
        ));
    }

    fn prompt_delete_current(&mut self) {
        if let Some(torrent) = self.current_torrent().cloned() {
            self.mode = InputMode::Confirm(ConfirmState::remove_torrent(
                torrent.name.clone(),
                torrent.torrent_id,
            ));
        } else {
            self.set_status(StatusUpdate::error("No torrent selected to delete"));
        }
    }

    fn resume_selected_torrent(&mut self, rpc_tx: &Sender<RpcCommand>) {
        if let Some(torrent) = self.current_torrent().cloned() {
            let id = torrent.torrent_id;
            let name = torrent.name.clone();
            self.set_status(StatusUpdate::info(format!("Resuming {name}…")));
            if rpc_tx.send(RpcCommand::ResumeTorrent { id, name }).is_err() {
                self.set_status(StatusUpdate::error("Failed to queue resume"));
            }
        } else {
            self.set_status(StatusUpdate::warning("No torrent selected; cannot resume"));
        }
    }

    fn pause_selected_torrent(&mut self, rpc_tx: &Sender<RpcCommand>) {
        if let Some(torrent) = self.current_torrent().cloned() {
            let id = torrent.torrent_id;
            let name = torrent.name.clone();
            self.set_status(StatusUpdate::info(format!("Pausing {name}…")));
            if rpc_tx.send(RpcCommand::PauseTorrent { id, name }).is_err() {
                self.set_status(StatusUpdate::error("Failed to queue pause"));
            }
        } else {
            self.set_status(StatusUpdate::warning("No torrent selected; cannot pause"));
        }
    }

    fn apply_snapshot(&mut self, result: RpcResult<Snapshot>) {
        match result {
            Ok(snapshot) => {
                let focus = self.pending_focus.take().or(self.selected_id);
                self.snapshot = Some(snapshot);
                self.selected_id = focus;
                if self.selected_id.is_none() {
                    self.selected_id = self
                        .snapshot
                        .as_ref()
                        .and_then(|snap| snap.torrents.first().map(|t| t.torrent_id));
                }
                self.rebuild_indices();
                if self.pending_manual_refresh || self.status.is_none() {
                    let count = self
                        .snapshot
                        .as_ref()
                        .map(|snap| snap.torrents.len())
                        .unwrap_or(0);
                    self.set_status(StatusUpdate::success(format!("Refreshed {count} torrents")));
                }
                self.pending_manual_refresh = false;
            }
            Err(err) => {
                self.set_status(StatusUpdate::error(format!("RPC error: {err}")));
                self.pending_manual_refresh = false;
            }
        }
    }
}

#[derive(Clone)]
struct PromptState {
    title: &'static str,
    buffer: String,
}

impl PromptState {
    fn new(title: &'static str) -> Self {
        Self {
            title,
            buffer: String::new(),
        }
    }
}

#[derive(Clone)]
struct ConfirmState {
    title: &'static str,
    message: String,
    target_id: i64,
    target_name: String,
    delete_data: bool,
}

impl ConfirmState {
    fn remove_torrent(name: String, id: i64) -> Self {
        Self {
            title: "Remove torrent",
            message: format!("Remove '{name}' from Transmission?"),
            target_id: id,
            target_name: name,
            delete_data: false,
        }
    }
}

enum InputMode {
    Normal,
    Filter { buffer: String },
    Prompt(PromptState),
    Confirm(ConfirmState),
    Help,
}

enum FilterAction {
    None,
    Apply(String),
    Cancel,
}

enum PromptAction {
    None,
    Submit(String),
    Cancel,
}

enum ConfirmAction {
    None,
    Accept,
    Cancel,
}

enum RpcCommand {
    Refresh,
    AddMagnet(String),
    RemoveTorrent {
        id: i64,
        name: String,
        delete_data: bool,
    },
    ResumeTorrent {
        id: i64,
        name: String,
    },
    PauseTorrent {
        id: i64,
        name: String,
    },
}

fn summary_line(summary: &TorrentSummary) -> String {
    format!(
        "{:<40.40}  {:<11}  {:>6}  DL {:>7}  UL {:>7}",
        summary.name,
        summary.status,
        format_progress(summary.percent_done),
        format_speed(summary.rate_download),
        format_speed(summary.rate_upload)
    )
}

fn status_style(level: StatusLevel) -> Style {
    match level {
        StatusLevel::Info => Style::default().fg(Color::Blue),
        StatusLevel::Success => Style::default().fg(Color::Green),
        StatusLevel::Warning => Style::default().fg(Color::Yellow),
        StatusLevel::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let vertical = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1]);
    vertical[1]
}

fn help_lines() -> Vec<Line<'static>> {
    let heading = |text: &'static str| {
        Line::from(Span::styled(
            text,
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ))
    };
    vec![
        heading("Navigation"),
        Line::from("  j / k: move selection"),
        Line::from("  g / G: jump to first / last"),
        Line::from("  Ctrl+d / Ctrl+u: half-page down/up"),
        Line::from(""),
        heading("Actions"),
        Line::from("  r: resume selected torrent"),
        Line::from("  R: refresh now"),
        Line::from("  p: pause selected torrent"),
        Line::from("  a: add magnet"),
        Line::from("  dd: delete highlighted torrent"),
        Line::from("  /: filter list"),
        Line::from("  Esc: clear filter / cancel dialog"),
        Line::from("  ?: toggle this help"),
        Line::from("  q or Ctrl+c: quit"),
        Line::from(""),
        heading("Dialogs"),
        Line::from("  Prompt: Enter to submit, Esc to cancel"),
        Line::from("  Confirm: y to accept, n/Esc to cancel"),
    ]
}
