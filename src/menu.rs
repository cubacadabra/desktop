use crate::network::CatalogEntry;
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor, wgpu};
use egui_winit::State as EguiState;
use std::mem;
use winit::{event::WindowEvent, window::Window};

const LOGO_BYTES: &[u8] = include_bytes!("../../rust/assets/images/logo.png");

#[path = "menu_view.rs"]
mod view;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WebRequest {
    Account,
}

#[derive(Debug)]
pub(crate) enum GameRequest {
    Bundled(String),
    Remote(CatalogEntry),
}

pub(crate) struct PreparedMenu {
    paint_jobs: Vec<egui::ClippedPrimitive>,
    screen: ScreenDescriptor,
}

pub(crate) struct PlayerMenu {
    context: egui::Context,
    state: EguiState,
    renderer: Renderer,
    pending_textures_delta: egui::TexturesDelta,
    signed_in_name: Option<String>,
    sign_in_requested: bool,
    web_request: Option<WebRequest>,
    game_request: Option<GameRequest>,
    catalog_requested: bool,
    catalog: CatalogState,
    loading_game_id: Option<String>,
    game_error: Option<String>,
    screen: Screen,
    auth_pending: bool,
    logo_texture: egui::TextureHandle,
    about_open: bool,
    about_texture: Option<egui::TextureId>,
}

fn load_logo_texture(context: &egui::Context) -> egui::TextureHandle {
    let image = image::load_from_memory(LOGO_BYTES)
        .expect("bundled Cubacadabra logo should decode")
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    context.load_texture(
        "cubacadabra-player-logo",
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        egui::TextureOptions::LINEAR,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Screen {
    Home,
    Catalog,
}

#[derive(Default)]
struct CatalogState {
    entries: Vec<CatalogEntry>,
    loading: bool,
    error: Option<String>,
}

impl PlayerMenu {
    pub(crate) fn new(
        window: &Window,
        game_renderer: &cubacadabra_client::native::Renderer,
    ) -> Self {
        let context = egui::Context::default();
        context.set_theme(egui::ThemePreference::System);
        let state = EguiState::new(
            context.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(4_096),
        );
        let logo_texture = load_logo_texture(&context);
        Self {
            context,
            state,
            renderer: Renderer::new(
                game_renderer.device(),
                game_renderer.studio_overlay_format(),
                RendererOptions::default(),
            ),
            pending_textures_delta: egui::TexturesDelta::default(),
            signed_in_name: None,
            sign_in_requested: false,
            web_request: None,
            game_request: None,
            catalog_requested: false,
            catalog: CatalogState::default(),
            loading_game_id: None,
            game_error: None,
            screen: Screen::Home,
            auth_pending: false,
            logo_texture,
            about_open: false,
            about_texture: None,
        }
    }

    pub(crate) fn open_about(&mut self) {
        self.about_open = true;
    }

    pub(crate) fn set_about_preview_texture(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
    ) {
        self.about_texture = Some(self.renderer.register_native_texture(
            device,
            texture,
            wgpu::FilterMode::Linear,
        ));
    }

    pub(crate) const fn about_is_open(&self) -> bool {
        self.about_open
    }

    pub(crate) fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    pub(crate) fn prepare(&mut self, window: &Window, current_game_id: &str) -> PreparedMenu {
        let input = self.state.take_egui_input(window);
        let context = self.context.clone();
        let output = context.run_ui(input, |context| {
            egui::CentralPanel::default().show(context, |ui| self.show(ui, current_game_id));
        });
        self.state
            .handle_platform_output(window, output.platform_output);
        let pixels_per_point = self.context.pixels_per_point();
        let paint_jobs = self.context.tessellate(output.shapes, pixels_per_point);
        let size = window.inner_size();
        self.pending_textures_delta.append(output.textures_delta);
        PreparedMenu {
            paint_jobs,
            screen: ScreenDescriptor {
                size_in_pixels: [size.width, size.height],
                pixels_per_point,
            },
        }
    }

    pub(crate) fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::TextureView,
        prepared: PreparedMenu,
    ) {
        let textures_delta = mem::take(&mut self.pending_textures_delta);
        for (id, image_delta) in &textures_delta.set {
            self.renderer
                .update_texture(device, queue, *id, image_delta);
        }
        let command_buffers = self.renderer.update_buffers(
            device,
            queue,
            encoder,
            &prepared.paint_jobs,
            &prepared.screen,
        );
        debug_assert!(command_buffers.is_empty());
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Cubacadabra player menu"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.renderer.render(
            &mut pass.forget_lifetime(),
            &prepared.paint_jobs,
            &prepared.screen,
        );
        for id in &textures_delta.free {
            self.renderer.free_texture(id);
        }
    }

    pub(crate) fn take_sign_in_requested(&mut self) -> bool {
        mem::take(&mut self.sign_in_requested)
    }

    pub(crate) fn take_web_request(&mut self) -> Option<WebRequest> {
        self.web_request.take()
    }

    pub(crate) fn take_game_request(&mut self) -> Option<GameRequest> {
        self.game_request.take()
    }

    pub(crate) fn take_catalog_requested(&mut self) -> bool {
        mem::take(&mut self.catalog_requested)
    }

    pub(crate) fn set_auth_pending(&mut self, pending: bool) {
        self.auth_pending = pending;
    }

    pub(crate) fn set_authenticated(&mut self, user: &crate::network::AuthUser) {
        self.signed_in_name = Some(user.username.clone().unwrap_or_else(|| user.name.clone()));
    }

    pub(crate) fn begin_catalog_load(&mut self) {
        self.screen = Screen::Catalog;
        self.catalog.loading = true;
        self.catalog.error = None;
        self.catalog.entries.clear();
    }

    pub(crate) fn set_catalog_result(&mut self, entries: Vec<CatalogEntry>) {
        self.screen = Screen::Catalog;
        self.catalog.loading = false;
        self.catalog.error = None;
        self.catalog.entries = entries;
    }

    pub(crate) fn set_catalog_error(&mut self, message: String) {
        self.screen = Screen::Catalog;
        self.catalog.loading = false;
        self.catalog.error = Some(message);
    }

    pub(crate) fn set_game_loading(&mut self, game_id: Option<String>) {
        self.loading_game_id = game_id;
        self.game_error = None;
    }

    pub(crate) fn set_game_error(&mut self, message: String) {
        self.loading_game_id = None;
        self.game_error = Some(message);
    }

    pub(crate) fn back_to_home(&mut self) {
        self.screen = Screen::Home;
    }
}
