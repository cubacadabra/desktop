use cubacadabra_client::ClientMovement;
use log::{debug, info, warn};
use std::{
    collections::VecDeque,
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc::{self, Receiver, Sender, TryRecvError}},
    thread,
    time::{Duration, Instant},
};
use tungstenite::{
    ClientRequestBuilder, Message, WebSocket, connect, stream::MaybeTlsStream,
};
use url::Url;

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
    BrowseGames,
    Account,
    About,
}

impl WebPage {
    fn location(self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::BrowseGames => ("/my-cube/", Some("cubes")),
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
                    thread::spawn(move || run_browser_auth(&backend_url, &events, &auth));
                }
                Ok(Command::OpenWeb(page)) => match web_page_url(&base_url, page) {
                    Ok(url) if open_browser(url.as_str()) => {}
                    Ok(url) => warn!("could not open web page: {url}"),
                    Err(message) => warn!("could not resolve web page: {message}"),
                },
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

fn http_url(base: &Url, path: &str) -> Result<Url, String> {
    let mut url = base.clone();
    let scheme = match url.scheme() {
        "http" | "ws" => "http",
        "https" | "wss" => "https",
        scheme => return Err(format!("unsupported backend URL scheme: {scheme}")),
    };
    url.set_scheme(scheme)
        .map_err(|()| "could not set HTTP scheme".to_owned())?;
    let base_path = url.path().trim_end_matches('/');
    let request_path = if path.starts_with('/') {
        format!("{base_path}{path}")
    } else if base_path.is_empty() {
        format!("/{path}")
    } else {
        format!("{base_path}/{path}")
    };
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Url::parse(&format!("{}{}", url.as_str().trim_end_matches('/'), request_path))
        .map_err(|error| format!("could not build HTTP URL: {error}"))
}

fn run_browser_auth(
    backend_url: &Url,
    events: &Sender<Event>,
    auth: &Arc<Mutex<Option<AuthSession>>>,
) {
    let result = browser_auth_url(backend_url).and_then(|(url, redirect_uri, state, listener)| {
        if !open_browser(&url) {
            return Err(
                "Could not open the default browser. Check your browser association and try again."
                    .to_owned(),
            );
        }
        let _ = events.send(Event::AuthStarted);
        wait_for_browser_callback(backend_url, events, listener, &redirect_uri, &state, auth)
    });
    if let Err(message) = result {
        warn!("browser authentication failed: {message}");
        let _ = events.send(Event::AuthError(message));
    }
}

fn browser_auth_url(backend_url: &Url) -> Result<(String, String, String, TcpListener), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("could not start the local login callback: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("could not configure the local login callback: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("could not read the local login callback address: {error}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/auth/callback");
    let mut state_bytes = [0u8; 32];
    getrandom::fill(&mut state_bytes)
        .map_err(|error| format!("could not create login state: {error}"))?;
    let state = state_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut login_url = web_base_url(backend_url)?;
    login_url
        .query_pairs_mut()
        .append_pair("app_redirect_uri", &redirect_uri)
        .append_pair("state", &state);
    Ok((login_url.to_string(), redirect_uri, state, listener))
}

fn web_base_url(backend_url: &Url) -> Result<Url, String> {
    let mut url = if let Some(raw) = std::env::var(WEB_URL_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        Url::parse(raw.trim()).map_err(|error| format!("{WEB_URL_ENV} is invalid: {error}"))?
    } else if backend_url
        .host_str()
        .is_some_and(|host| host == "localhost" || host == "127.0.0.1")
    {
        Url::parse("http://127.0.0.1:5173").expect("local web URL must be valid")
    } else {
        Url::parse("https://cubacadabra.com").expect("production web URL must be valid")
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!("{WEB_URL_ENV} must be an http(s) URL with a host"));
    }
    url.set_path("/login/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn web_page_url(backend_url: &Url, page: WebPage) -> Result<Url, String> {
    Ok(apply_web_page(web_base_url(backend_url)?, page))
}

fn apply_web_page(mut url: Url, page: WebPage) -> Url {
    let (path, fragment) = page.location();
    url.set_path(path);
    url.set_query(None);
    url.set_fragment(fragment);
    url
}

fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return false;
    command.arg(url).spawn().is_ok()
}

