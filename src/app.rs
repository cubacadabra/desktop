use crate::{assets, bundled, menu, network, options::DesktopError};
use cubacadabra_client::{ClientAction, ClientSession, native::Renderer};
use log::{error, info};
use std::{collections::HashSet, error::Error, path::PathBuf, time::Instant};
use winit::{
    dpi::{LogicalSize, PhysicalSize},
    event_loop::ActiveEventLoop,
    keyboard::KeyCode,
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowAttributes},
};

#[path = "app_input.rs"]
mod input;
#[path = "app_package.rs"]
mod package;
#[path = "app_window.rs"]
mod window;

const WINDOW_WIDTH: f64 = 1280.0;
const WINDOW_HEIGHT: f64 = 800.0;

fn axis(keys: &HashSet<KeyCode>, positive: &[KeyCode], negative: &[KeyCode]) -> f32 {
    f32::from(positive.iter().any(|key| keys.contains(key)))
        - f32::from(negative.iter().any(|key| keys.contains(key)))
}

pub(crate) struct DesktopApp {
    package_name: String,
    client: ClientSession,
    about_preview: cubacadabra_about_preview::AboutPreview,
    network: network::BackendClient,
    atlas: Option<assets::ImageAtlas>,
    models: Vec<assets::ModelAsset>,
    window: Option<Window>,
    renderer: Option<Renderer>,
    menu: Option<menu::PlayerMenu>,
    in_game: bool,
    auth_pending: bool,
    username: String,
    keys: HashSet<KeyCode>,
    jump_queued: bool,
    pointer: Option<(f32, f32)>,
    camera_active: bool,
    ui_active: bool,
    look_delta: (f32, f32),
    zoom_delta: f32,
    last_frame: Instant,
}

struct PackageContent {
    id: String,
    manifest: String,
    script: String,
    atlas: Option<assets::ImageAtlas>,
    models: Vec<assets::ModelAsset>,
}

impl DesktopApp {
    pub(crate) fn load(package_root: PathBuf) -> Result<Self, Box<dyn Error>> {
        let manifest_source = crate::read_package_file(&package_root, "manifest.json")?;
        let script_source = crate::read_package_file(&package_root, "game.luau")?;
        let atlas = assets::load(&package_root, &manifest_source)?;
        let models = assets::load_models(&package_root, &manifest_source)?;
        let package_name = package_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("game")
            .to_owned();
        Self::from_package(
            PackageContent {
                id: package_name,
                manifest: manifest_source,
                script: script_source,
                atlas,
                models,
            },
            true,
        )
    }

    pub(crate) fn load_bundled() -> Result<Self, Box<dyn Error>> {
        let package = bundled::PACKAGES
            .first()
            .ok_or_else(|| DesktopError("no bundled game packages were generated".into()))?;
        Self::from_package(
            PackageContent {
                id: package.id.to_owned(),
                manifest: package.manifest.to_owned(),
                script: package.script.to_owned(),
                atlas: assets::load_bundled(package.files, package.manifest)?,
                models: assets::load_bundled_models(package.files, package.manifest)?,
            },
            true,
        )
    }

