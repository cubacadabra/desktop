use cubacadabra_client::ClientMovement;
use log::{debug, info};
use std::{
    collections::VecDeque,
    io::ErrorKind,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, connect, stream::MaybeTlsStream};
use url::Url;

const ENV: &str = "CUBACADABRA_BACKEND_URL";
const DEFAULT_URL: &str = match option_env!("CUBACADABRA_BACKEND_URL") {
    Some(url) => url,
    None => "http://127.0.0.1:8787",
};
const RECONNECT: Duration = Duration::from_millis(750);
const POLL: Duration = Duration::from_millis(10);
const MOVE_INTERVAL: Duration = Duration::from_millis(83);
const HEARTBEAT: Duration = Duration::from_secs(30);

pub enum Event {
    Connected,
    Disconnected,
    Message(String),
}

struct Move {
    movement: ClientMovement,
}

enum Command {
    SetWorld(String),
    Send(String),
    Move(Move),
    Shutdown,
}

type Socket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

pub struct BackendClient {
    commands: Sender<Command>,
    events: Receiver<Event>,
    worker: Option<thread::JoinHandle<()>>,
}

impl BackendClient {
    pub fn new(game_id: &str) -> Result<Self, String> {
        let raw = std::env::var(ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_URL.into());
        let url = Url::parse(raw.trim()).map_err(|error| format!("{ENV} is invalid: {error}"))?;
        if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") || url.host_str().is_none() {
            return Err(format!("{ENV} must be an http(s) or ws(s) URL with a host"));
        }
        let (commands, command_receiver) = mpsc::channel();
        let (events, event_receiver) = mpsc::channel();
        let game_id = game_id.to_owned();
        let worker = thread::Builder::new()
            .name("desktop-backend".into())
            .spawn(move || run_worker(url, game_id, command_receiver, events))
            .map_err(|error| format!("could not start backend worker: {error}"))?;
        Ok(Self {
            commands,
            events: event_receiver,
            worker: Some(worker),
        })
    }

    pub fn set_world(&self, world: String) {
        let _ = self.commands.send(Command::SetWorld(world));
    }

    pub fn send(&self, message: String) {
        let _ = self.commands.send(Command::Send(message));
    }

    pub fn send_move(&self, movement: ClientMovement) {
        let _ = self.commands.send(Command::Move(Move { movement }));
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

impl Drop for BackendClient {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    base_url: Url,
    game_id: String,
    commands: Receiver<Command>,
    events: Sender<Event>,
) {
    let mut desired_world = None;
    let mut connected_world = None;
    let mut socket: Option<Socket> = None;
    let mut pending = VecDeque::new();
    let mut latest_move = None;
    let mut last_sent_move = None;
    let mut last_move_at = Instant::now() - HEARTBEAT;
    let mut retry_at = Instant::now();

    'worker: loop {
        loop {
            match commands.try_recv() {
                Ok(Command::SetWorld(world)) => {
                    if desired_world.as_deref() != Some(world.as_str()) {
                        desired_world = Some(world);
                        disconnect(&mut socket, &mut connected_world, &events);
                        pending.clear();
                        last_sent_move = None;
                        retry_at = Instant::now();
                    }
                }
                Ok(Command::Send(message)) => pending.push_back(message),
                Ok(Command::Move(movement)) => latest_move = Some(movement.movement),
                Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => break 'worker,
                Err(TryRecvError::Empty) => break,
            }
        }

        let Some(world) = desired_world.as_deref() else {
            thread::sleep(POLL);
            continue;
        };
        if socket.is_none() && Instant::now() >= retry_at {
            match socket_url(&base_url, &game_id, world)
                .and_then(|url| {
                    connect(url.as_str())
                        .map(|(socket, _)| socket)
                        .map_err(|error| error.to_string())
                })
                .and_then(|mut socket| {
                    set_nonblocking(&mut socket)
                        .map(|()| socket)
                        .map_err(|error| error.to_string())
                }) {
                Ok(next) => {
                    info!("backend connected: world_id={world}");
                    socket = Some(next);
                    connected_world = Some(world.to_owned());
                    let _ = events.send(Event::Connected);
                }
                Err(error) => {
                    debug!("backend connection failed: world_id={world} error={error}");
                    retry_at = Instant::now() + RECONNECT;
                }
            }
        }

        let Some(current) = socket.as_mut() else {
            thread::sleep(POLL);
            continue;
        };
        let mut failed = false;
        while let Some(message) = pending.front() {
            match current.send(Message::Text(message.clone().into())) {
                Ok(()) => {
                    pending.pop_front();
                }
                Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        if !failed && let Some(movement) = latest_move.as_ref() {
            let now = Instant::now();
            let json = serde_json::json!({
                "type": "move",
                "x": movement.position[0],
                "y": movement.position[1],
                "z": movement.position[2],
                "yaw": movement.yaw,
                "moving": movement.moving,
                "sprinting": movement.sprinting,
                "respawnEventId": movement.respawn_event_id,
            })
            .to_string();
            if now.duration_since(last_move_at) >= MOVE_INTERVAL
                && (last_sent_move.as_deref() != Some(json.as_str())
                    || now.duration_since(last_move_at) >= HEARTBEAT)
            {
                match current.send(Message::Text(json.clone().into())) {
                    Ok(()) => {
                        last_sent_move = Some(json);
                        last_move_at = now;
                    }
                    Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
                    Err(_) => failed = true,
                }
            }
        }
        if !failed {
            loop {
                match current.read() {
                    Ok(Message::Text(message)) => {
                        let _ = events.send(Event::Message(message.to_string()));
                    }
                    Ok(Message::Ping(payload)) => {
                        if current.send(Message::Pong(payload)).is_err() {
                            failed = true;
                            break;
                        }
                    }
                    Ok(Message::Close(_)) => {
                        failed = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
        }
        if failed {
            disconnect(&mut socket, &mut connected_world, &events);
            retry_at = Instant::now() + RECONNECT;
        }
        thread::sleep(POLL);
    }
    disconnect(&mut socket, &mut connected_world, &events);
}

fn socket_url(base: &Url, game_id: &str, world: &str) -> Result<Url, String> {
    let mut url = base.clone();
    let scheme = match url.scheme() {
        "http" | "ws" => "ws",
        "https" | "wss" => "wss",
        scheme => return Err(format!("unsupported backend URL scheme: {scheme}")),
    };
    url.set_scheme(scheme)
        .map_err(|()| "could not set WebSocket scheme")?;
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/world/{}", encode_segment(world)));
    url.query_pairs_mut()
        .append_pair("client", "desktop")
        .append_pair("game", game_id);
    Ok(url)
}

fn set_nonblocking(socket: &mut Socket) -> std::io::Result<()> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream.set_nonblocking(true),
        MaybeTlsStream::NativeTls(stream) => stream.get_mut().set_nonblocking(true),
        _ => Err(std::io::Error::new(
            ErrorKind::Unsupported,
            "unsupported TLS stream",
        )),
    }
}

fn encode_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => {
                char::from(byte).to_string()
            }
            byte => format!("%{byte:02X}"),
        })
        .collect()
}

fn disconnect(
    socket: &mut Option<Socket>,
    connected_world: &mut Option<String>,
    events: &Sender<Event>,
) {
    if socket.take().is_some() {
        *connected_world = None;
        let _ = events.send(Event::Disconnected);
    }
}

#[cfg(test)]
mod tests {
    use super::encode_segment;

    #[test]
    fn encodes_world_path_segments_without_encoding_colons() {
        assert_eq!(encode_segment("arena:7/blue"), "arena:7%2Fblue");
    }
}
