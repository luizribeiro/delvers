use crate::protocol::{ClientMsg, Dir, ServerMsg, WorldView};
use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};
use std::io::{Stdout, stdout};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

pub async fn run(socket_path: &str, name: &str) -> Result<()> {
    let stream = UnixStream::connect(socket_path).await?;
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);

    // Send Hello
    let hello = serde_json::to_string(&ClientMsg::Hello {
        name: name.to_string(),
    })?;
    w.write_all(hello.as_bytes()).await?;
    w.write_all(b"\n").await?;

    let (srv_tx, mut srv_rx) = mpsc::unbounded_channel::<ServerMsg>();
    tokio::spawn(async move {
        loop {
            let mut line = String::new();
            let n = match reader.read_line(&mut line).await {
                Ok(n) => n,
                Err(_) => break,
            };
            if n == 0 {
                break;
            }
            match serde_json::from_str::<ServerMsg>(line.trim()) {
                Ok(m) => {
                    if srv_tx.send(m).is_err() {
                        break;
                    }
                }
                Err(_) => continue,
            }
        }
    });

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal: Terminal<CrosstermBackend<Stdout>> = Terminal::new(backend)?;

    let mut app = App::new(name.to_string());
    let mut events = EventStream::new();
    let mut should_quit = false;

    while !should_quit {
        terminal.draw(|f| app.draw(f))?;

        tokio::select! {
            maybe_ev = events.next() => {
                if let Some(Ok(ev)) = maybe_ev {
                    match ev {
                        Event::Key(k) => {
                            if handle_key(&mut app, k, &mut w).await? {
                                should_quit = true;
                            }
                        }
                        Event::Resize(_, _) => {}
                        _ => {}
                    }
                } else {
                    break;
                }
            }
            Some(msg) = srv_rx.recv() => {
                app.on_server_msg(msg);
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    // Best-effort send Quit
    let q = serde_json::to_string(&ClientMsg::Quit)?;
    let _ = w.write_all(q.as_bytes()).await;
    let _ = w.write_all(b"\n").await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn handle_key(
    app: &mut App,
    k: KeyEvent,
    w: &mut tokio::net::unix::OwnedWriteHalf,
) -> Result<bool> {
    if k.kind != KeyEventKind::Press {
        return Ok(false);
    }

    // Ctrl+C always exits
    if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
        return Ok(true);
    }

    if app.chat_input {
        match k.code {
            KeyCode::Esc => {
                app.chat_input = false;
                app.chat_buf.clear();
                app.chat_is_shout = false;
            }
            KeyCode::Enter => {
                let text = std::mem::take(&mut app.chat_buf);
                let was_shout = app.chat_is_shout;
                app.chat_input = false;
                app.chat_is_shout = false;
                if !text.trim().is_empty() {
                    if was_shout {
                        send(w, &ClientMsg::Shout(text)).await?;
                    } else {
                        send(w, &ClientMsg::Chat(text)).await?;
                    }
                }
            }
            KeyCode::Backspace => {
                app.chat_buf.pop();
            }
            KeyCode::Char(c) => {
                if app.chat_buf.len() < 140 {
                    app.chat_buf.push(c);
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    if app.help_open {
        if matches!(k.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char(' ')) {
            app.help_open = false;
        }
        return Ok(false);
    }

    // Dead: show respawn screen (but keep chat usable)
    if let Some(v) = &app.view {
        if !v.alive {
            match k.code {
                KeyCode::Enter | KeyCode::Char(' ') => {
                    send(w, &ClientMsg::Respawn).await?;
                }
                KeyCode::Char('Q') => return Ok(true),
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    app.chat_input = true;
                    app.chat_buf.clear();
                }
                KeyCode::Char('?') => app.help_open = true,
                _ => {}
            }
            return Ok(false);
        }
    }

    match k.code {
        KeyCode::Char('Q') => return Ok(true),
        KeyCode::Char('?') => app.help_open = true,
        KeyCode::Char('t') | KeyCode::Char('T') => {
            app.chat_input = true;
            app.chat_buf.clear();
            app.chat_is_shout = false;
        }
        KeyCode::Char('s') => {
            app.chat_input = true;
            app.chat_buf.clear();
            app.chat_is_shout = true;
        }
        KeyCode::Char('r') => send(w, &ClientMsg::Rest).await?,
        KeyCode::Tab => {
            app.show_labels = !app.show_labels;
        }
        KeyCode::Char('h') | KeyCode::Left => send(w, &ClientMsg::Move(Dir::W)).await?,
        KeyCode::Char('l') | KeyCode::Right => send(w, &ClientMsg::Move(Dir::E)).await?,
        KeyCode::Char('k') | KeyCode::Up => send(w, &ClientMsg::Move(Dir::N)).await?,
        KeyCode::Char('j') | KeyCode::Down => send(w, &ClientMsg::Move(Dir::S)).await?,
        KeyCode::Char('y') => send(w, &ClientMsg::Move(Dir::NW)).await?,
        KeyCode::Char('u') => send(w, &ClientMsg::Move(Dir::NE)).await?,
        KeyCode::Char('b') => send(w, &ClientMsg::Move(Dir::SW)).await?,
        KeyCode::Char('n') => send(w, &ClientMsg::Move(Dir::SE)).await?,
        KeyCode::Char('.') => send(w, &ClientMsg::Wait).await?,
        KeyCode::Char(',') | KeyCode::Char('g') => send(w, &ClientMsg::Pickup).await?,
        KeyCode::Char('>') => send(w, &ClientMsg::Descend).await?,
        KeyCode::Char('<') => send(w, &ClientMsg::Ascend).await?,
        KeyCode::Char('q') => send(w, &ClientMsg::Quaff).await?,
        _ => {}
    }
    Ok(false)
}

async fn send(w: &mut tokio::net::unix::OwnedWriteHalf, msg: &ClientMsg) -> Result<()> {
    let s = serde_json::to_string(msg)?;
    w.write_all(s.as_bytes()).await?;
    w.write_all(b"\n").await?;
    Ok(())
}

pub struct App {
    pub my_name: String,
    pub player_id: Option<u64>,
    pub motd: String,
    pub view: Option<WorldView>,
    pub log: Vec<(String, u8)>,
    pub chat: Vec<(String, String, u8)>,
    pub chat_input: bool,
    pub chat_buf: String,
    pub chat_is_shout: bool,
    pub help_open: bool,
    pub show_labels: bool,
    pub last_death_by: Option<String>,
    pub victory_by: Option<String>,
}

impl App {
    pub fn new(name: String) -> Self {
        App {
            my_name: name,
            player_id: None,
            motd: String::new(),
            view: None,
            log: Vec::new(),
            chat: Vec::new(),
            chat_input: false,
            chat_buf: String::new(),
            chat_is_shout: false,
            help_open: false,
            show_labels: false,
            last_death_by: None,
            victory_by: None,
        }
    }

    pub fn on_server_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Welcome {
                player_id,
                name,
                motd,
            } => {
                self.player_id = Some(player_id);
                self.my_name = name;
                self.motd = motd;
                self.log.push(("Welcome, adventurer!".into(), 14));
            }
            ServerMsg::State(v) => {
                self.view = Some(v);
            }
            ServerMsg::Log { text, color } => {
                self.log.push((text, color));
                if self.log.len() > 400 {
                    self.log.drain(0..200);
                }
            }
            ServerMsg::Chat { who, text, color } => {
                self.chat.push((who, text, color));
                if self.chat.len() > 200 {
                    self.chat.drain(0..100);
                }
            }
            ServerMsg::Death { by } => {
                self.last_death_by = Some(by);
            }
            ServerMsg::Victory { by } => {
                self.victory_by = Some(by);
            }
            ServerMsg::Error(e) => {
                self.log.push((format!("[error] {e}"), 9));
            }
        }
    }

    pub fn draw(&self, f: &mut ratatui::Frame) {
        let area = f.area();

        // Top-level: [map+sidebar] top, [log/chat] bottom, status line
        let vchunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(10),
                Constraint::Length(8),
                Constraint::Length(1),
            ])
            .split(area);

        let top = vchunks[0];
        let middle = vchunks[1];
        let status_line = vchunks[2];

        // Top: map | sidebar
        let hchunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(30)])
            .split(top);
        let map_area = hchunks[0];
        let side_area = hchunks[1];

        let side_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(11), Constraint::Min(3)])
            .split(side_area);
        self.draw_map(f, map_area);
        self.draw_sidebar(f, side_chunks[0]);
        self.draw_roster(f, side_chunks[1]);

        // Middle: log | chat
        let mchunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(middle);
        self.draw_log(f, mchunks[0]);
        self.draw_chat(f, mchunks[1]);

        // Status line
        self.draw_status_line(f, status_line);

        if self.help_open {
            self.draw_help(f, area);
        }

        if let Some(v) = &self.view {
            if !v.alive {
                self.draw_death(f, area);
            }
        }
        if self.victory_by.is_some() {
            self.draw_victory(f, area);
        }
    }

    fn draw_map(&self, f: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Dungeon ")
            .title_style(Style::default().fg(Color::LightYellow));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let buf = f.buffer_mut();

        let Some(v) = &self.view else {
            return;
        };

        let w = v.width as i32;
        let h = v.height as i32;

        // Center map on player (if alive)
        let my_id = self.player_id.unwrap_or(0);
        let (cx, cy) = v
            .entities
            .iter()
            .find(|e| e.is_self)
            .map(|e| (e.x, e.y))
            .unwrap_or((w / 2, h / 2));
        let view_w = inner.width as i32;
        let view_h = inner.height as i32;
        let ox = (cx - view_w / 2).max(0).min((w - view_w).max(0));
        let oy = (cy - view_h / 2).max(0).min((h - view_h).max(0));

        // Draw tiles with visibility shading
        for ty in 0..view_h {
            for tx in 0..view_w {
                let mx = ox + tx;
                let my = oy + ty;
                if mx < 0 || my < 0 || mx >= w || my >= h {
                    continue;
                }
                let idx = (my as usize) * (w as usize) + mx as usize;
                let tile = v.tiles[idx];
                let visibility = v.vis.get(idx).copied().unwrap_or(0);
                if visibility == 0 {
                    continue;
                }
                let (ch, mut style) = tile_style(tile);
                if visibility == 1 {
                    // remembered: darken substantially
                    style = Style::default().fg(Color::Rgb(50, 50, 70));
                }
                let sx = inner.x + tx as u16;
                let sy = inner.y + ty as u16;
                if sx < inner.x + inner.width && sy < inner.y + inner.height {
                    buf[(sx, sy)].set_char(ch).set_style(style);
                }
            }
        }

        // Draw entities (items first then monsters then players)
        let mut sorted = v.entities.clone();
        sorted.sort_by_key(|e| {
            if e.is_self {
                3
            } else if e.is_player {
                2
            } else if e.glyph.is_ascii_lowercase() || e.glyph.is_ascii_uppercase() {
                1
            } else {
                0
            }
        });
        for e in sorted.iter() {
            let sx = inner.x as i32 + (e.x - ox);
            let sy = inner.y as i32 + (e.y - oy);
            if sx < inner.x as i32
                || sy < inner.y as i32
                || sx >= (inner.x + inner.width) as i32
                || sy >= (inner.y + inner.height) as i32
            {
                continue;
            }
            let mut style = Style::default().fg(ansi_color(e.color));
            if e.is_self {
                style = style
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::Rgb(32, 32, 48));
            } else if e.is_player {
                style = style.add_modifier(Modifier::BOLD);
            }
            buf[(sx as u16, sy as u16)].set_char(e.glyph).set_style(style);
        }

        // Damage floaters on tiles
        for flo in v.floaters.iter() {
            let sx = inner.x as i32 + (flo.x - ox);
            let sy = inner.y as i32 + (flo.y - oy) - 1;
            if sy < inner.y as i32 || sy >= (inner.y + inner.height) as i32 {
                continue;
            }
            let base_x = sx;
            for (i, ch) in flo.text.chars().enumerate() {
                let cx = base_x + i as i32;
                if cx < inner.x as i32 || cx >= (inner.x + inner.width) as i32 {
                    continue;
                }
                let style = Style::default()
                    .fg(ansi_color(flo.color))
                    .bg(Color::Rgb(30, 10, 10))
                    .add_modifier(Modifier::BOLD);
                buf[(cx as u16, sy as u16)].set_char(ch).set_style(style);
            }
        }

        // Chat bubbles: every visible player with a bubble gets their text
        // rendered above them.
        for e in v.entities.iter().filter(|e| e.is_player && e.bubble.is_some()) {
            let text = e.bubble.as_ref().unwrap();
            let label_y = inner.y as i32 + (e.y - oy) - 1;
            if label_y < inner.y as i32 {
                continue;
            }
            let max_w = (inner.x + inner.width) as i32 - (inner.x as i32 + (e.x - ox));
            if max_w <= 2 {
                continue;
            }
            let trunc = truncate(text, (max_w as usize).saturating_sub(2).min(40));
            let base_x = inner.x as i32 + (e.x - ox);
            let total_len = trunc.chars().count() + 2;
            for (i, ch) in format!("‹{}›", trunc).chars().enumerate() {
                let sx = base_x + i as i32;
                if sx < inner.x as i32 || sx >= (inner.x + inner.width) as i32 {
                    continue;
                }
                let style = Style::default()
                    .fg(ansi_color(e.color))
                    .bg(Color::Rgb(30, 30, 50))
                    .add_modifier(Modifier::BOLD);
                buf[(sx as u16, label_y as u16)]
                    .set_char(ch)
                    .set_style(style);
            }
            let _ = total_len;
        }

        // Invuln shimmer: draw a marker cell over invulnerable players (self too)
        for e in v.entities.iter().filter(|e| e.is_player && e.invuln) {
            let sx = inner.x as i32 + (e.x - ox);
            let sy = inner.y as i32 + (e.y - oy);
            if sx < inner.x as i32
                || sy < inner.y as i32
                || sx >= (inner.x + inner.width) as i32
                || sy >= (inner.y + inner.height) as i32
            {
                continue;
            }
            let style = Style::default()
                .fg(Color::Rgb(200, 220, 255))
                .bg(Color::Rgb(40, 40, 90))
                .add_modifier(Modifier::BOLD);
            buf[(sx as u16, sy as u16)].set_char(e.glyph).set_style(style);
        }

        // Optional: player name labels above each visible player (Tab toggles).
        // Labels alternate above/below so adjacent players don't collide, and we
        // skip a label if it would overwrite another player's glyph.
        if self.show_labels {
            let players: Vec<&crate::protocol::EntityView> =
                v.entities.iter().filter(|e| e.is_player).collect();
            for (pi, e) in players.iter().enumerate() {
                let side = if pi % 2 == 0 { -1 } else { 2 };
                let label_y = inner.y as i32 + (e.y - oy) + side;
                if label_y < inner.y as i32 || label_y >= (inner.y + inner.height) as i32 {
                    continue;
                }
                let label = &e.name;
                // left-anchor at player's x, truncated to fit
                let mut label_x = inner.x as i32 + (e.x - ox);
                // If label would extend past right edge, shift left
                let avail = (inner.x + inner.width) as i32 - label_x;
                let take = label.chars().count().min(avail.max(0) as usize);
                if take == 0 {
                    continue;
                }
                // avoid overwriting another player glyph directly
                for (i, ch) in label.chars().take(take).enumerate() {
                    let sx = label_x + i as i32;
                    if sx < inner.x as i32 {
                        continue;
                    }
                    // don't overwrite other players' tiles (those are drawn above)
                    let style = Style::default()
                        .fg(ansi_color(e.color))
                        .bg(Color::Rgb(15, 15, 25))
                        .add_modifier(Modifier::BOLD);
                    buf[(sx as u16, label_y as u16)]
                        .set_char(ch)
                        .set_style(style);
                }
                let _ = label_x;
            }
        }
        let _ = my_id;
    }

    fn draw_sidebar(&self, f: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Stats ")
            .title_style(Style::default().fg(Color::LightYellow));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let Some(v) = &self.view else {
            return;
        };
        let s = &v.stats;

        // HP gauge
        let hp_ratio = (s.hp as f64 / s.max_hp.max(1) as f64).clamp(0.0, 1.0);
        let hp_color = if hp_ratio > 0.66 {
            Color::Green
        } else if hp_ratio > 0.33 {
            Color::Yellow
        } else {
            Color::Red
        };
        let gauge_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        let hp_gauge = Gauge::default()
            .gauge_style(Style::default().fg(hp_color))
            .ratio(hp_ratio)
            .label(format!("HP {}/{}", s.hp, s.max_hp));
        f.render_widget(hp_gauge, gauge_area);

        // XP gauge
        let xp_ratio = (s.xp as f64 / s.xp_next.max(1) as f64).clamp(0.0, 1.0);
        let xp_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: 1,
        };
        let xp_gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(xp_ratio)
            .label(format!("L{} XP {}/{}", s.level, s.xp, s.xp_next));
        f.render_widget(xp_gauge, xp_area);

        let text_area = Rect {
            x: inner.x,
            y: inner.y + 3,
            width: inner.width,
            height: inner.height.saturating_sub(3),
        };

        let lines = vec![
            Line::from(vec![
                Span::styled("Name ", Style::default().fg(Color::Gray)),
                Span::styled(s.name.clone(), Style::default().fg(Color::LightCyan)),
            ]),
            Line::from(vec![
                Span::styled("Depth ", Style::default().fg(Color::Gray)),
                Span::styled(format!("{}", s.depth), Style::default().fg(Color::Yellow)),
                Span::raw("  "),
                Span::styled("Here ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{}", v.players_here),
                    Style::default().fg(Color::LightGreen),
                ),
            ]),
            Line::from(vec![
                Span::styled("Atk ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{}", s.attack),
                    Style::default().fg(Color::LightRed),
                ),
                Span::raw("  "),
                Span::styled("Def ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{}", s.defense),
                    Style::default().fg(Color::LightBlue),
                ),
            ]),
            Line::from(vec![
                Span::styled("Gold ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{}", s.gold),
                    Style::default().fg(Color::LightYellow),
                ),
                Span::raw("  "),
                Span::styled("Potions ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{}", s.potions),
                    Style::default().fg(Color::LightMagenta),
                ),
            ]),
            Line::from(vec![
                Span::styled("Wpn ", Style::default().fg(Color::Gray)),
                Span::styled(s.weapon.clone(), Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Arm ", Style::default().fg(Color::Gray)),
                Span::styled(s.armor.clone(), Style::default().fg(Color::White)),
            ]),
        ];
        let p = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(p, text_area);
    }

    fn draw_roster(&self, f: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Adventurers ")
            .title_style(Style::default().fg(Color::LightGreen));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let Some(v) = &self.view else { return };
        let mut lines: Vec<Line> = Vec::new();
        for r in &v.roster {
            let name_color = if r.alive {
                ansi_color(r.color)
            } else {
                Color::DarkGray
            };
            let hp_color = if !r.alive {
                Color::DarkGray
            } else if r.hp_frac > 0.66 {
                Color::Green
            } else if r.hp_frac > 0.33 {
                Color::Yellow
            } else {
                Color::Red
            };
            let hp_bar = hp_bar_str(r.hp_frac, 6);
            let depth_mark = if r.depth == v.depth { '@' } else { ' ' };
            let marker = if r.name == v.stats.name { '>' } else { depth_mark };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", marker),
                    Style::default().fg(Color::LightYellow),
                ),
                Span::styled(
                    format!("{:<8}", truncate(&r.name, 8)),
                    Style::default().fg(name_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" L{:<2} ", r.level), Style::default().fg(Color::Gray)),
                Span::styled(hp_bar, Style::default().fg(hp_color)),
                Span::styled(
                    format!(" d{}", r.depth),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "no one here yet",
                Style::default().fg(Color::DarkGray),
            )));
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    fn draw_log(&self, f: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Adventure Log ")
            .title_style(Style::default().fg(Color::LightYellow));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let max = inner.height as usize;
        let start = self.log.len().saturating_sub(max);
        let lines: Vec<Line> = self.log[start..]
            .iter()
            .map(|(t, c)| Line::from(Span::styled(t.clone(), Style::default().fg(ansi_color(*c)))))
            .collect();
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    fn draw_chat(&self, f: &mut ratatui::Frame, area: Rect) {
        let title = if self.chat_input {
            if self.chat_is_shout {
                " Chat [shouting!] ".to_string()
            } else {
                " Chat [typing] ".to_string()
            }
        } else {
            " Chat (t global, s shout) ".to_string()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().fg(Color::LightMagenta));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let (input_h, chat_h) = if self.chat_input {
            (1, inner.height.saturating_sub(1))
        } else {
            (0, inner.height)
        };
        let chat_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: chat_h,
        };
        let input_area = Rect {
            x: inner.x,
            y: inner.y + chat_h,
            width: inner.width,
            height: input_h,
        };

        let max = chat_area.height as usize;
        let start = self.chat.len().saturating_sub(max);
        let lines: Vec<Line> = self.chat[start..]
            .iter()
            .map(|(who, text, color)| {
                Line::from(vec![
                    Span::styled("<", Style::default().fg(Color::DarkGray)),
                    Span::styled(who.clone(), Style::default().fg(ansi_color(*color)).add_modifier(Modifier::BOLD)),
                    Span::styled("> ", Style::default().fg(Color::DarkGray)),
                    Span::styled(text.clone(), Style::default().fg(Color::White)),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), chat_area);

        if self.chat_input {
            let prompt = Line::from(vec![
                Span::styled("> ", Style::default().fg(Color::LightCyan)),
                Span::styled(self.chat_buf.clone(), Style::default().fg(Color::White)),
                Span::styled(
                    "_",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ]);
            f.render_widget(Paragraph::new(prompt), input_area);
        }
    }

    fn draw_status_line(&self, f: &mut ratatui::Frame, area: Rect) {
        let msg = if self.chat_input {
            " [Enter] send  [Esc] cancel ".to_string()
        } else if self.help_open {
            " [?] [Esc] close help ".to_string()
        } else if let Some(v) = &self.view {
            if !v.alive {
                " You are dead. [Enter] respawn  [t] chat  [Q] quit ".to_string()
            } else {
                format!(
                    " hjkl move  yubn diag  , pick  > desc  < asc  q quaff  r rest  t chat  s shout  Tab labels  ? help  Q quit   [d{}]",
                    v.depth
                )
            }
        } else {
            " connecting... ".to_string()
        };
        let p = Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        )));
        f.render_widget(p, area);
    }

    fn draw_help(&self, f: &mut ratatui::Frame, area: Rect) {
        let w = 60.min(area.width as u16);
        let h = 18.min(area.height as u16);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let r = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Help ")
            .title_style(Style::default().fg(Color::LightCyan));
        let inner = block.inner(r);
        f.render_widget(ratatui::widgets::Clear, r);
        f.render_widget(block, r);
        let text = vec![
            Line::from("Neth4x0rs — cooperative multi-player roguelike"),
            Line::from(""),
            Line::from("Movement: hjkl or arrow keys"),
            Line::from("Diagonals: y (NW)  u (NE)  b (SW)  n (SE)"),
            Line::from("Wait a turn: ."),
            Line::from(""),
            Line::from(",   Pick up item under your feet"),
            Line::from(">   Descend stairs (glyph '>')"),
            Line::from("<   Ascend stairs   (glyph '<')"),
            Line::from("q   Quaff a healing potion"),
            Line::from("r   Rest (heal ~4 HP, only if no monsters near)"),
            Line::from("t   Global chat (everyone hears you)"),
            Line::from("s   Shout to current dungeon level"),
            Line::from("Tab Toggle player name labels"),
            Line::from(""),
            Line::from("Bump into monsters to attack them."),
            Line::from("Find the Amulet of Yendor on depth 10!"),
            Line::from(""),
            Line::from(Span::styled(
                "Esc / ? / Space to close",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
    }

    fn draw_victory(&self, f: &mut ratatui::Frame, area: Rect) {
        let who = self
            .victory_by
            .clone()
            .unwrap_or_else(|| "??".to_string());
        let w = 60.min(area.width as u16);
        let h = 11.min(area.height as u16);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let r = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        f.render_widget(ratatui::widgets::Clear, r);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" VICTORY ")
            .title_style(
                Style::default()
                    .fg(Color::LightYellow)
                    .bg(Color::Rgb(40, 20, 0))
                    .add_modifier(Modifier::BOLD),
            );
        let inner = block.inner(r);
        f.render_widget(block, r);
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}  ", who),
                Style::default()
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "claims the Amulet of Yendor!",
                Style::default().fg(Color::LightYellow),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "The dungeon bows to a new champion.",
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "[Q] exit",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
            inner,
        );
    }

    fn draw_death(&self, f: &mut ratatui::Frame, area: Rect) {
        let w = 50.min(area.width as u16);
        let h = 9.min(area.height as u16);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let r = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        f.render_widget(ratatui::widgets::Clear, r);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" You Have Died ")
            .title_style(
                Style::default()
                    .fg(Color::LightRed)
                    .add_modifier(Modifier::BOLD),
            );
        let inner = block.inner(r);
        f.render_widget(block, r);
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  R I P  ",
                Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Press [Enter] to rise again"),
            Line::from("Press [Q] to abandon the dungeon"),
        ];
        f.render_widget(
            Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
            inner,
        );
    }
}

