use crate::game::{TICK_MS, tick};
use crate::protocol::{ClientMsg, Dir, ServerMsg};
use crate::world::{PlayerAction, World};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Duration, Instant, interval};

type Tx = mpsc::UnboundedSender<ServerMsg>;

pub struct ServerState {
    pub world: Mutex<World>,
    pub clients: Mutex<HashMap<u64, Tx>>,
}

impl ServerState {
    pub fn new() -> Self {
        ServerState {
            world: Mutex::new(World::new()),
            clients: Mutex::new(HashMap::new()),
        }
    }
}

pub async fn run(socket_path: &str) -> Result<()> {
    let path = Path::new(socket_path);
    if path.exists() {
        // Try to connect — if fails, assume stale.
        if UnixStream::connect(path).await.is_ok() {
            anyhow::bail!("server already running at {}", socket_path);
        }
        let _ = std::fs::remove_file(path);
    }
    let listener = UnixListener::bind(socket_path)?;
    let state = Arc::new(ServerState::new());
    eprintln!("[delvers server] listening on {}", socket_path);

    // Spawn game tick task
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut t = interval(Duration::from_millis(TICK_MS));
            t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                t.tick().await;
                game_tick(&state).await;
            }
        });
    }

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(state, stream).await {
                eprintln!("[server] client error: {e:?}");
            }
        });
    }
}

async fn game_tick(state: &Arc<ServerState>) {
    let mut world = state.world.lock().await;
    tick(&mut world);

    // Broadcast state to all clients
    let clients = state.clients.lock().await;
    let global_tail: Vec<(String, u8)> = world.global_log.drain(..).collect();
    // Check for victory (someone has the amulet AND is on depth 1 to escape)
    let victory_player: Option<String> = world
        .players
        .values()
        .find(|p| p.has_amulet && p.depth == 1)
        .map(|p| p.name.clone());
    for (pid, tx) in clients.iter() {
        if let Some(view) = world.build_view_for(*pid) {
            let _ = tx.send(ServerMsg::State(view));
        }
        for (text, color) in &global_tail {
            let _ = tx.send(ServerMsg::Log {
                text: text.clone(),
                color: *color,
            });
        }
        if let Some(name) = &victory_player {
            let _ = tx.send(ServerMsg::Victory { by: name.clone() });
        }
    }
    // Push per-player log entries
    let pids: Vec<u64> = world.players.keys().copied().collect();
    for pid in pids {
        if let Some(tx) = clients.get(&pid) {
            let p = world.players.get_mut(&pid).unwrap();
            let drained: Vec<(String, u8)> = p.log.drain(..).collect();
            for (text, color) in drained {
                let _ = tx.send(ServerMsg::Log { text, color });
            }
        }
    }
}