fn wait_for_browser_callback(
    backend_url: &Url,
    events: &Sender<Event>,
    listener: TcpListener,
    redirect_uri: &str,
    expected_state: &str,
    auth: &Arc<Mutex<Option<AuthSession>>>,
) -> Result<(), String> {
    let deadline = Instant::now() + AUTH_TIMEOUT;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .map_err(|error| format!("could not configure login callback: {error}"))?;
                let request = read_callback_request(&mut stream)?;
                let callback = Url::parse(&format!("http://127.0.0.1{request}"))
                    .map_err(|error| format!("invalid login callback: {error}"))?;
                if callback.path() != "/auth/callback"
                    || callback
                        .query_pairs()
                        .find(|(key, _)| key == "state")
                        .map(|(_, value)| value.to_string())
                        .as_deref()
                        != Some(expected_state)
                {
                    write_browser_response(&mut stream, false);
                    continue;
                }
                let Some(code) = callback
                    .query_pairs()
                    .find(|(key, _)| key == "code")
                    .map(|(_, value)| value.to_string())
                else {
                    write_browser_response(&mut stream, false);
                    return Err("The browser login did not return an authorization code.".to_owned());
                };
                let session = match exchange_browser_code(backend_url, &code, redirect_uri) {
                    Ok(session) => session,
                    Err(message) => {
                        write_browser_response(&mut stream, false);
                        return Err(message);
                    }
                };
                if let Ok(mut current) = auth.lock() {
                    *current = Some(session.clone());
                }
                write_browser_response(&mut stream, true);
                let _ = events.send(Event::AuthCompleted { user: session.user });
                return Ok(());
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(AUTH_POLL_INTERVAL);
            }
            Err(error) => return Err(format!("login callback failed: {error}")),
        }
    }
    Err("Browser login timed out. Try signing in again.".to_owned())
}

fn read_callback_request(stream: &mut TcpStream) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(1024);
    let mut buffer = [0u8; 1024];
    while bytes.len() < 8 * 1024 {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(error) => return Err(format!("could not read login callback: {error}")),
        }
    }
    let request = String::from_utf8(bytes)
        .map_err(|_| "login callback was not valid HTTP".to_owned())?;
    request
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .ok_or_else(|| "login callback was not a GET request".to_owned())
}

fn write_browser_response(stream: &mut TcpStream, success: bool) {
    let (title, message) = if success {
        ("Signed in to cubacadabra", "You can close this browser window.")
    } else {
        ("Sign-in failed", "Cubacadabra could not finish signing you in.")
    };
    let body = format!(
        "<!doctype html><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>{title}</title><style>body{{font:16px system-ui,sans-serif;background:#17181c;color:#f3f4f6;display:grid;place-items:center;min-height:100vh;margin:0}}main{{max-width:34rem;padding:32px}}p{{color:#b6bac5;line-height:1.5}}</style><main><h1>{title}</h1><p>{message}</p></main>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
}

fn exchange_browser_code(
    backend_url: &Url,
    code: &str,
    redirect_uri: &str,
) -> Result<AuthSession, String> {
    let endpoint = http_url(backend_url, "/auth/app/exchange")?;
    let payload = serde_json::json!({ "code": code, "redirect_uri": redirect_uri });
    let mut response = ureq::post(endpoint.as_str())
        .header("content-type", "application/json")
        .send(payload.to_string())
        .map_err(|error| format!("could not exchange the browser login: {error}"))?;
    let source = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("could not read the browser login response: {error}"))?;
    if !response.status().is_success() {
        return Err("The browser login could not be exchanged for a desktop session.".to_owned());
    }
    let value: serde_json::Value = serde_json::from_str(&source)
        .map_err(|error| format!("browser login response was invalid: {error}"))?;
    let user: AuthUser = serde_json::from_value(
        value
            .get("user")
            .cloned()
            .ok_or_else(|| "browser login response did not include a user".to_owned())?,
    )
    .map_err(|error| format!("browser login user was invalid: {error}"))?;
    let access_token = value
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "browser login response did not include an access token".to_owned())?;
    let refresh_token = value
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "browser login response did not include a refresh token".to_owned())?;
    Ok(AuthSession {
        access_token: access_token.to_owned(),
        refresh_token: refresh_token.to_owned(),
        user,
    })
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_URL, WebPage, apply_web_page, encode_segment};
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
            apply_web_page(base.clone(), WebPage::BrowseGames).as_str(),
            "https://cubacadabra.com/my-cube/#cubes"
        );
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
