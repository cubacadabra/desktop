use cubacadabra_client::ClientMovement;
use log::{debug, info, warn};
use std::{
    collections::VecDeque,
    io::ErrorKind,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{ClientRequestBuilder, Message, WebSocket, connect, stream::MaybeTlsStream};
use url::Url;

#[path = "network_auth.rs"]
mod network_auth;
#[path = "network_package.rs"]
mod network_package;

const ENV: &str = "CUBACADABRA_BACKEND_URL";
const WEB_URL_ENV: &str = "CUBACADABRA_WEB_URL";
#[cfg(debug_assertions)]
const DEFAULT_URL: &str = "http://127.0.0.1:8787";
#[cfg(not(debug_assertions))]
const DEFAULT_URL: &str = "https://api.cubacadabra.com";
const RECONNECT: Duration = Duration::from_millis(750);
const POLL: Duration = Duration::from_millis(10);
const MOVE_INTERVAL: Duration = Duration::from_millis(83);
const HEARTBEAT: Duration = Duration::from_secs(30);
const AUTH_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const AUTH_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub enum Event {
    Connected,
    Disconnected,
    Message(String),
    AuthStarted,
    AuthCompleted { user: AuthUser },
    AuthError(String),
    CatalogLoaded(Vec<CatalogEntry>),
    CatalogError(String),
    PackageLoaded(RemoteGamePackage),
    PackageError { game_id: String, message: String },
}

#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub package_url: Url,
}

#[derive(Debug)]
pub struct RemoteGamePackage {
    pub id: String,
    pub manifest: String,
    pub script: String,
    pub files: Vec<(String, Vec<u8>)>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub name: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct AuthSession {
    pub access_token: String,
    pub refresh_token: String,
    pub user: AuthUser,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebPage {
    Account,
    About,
}

impl WebPage {
    fn location(self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::Account => ("/my-cube/", None),
            Self::About => ("/about/", None),
        }
    }
}

struct Move {
    movement: ClientMovement,
}

enum Command {
    SetWorld(String),
    Disconnect,
    Send(String),
    Move(Move),
    BeginBrowserAuth,
    OpenWeb(WebPage),
    LoadCatalog,
    LoadPackage(CatalogEntry),
    Shutdown,
}

type Socket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

pub struct BackendClient {
    commands: Sender<Command>,
    events: Receiver<Event>,
    worker: Option<thread::JoinHandle<()>>,
    auth: Arc<Mutex<Option<AuthSession>>>,
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
        let auth = Arc::new(Mutex::new(None));
        let worker_auth = Arc::clone(&auth);
        let game_id = game_id.to_owned();
        let worker = thread::Builder::new()
            .name("desktop-backend".into())
            .spawn(move || run_worker(url, game_id, command_receiver, events, worker_auth))
            .map_err(|error| format!("could not start backend worker: {error}"))?;
        Ok(Self {
            commands,
            events: event_receiver,
            worker: Some(worker),
            auth,
        })
    }

    pub fn set_world(&self, world: String) {
        let _ = self.commands.send(Command::SetWorld(world));
    }

    pub fn disconnect(&self) {
        let _ = self.commands.send(Command::Disconnect);
    }

    pub fn send(&self, message: String) {
        let _ = self.commands.send(Command::Send(message));
    }

    pub fn send_move(&self, movement: ClientMovement) {
        let _ = self.commands.send(Command::Move(Move { movement }));
    }

    pub fn begin_browser_auth(&self) {
        let _ = self.commands.send(Command::BeginBrowserAuth);
    }

    pub fn open_web(&self, page: WebPage) {
        let _ = self.commands.send(Command::OpenWeb(page));
    }

    pub fn load_catalog(&self) {
        let _ = self.commands.send(Command::LoadCatalog);
    }

    pub fn load_package(&self, entry: CatalogEntry) {
        let _ = self.commands.send(Command::LoadPackage(entry));
    }

    pub fn set_auth_session(&self, session: AuthSession) {
        if let Ok(mut current) = self.auth.lock() {
            *current = Some(session);
        }
    }