async fn handle_client(state: Arc<ServerState>, stream: UnixStream) -> Result<()> {
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();

    // First message must be Hello
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(());
    }
    let hello: ClientMsg = serde_json::from_str(line.trim())?;
    let name = match hello {
        ClientMsg::Hello { name } => name,
        _ => {
            let err = serde_json::to_string(&ServerMsg::Error("expected Hello".into()))?;
            w.write_all(err.as_bytes()).await?;
            w.write_all(b"\n").await?;
            return Ok(());
        }
    };
    // Reject silly names
    let name = sanitize_name(&name);

    // Register player
    let pid = {
        let mut world = state.world.lock().await;
        world.spawn_player(name.clone())
    };
    {
        let mut clients = state.clients.lock().await;
        clients.insert(pid, tx.clone());
    }

    // Welcome
    let welcome = ServerMsg::Welcome {
        player_id: pid,
        name: name.clone(),
        motd: motd(),
    };
    let s = serde_json::to_string(&welcome)?;
    w.write_all(s.as_bytes()).await?;
    w.write_all(b"\n").await?;

    // Announce join
    {
        let mut world = state.world.lock().await;
        world
            .global_log
            .push((format!("{} has entered the dungeon.", name), 14));
    }

    // Send initial state
    {
        let mut world = state.world.lock().await;
        if let Some(v) = world.build_view_for(pid) {
            let s = serde_json::to_string(&ServerMsg::State(v))?;
            w.write_all(s.as_bytes()).await?;
            w.write_all(b"\n").await?;
        }
    }

    // Writer task
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match serde_json::to_string(&msg) {
                Ok(s) => {
                    if w.write_all(s.as_bytes()).await.is_err() {
                        break;
                    }
                    if w.write_all(b"\n").await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Reader loop
    loop {
        let mut line = String::new();
        let n = match reader.read_line(&mut line).await {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        let msg: ClientMsg = match serde_json::from_str(line.trim()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if matches!(msg, ClientMsg::Quit) {
            break;
        }
        process_client_msg(&state, pid, msg).await;
    }

    // Cleanup
    {
        let mut clients = state.clients.lock().await;
        clients.remove(&pid);
    }
    {
        let mut world = state.world.lock().await;
        let name = world.players.get(&pid).map(|p| p.name.clone());
        world.remove_player(pid);
        if let Some(n) = name {
            world
                .global_log
                .push((format!("{} has left the dungeon.", n), 8));
        }
    }
    writer_task.abort();
    Ok(())
}

async fn process_client_msg(state: &Arc<ServerState>, pid: u64, msg: ClientMsg) {
    let mut world = state.world.lock().await;
    // Game actions go on the per-player queue; one is popped and executed
    // each tick. This bounds how fast any single client can act.
    let enqueue = |world: &mut crate::world::World, action| {
        if let Some(p) = world.players.get_mut(&pid) {
            p.enqueue(action);
        }
    };
    match msg {
        ClientMsg::Move(d) => enqueue(&mut world, crate::world::QueuedAction::Move(d)),
        ClientMsg::Wait => {}
        ClientMsg::Pickup => enqueue(
            &mut world,
            crate::world::QueuedAction::Act(PlayerAction::Pickup),
        ),
        ClientMsg::Descend => enqueue(
            &mut world,
            crate::world::QueuedAction::Act(PlayerAction::Descend),
        ),
        ClientMsg::Ascend => enqueue(
            &mut world,
            crate::world::QueuedAction::Act(PlayerAction::Ascend),
        ),
        ClientMsg::Quaff => enqueue(
            &mut world,
            crate::world::QueuedAction::Act(PlayerAction::Quaff),
        ),
        ClientMsg::Rest => enqueue(
            &mut world,
            crate::world::QueuedAction::Act(PlayerAction::Rest),
        ),
        ClientMsg::Respawn => enqueue(
            &mut world,
            crate::world::QueuedAction::Act(PlayerAction::Respawn),
        ),
        ClientMsg::Chat(text) => {
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            let (who, color) = if let Some(p) = world.players.get_mut(&pid) {
                // Bubble lasts ~40 ticks (~5 seconds at 120ms tick)
                p.bubble = Some((text.clone(), 40));
                (p.name.clone(), p.color)
            } else {
                ("???".to_string(), 7)
            };
            world.chat_log.push((who.clone(), text.clone(), color));
            if world.chat_log.len() > 500 {
                world.chat_log.drain(0..250);
            }
            let msg = ServerMsg::Chat { who, text, color };
            let clients = state.clients.lock().await;
            for tx in clients.values() {
                let _ = tx.send(msg.clone());
            }
        }
        ClientMsg::Shout(text) => {
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            let (who, color, depth) = if let Some(p) = world.players.get_mut(&pid) {
                p.bubble = Some((format!("!{}", text), 40));
                (p.name.clone(), p.color, p.depth)
            } else {
                return;
            };
            // Broadcast to players on same depth as a log entry (and to the shouter)
            let pids_on_depth: Vec<u64> = world
                .players
                .values()
                .filter(|p| p.depth == depth)
                .map(|p| p.id)
                .collect();
            for id in pids_on_depth {
                if let Some(p) = world.players.get_mut(&id) {
                    p.push_log(format!("{} shouts: {}", who, text), color);
                }
            }
        }
        _ => {}
    }
    let _ = Dir::N;
}

fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .take(16)
        .collect();
    if cleaned.is_empty() {
        format!("adventurer{}", rand::random::<u16>() % 1000)
    } else {
        cleaned
    }
}

fn motd() -> String {
    let now = Instant::now();
    let _ = now;
    "Welcome to delvers! hjkl/arrows to move, ',' to pickup, '>' descend, 'q' quaff potion, 't' chat, '?' help, 'Q' quit.".into()
}