    fn from_package(package: PackageContent, in_game: bool) -> Result<Self, Box<dyn Error>> {
        let mut client = ClientSession::load(&package.manifest, &package.script)?;
        let about_preview = cubacadabra_about_preview::AboutPreview::new().map_err(DesktopError)?;
        let username = std::env::var("CUBACADABRA_USERNAME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Player".to_owned());
        let _ = client.engine_mut().set_username_value(&username);
        let network = network::BackendClient::new(client.game_id()).map_err(DesktopError)?;
        info!("desktop loaded game: game_id={}", client.game_id());
        Ok(Self {
            package_name: package.id,
            client,
            about_preview,
            network,
            atlas: package.atlas,
            models: package.models,
            window: None,
            renderer: None,
            menu: None,
            in_game,
            auth_pending: false,
            username,
            keys: HashSet::new(),
            jump_queued: false,
            pointer: None,
            camera_active: false,
            ui_active: false,
            look_delta: (0.0, 0.0),
            zoom_delta: 0.0,
            last_frame: Instant::now(),
        })
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let title = &self.package_name;
        let window = event_loop.create_window(
            WindowAttributes::default()
                .with_title(format!("Cubacadabra — {title}"))
                .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT)),
        )?;
        let size = window.inner_size();
        let mut renderer = Renderer::new(
            window.display_handle()?.as_raw(),
            window.window_handle()?.as_raw(),
            size.width as f32,
            size.height as f32,
        )
        .ok_or_else(|| DesktopError("the shared wgpu renderer could not start".into()))?;
        if let Some(atlas) = &self.atlas
            && !renderer.set_package_image_atlas(
                atlas.width,
                atlas.height,
                &atlas.pixels,
                atlas.regions.clone(),
            )
        {
            return Err(Box::new(DesktopError(
                "the game's image atlas could not be uploaded".into(),
            )));
        }
        for model in &self.models {
            renderer
                .register_world_mesh(&model.id, &model.bytes)
                .map_err(|error| {
                    DesktopError(format!("world model {} was rejected: {error}", model.id))
                })?;
        }
        let mut player_menu = menu::PlayerMenu::new(&window, &renderer);
        player_menu.set_about_preview_texture(renderer.device(), renderer.about_preview_texture());
        self.window = Some(window);
        self.menu = Some(player_menu);
        self.renderer = Some(renderer);
        self.update_viewport();
        self.request_redraw();
        Ok(())
    }

    fn update_viewport(&mut self) {
        let Some(window) = &self.window else { return };
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        self.client.set_ui_viewport_values(
            size.width as f32 / scale,
            size.height as f32 / scale,
            scale,
            0.0,
            0.0,
            0.0,
            0.0,
        );
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width > 0 && size.height > 0 {
            if let Some(renderer) = &mut self.renderer {
                renderer.resize(size.width as f32, size.height as f32);
            }
            self.update_viewport();
        }
    }

    fn render(&mut self) {
        self.drain_network();
        #[cfg(target_os = "macos")]
        if crate::macos::take_about_requested() {
            if let Some(menu) = &mut self.menu {
                menu.open_about();
            }
            self.clear_input();
        }
        if !self.in_game {
            self.render_menu();
            return;
        }
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;

        let forward = axis(
            &self.keys,
            &[KeyCode::KeyW, KeyCode::ArrowUp],
            &[KeyCode::KeyS, KeyCode::ArrowDown],
        );
        let strafe = axis(
            &self.keys,
            &[KeyCode::KeyD, KeyCode::ArrowRight],
            &[KeyCode::KeyA, KeyCode::ArrowLeft],
        );
        let moving = (forward * forward + strafe * strafe).sqrt() > 0.01;
        let sprint =
            self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
        self.client.set_input_values(
            forward,
            strafe,
            sprint,
            self.jump_queued,
            false,
            self.look_delta.0,
            self.look_delta.1,
            self.zoom_delta,
        );
        self.jump_queued = false;
        self.look_delta = (0.0, 0.0);
        self.zoom_delta = 0.0;
        self.dispatch_actions();
        self.client.step(delta);
        if self.drain_ui_events() {
            self.leave_game();
            return;
        }
        self.dispatch_actions();
        if let Some(movement) = self.client.local_movement(moving, sprint) {
            self.network.send_move(movement);
        }

        let prepared_about = if self
            .menu
            .as_ref()
            .is_some_and(menu::PlayerMenu::about_is_open)
        {
            let game_name = self.client.game_id().to_owned();
            match (&self.window, &mut self.menu) {
                (Some(window), Some(menu)) => Some(menu.prepare(window, &game_name, false)),
                _ => None,
            }
        } else {
            None
        };

        if let Some(renderer) = &mut self.renderer {
            renderer.sync(self.client.engine());
            match (&mut self.menu, prepared_about) {
                (Some(menu), Some(prepared)) => {
                    self.about_preview.step();
                    renderer.render_about_preview(self.about_preview.engine());
                    renderer.draw_with_overlay(|device, queue, encoder, destination| {
                        menu.paint(device, queue, encoder, destination, prepared);
                    });
                }
                _ => renderer.draw(),
            }
        }
    }

    fn drain_network(&mut self) {
        while let Some(event) = self.network.try_recv() {
            match event {
                network::Event::Connected => {
                    self.client.transport_connected();
                    self.network.send(
                        serde_json::json!({
                            "type": "set_username",
                            "username": self.username,
                        })
                        .to_string(),
                    );
                }
                network::Event::Disconnected => self.client.transport_disconnected(),
                network::Event::AuthStarted => {
                    self.auth_pending = true;
                    if let Some(menu) = &mut self.menu {
                        menu.set_auth_pending(true);
                    }
                    info!("browser sign-in started");
                }
                network::Event::AuthCompleted { user } => {
                    self.auth_pending = false;
                    if let Some(menu) = &mut self.menu {
                        menu.set_auth_pending(false);
                    }
                    let username = user.username.clone().unwrap_or_else(|| user.name.clone());
                    self.username = username.clone();
                    let _ = self.client.engine_mut().set_username_value(&username);
                    self.client.engine_mut().set_authenticated_value(true);
                    if let Some(menu) = &mut self.menu {
                        menu.set_authenticated(&user);
                    }
                    self.network.disconnect();
                    self.client.request_transport();
                    if self.in_game {
                        self.dispatch_actions();
                    }
                    info!("signed in as {}", user.name);
                }
                network::Event::AuthError(message) => {
                    self.auth_pending = false;
                    if let Some(menu) = &mut self.menu {
                        menu.set_auth_pending(false);
                    }
                    error!("sign-in failed: {message}");
                }
                network::Event::CatalogLoaded(entries) => {
                    if let Some(menu) = &mut self.menu {
                        menu.set_catalog_result(entries);
                    }
                }
                network::Event::CatalogError(message) => {
                    if let Some(menu) = &mut self.menu {
                        menu.set_catalog_error(message);
                    }
                }
                network::Event::PackageLoaded(package) => {
                    let package_id = package.id.clone();
                    match self.install_remote_package(package) {
                        Ok(()) => info!("desktop loaded remote game: game_id={package_id}"),
                        Err(error) => {
                            if let Some(menu) = &mut self.menu {
                                menu.set_game_error(error.to_string());
                            }
                        }
                    }
                }
                network::Event::PackageError { game_id, message } => {
                    error!("could not load remote game {game_id}: {message}");
                    if let Some(menu) = &mut self.menu {
                        menu.set_game_error(message);
                    }
                }
                network::Event::Message(source) => {
                    let _ = self.client.receive_text(&source);
                }
            }
        }
    }

    fn dispatch_actions(&mut self) {
        for action in self.client.poll_actions() {
            match action {
                ClientAction::SetWorld(world) => self.network.set_world(world),
                ClientAction::SendText(message) => self.network.send(message),
            }
        }
    }

    fn drain_ui_events(&mut self) -> bool {
        let mut leave_game = false;
        let mut open_about = false;
        let mut sign_in = false;
        while let Some(source) = self.client.poll_ui_event_json() {
            match crate::shared_ui_action(&source) {
                Some(crate::SharedUiAction::OpenAbout) => open_about = true,
                Some(crate::SharedUiAction::LeaveGame) => leave_game = true,
                Some(crate::SharedUiAction::SignIn) => sign_in = true,
                None => {}
            }
        }
        if open_about {
            if let Some(menu) = &mut self.menu {
                menu.open_about();
            }
            self.clear_input();
        }
        if sign_in {
            self.begin_browser_auth();
        }
        leave_game
    }

    fn begin_browser_auth(&mut self) {
        if self.auth_pending {
            return;
        }
        self.auth_pending = true;
        if let Some(menu) = &mut self.menu {
            menu.set_auth_pending(true);
        }
        self.network.begin_browser_auth();
    }

    fn leave_game(&mut self) {
        let returned_to_lobby = self.client.engine_mut().start_world_by_id("lobby");
        self.client.transport_disconnected();
        self.client.request_transport();
        self.network.disconnect();
        self.clear_input();
        self.in_game = false;
        info!("returned to player menu (lobby={returned_to_lobby})");
    }

    fn render_menu(&mut self) {
        let Some(window) = &self.window else { return };
        let game_name = self.client.game_id().to_owned();
        let (sign_in_requested, web_request, catalog_requested, game_request) = {
            let Some(menu) = &mut self.menu else { return };
            let prepared = menu.prepare(window, &game_name, true);
            if let Some(renderer) = &mut self.renderer {
                if menu.about_is_open() {
                    self.about_preview.step();
                    renderer.render_about_preview(self.about_preview.engine());
                }
                renderer.draw_with_overlay(|device, queue, encoder, destination| {
                    menu.paint(device, queue, encoder, destination, prepared);
                });
            }
            (
                menu.take_sign_in_requested(),
                menu.take_web_request(),
                menu.take_catalog_requested(),
                menu.take_game_request(),
            )
        };
        if sign_in_requested {
            self.begin_browser_auth();
        }
        if let Some(request) = web_request {
            let page = match request {
                menu::WebRequest::Account => network::WebPage::Account,
            };
            self.network.open_web(page);
        }
        if catalog_requested {
            if let Some(menu) = &mut self.menu {
                menu.begin_catalog_load();
            }
            self.network.load_catalog();
        }
        if let Some(request) = game_request {
            match request {
                menu::GameRequest::Bundled(game_id) => {
                    info!("requesting bundled game: game_id={game_id}");
                    if let Some(menu) = &mut self.menu {
                        menu.set_game_loading(Some(game_id.clone()));
                    }
                    if let Err(error) = self.install_bundled_game(&game_id) {
                        if let Some(menu) = &mut self.menu {
                            menu.set_game_error(error.to_string());
                        }
                    }
                }
                menu::GameRequest::Remote(entry) => {
                    info!(
                        "requesting catalog game: game_id={} package_url={}",
                        entry.id, entry.package_url
                    );
                    if let Some(menu) = &mut self.menu {
                        menu.set_game_loading(Some(entry.id.clone()));
                    }
                    self.network.load_package(entry);
                }
            }
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn logical_pointer(&self, x: f64, y: f64) -> (f32, f32) {
        let scale = self.window.as_ref().map_or(1.0, Window::scale_factor) as f32;
        (x as f32 / scale, y as f32 / scale)
    }

    fn clear_input(&mut self) {
        self.keys.clear();
        self.jump_queued = false;
        self.camera_active = false;
        self.ui_active = false;
        self.look_delta = (0.0, 0.0);
        self.zoom_delta = 0.0;
        let _ = self.client.ui_pointer_event(1, 3, 0.0, 0.0);
    }
}