    #[allow(dead_code)]
    pub fn auth_session(&self) -> Option<AuthSession> {
        self.auth.lock().ok().and_then(|session| session.clone())
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
    auth: Arc<Mutex<Option<AuthSession>>>,
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
                Ok(Command::Disconnect) => {
                    desired_world = None;
                    disconnect(&mut socket, &mut connected_world, &events);
                    pending.clear();
                    last_sent_move = None;
                }
                Ok(Command::Send(message)) => pending.push_back(message),
                Ok(Command::Move(movement)) => latest_move = Some(movement.movement),
                Ok(Command::BeginBrowserAuth) => {
                    let backend_url = base_url.clone();
                    let auth = Arc::clone(&auth);
                    let events = events.clone();
                    thread::spawn(move || {
                        network_auth::run_browser_auth(&backend_url, &events, &auth)
                    });
                }
                Ok(Command::OpenWeb(page)) => match network_auth::web_page_url(&base_url, page) {
                    Ok(url) if network_auth::open_browser(url.as_str()) => {}
                    Ok(url) => warn!("could not open web page: {url}"),
                    Err(message) => warn!("could not resolve web page: {message}"),
                },
                Ok(Command::LoadCatalog) => {
                    let backend_url = base_url.clone();
                    let events = events.clone();
                    thread::spawn(move || match network_package::load_catalog(&backend_url) {
                        Ok(entries) => {
                            let _ = events.send(Event::CatalogLoaded(entries));
                        }
                        Err(message) => {
                            let _ = events.send(Event::CatalogError(message));
                        }
                    });
                }
                Ok(Command::LoadPackage(entry)) => {
                    let events = events.clone();
                    thread::spawn(move || {
                        let game_id = entry.id.clone();
                        match network_package::load_remote_package(&entry) {
                            Ok(package) => {
                                let _ = events.send(Event::PackageLoaded(package));
                            }
                            Err(message) => {
                                let _ = events.send(Event::PackageError { game_id, message });
                            }
                        }
                    });
                }
                Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => break 'worker,
                Err(TryRecvError::Empty) => break,
            }
        }

        let Some(world) = desired_world.as_deref() else {
            thread::sleep(POLL);
            continue;
        };
        if socket.is_none() && Instant::now() >= retry_at {
            let access_token = auth
                .lock()
                .ok()
                .and_then(|session| session.as_ref().map(|session| session.access_token.clone()));
            match socket_url(&base_url, &game_id, world)
                .and_then(|url| {
                    let uri = url
                        .as_str()
                        .parse()
                        .map_err(|error| format!("invalid WebSocket URL: {error}"))?;
                    let request = ClientRequestBuilder::new(uri);
                    let request = access_token.as_deref().map_or(request.clone(), |token| {
                        request.with_header("Authorization", format!("Bearer {token}"))
                    });
                    connect(request)
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
                Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                    break;
                }
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
                    Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                    }
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
                    Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                        break;
                    }
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
    use super::network_auth::apply_web_page;
    use super::{DEFAULT_URL, WebPage, encode_segment};
    use url::Url;

    #[test]
    fn encodes_world_path_segments_without_encoding_colons() {
        assert_eq!(encode_segment("arena:7/blue"), "arena:7%2Fblue");
    }

    #[test]
    fn build_profile_selects_the_expected_backend() {
        #[cfg(debug_assertions)]
        assert_eq!(DEFAULT_URL, "http://127.0.0.1:8787");
        #[cfg(not(debug_assertions))]
        assert_eq!(DEFAULT_URL, "https://api.cubacadabra.com");
    }

    #[test]
    fn web_control_plane_pages_have_stable_destinations() {
        let base = Url::parse("https://cubacadabra.com/login/?stale=true").unwrap();
        assert_eq!(
            apply_web_page(base.clone(), WebPage::Account).as_str(),
            "https://cubacadabra.com/my-cube/"
        );
        assert_eq!(
            apply_web_page(base, WebPage::About).as_str(),
            "https://cubacadabra.com/about/"
        );
    }
}
