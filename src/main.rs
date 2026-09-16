mod assets;
mod network;
mod options;

use cubacadabra_client::{ClientAction, ClientSession, native::Renderer};
use log::{debug, error, info};
use options::DesktopError;
use std::{
    collections::HashSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowAttributes},
};

const WINDOW_WIDTH: f64 = 1280.0;
const WINDOW_HEIGHT: f64 = 800.0;

struct DesktopApp {
    package_root: PathBuf,
    client: ClientSession,
    network: network::BackendClient,
    atlas: Option<assets::ImageAtlas>,
    window: Option<Window>,
    renderer: Option<Renderer>,
    keys: HashSet<KeyCode>,
    jump_queued: bool,
    pointer: Option<(f32, f32)>,
    camera_active: bool,
    ui_active: bool,
    look_delta: (f32, f32),
    zoom_delta: f32,
    last_frame: Instant,
}

impl DesktopApp {
    fn load(package_root: PathBuf) -> Result<Self, Box<dyn Error>> {
        let manifest_source = read_package_file(&package_root, "manifest.json")?;
        let script_source = read_package_file(&package_root, "game.luau")?;
        let client = ClientSession::load(&manifest_source, &script_source)?;
        let network = network::BackendClient::new(client.game_id()).map_err(DesktopError)?;
        let atlas = assets::load(&package_root, &manifest_source)?;
        info!("desktop loaded: game_id={}", client.game_id());
        Ok(Self {
            package_root,
            client,
            network,
            atlas,
            window: None,
            renderer: None,
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
        let title = self
            .package_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("game");
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
        self.window = Some(window);
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
        let sprint = self.keys.contains(&KeyCode::ShiftLeft)
            || self.keys.contains(&KeyCode::ShiftRight);
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
        self.dispatch_actions();
        if let Some(movement) = self.client.local_movement(moving, sprint) {
            self.network.send_move(movement);
        }

        if let Some(renderer) = &mut self.renderer {
            renderer.sync(self.client.engine());
            renderer.draw();
        }
    }

    fn drain_network(&mut self) {
        while let Some(event) = self.network.try_recv() {
            match event {
                network::Event::Connected => self.client.transport_connected(),
                network::Event::Disconnected => self.client.transport_disconnected(),
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

impl ApplicationHandler for DesktopApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(error) = self.create_window(event_loop) {
            error!("Cubacadabra Desktop: {error}");
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size),
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.window {
                    self.resize(window.inner_size());
                }
            }
            WindowEvent::RedrawRequested => {
                self.render();
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => self.handle_key(event),
            WindowEvent::CursorMoved { position, .. } => {
                let next = self.logical_pointer(position.x, position.y);
                if let Some(previous) = self.pointer
                    && self.camera_active
                    && !self.ui_active
                {
                    self.look_delta.0 += next.0 - previous.0;
                    self.look_delta.1 += next.1 - previous.1;
                }
                if self.ui_active {
                    let _ = self.client.ui_pointer_event(1, 1, next.0, next.1);
                }
                self.pointer = Some(next);
            }
            WindowEvent::MouseInput { state, button, .. } => self.handle_mouse(state, button),
            WindowEvent::MouseWheel { delta, .. } => {
                self.zoom_delta += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 0.9,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 / 100.0,
                };
            }
            WindowEvent::Focused(false) => self.clear_input(),
            _ => {}
        }
    }
}

impl DesktopApp {
    fn handle_key(&mut self, event: KeyEvent) {
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        match event.state {
            ElementState::Pressed => {
                if code == KeyCode::Space && !event.repeat {
                    self.jump_queued = true;
                }
                self.keys.insert(code);
            }
            ElementState::Released => {
                self.keys.remove(&code);
            }
        }
    }

    fn handle_mouse(&mut self, state: ElementState, button: MouseButton) {
        let Some((x, y)) = self.pointer else { return };
        match (state, button) {
            (ElementState::Pressed, MouseButton::Left) => {
                self.ui_active = self.client.ui_pointer_event(1, 0, x, y);
                self.camera_active = !self.ui_active;
            }
            (ElementState::Pressed, MouseButton::Right) => {
                self.camera_active = true;
                self.ui_active = false;
            }
            (ElementState::Released, MouseButton::Left) => {
                if self.ui_active {
                    let _ = self.client.ui_pointer_event(1, 2, x, y);
                }
                self.ui_active = false;
                self.camera_active = false;
            }
            (ElementState::Released, MouseButton::Right) => self.camera_active = false,
            _ => {}
        }
    }
}

fn axis(keys: &HashSet<KeyCode>, positive: &[KeyCode], negative: &[KeyCode]) -> f32 {
    f32::from(positive.iter().any(|key| keys.contains(key)))
        - f32::from(negative.iter().any(|key| keys.contains(key)))
}

fn read_package_file(root: &Path, name: &str) -> Result<String, Box<dyn Error>> {
    fs::read_to_string(root.join(name)).map_err(|error| {
        Box::new(DesktopError(format!(
            "could not read package {name} in {}: {error}",
            root.display()
        ))) as Box<dyn Error>
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();
    debug!("Desktop logging initialized");
    let options = options::parse()?;
    let mut app = DesktopApp::load(options.package)?;
    let event_loop = EventLoop::builder().build()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut app)?;
    Ok(())
}