fn tile_style(tile: u8) -> (char, Style) {
    match tile {
        0 => (' ', Style::default()),
        1 => ('#', Style::default().fg(Color::Rgb(110, 85, 60))),
        2 => ('.', Style::default().fg(Color::Rgb(160, 160, 150))),
        3 => ('+', Style::default().fg(Color::Rgb(200, 130, 60))),
        4 => ('·', Style::default().fg(Color::Rgb(110, 110, 110))),
        5 => (
            '>',
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
        6 => (
            '<',
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
        7 => (
            '_',
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ),
        8 => (
            '+',
            Style::default()
                .fg(Color::Rgb(190, 190, 220))
                .add_modifier(Modifier::BOLD),
        ),
        _ => (' ', Style::default()),
    }
}

pub fn ansi_color(c: u8) -> Color {
    match c {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        15 => Color::White,
        _ => Color::White,
    }
}

fn hp_bar_str(frac: f32, width: usize) -> String {
    let filled = (frac.clamp(0.0, 1.0) * width as f32).round() as usize;
    let mut s = String::with_capacity(width + 2);
    s.push('[');
    for i in 0..width {
        if i < filled {
            s.push('#');
        } else {
            s.push('·');
        }
    }
    s.push(']');
    s
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

// Silence unused import warning on some builds.
#[allow(dead_code)]
fn _keep(_: &Buffer) {}
