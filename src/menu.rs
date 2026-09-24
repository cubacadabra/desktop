use crate::network::CatalogEntry;
use cubacadabra_client::AboutAnimation;
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor, wgpu};
use egui_winit::State as EguiState;
use std::mem;
use winit::{event::WindowEvent, window::Window};

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
    about_open: bool,
    about_video: Option<AboutVideo>,
    about_video_error: Option<String>,
}

struct AboutVideo {
    animation: AboutAnimation,
    texture: Option<egui::TextureHandle>,
    displayed_frame: u64,
}

impl AboutVideo {
    fn decode() -> Result<Self, String> {
        Ok(Self {
            animation: AboutAnimation::decode()?,
            texture: None,
            displayed_frame: u64::MAX,
        })
    }

    fn update_texture(&mut self, context: &egui::Context) -> &egui::TextureHandle {
        let (frame_id, width, height, pixels) = {
            let frame = self
                .animation
                .current_frame()
                .expect("bundled About animation should provide a frame");
            (frame.id, frame.width, frame.height, frame.pixels.clone())
        };
        if self.displayed_frame != frame_id {
            let image = egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels);
            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.texture = Some(context.load_texture(
                    "cubacadabra-player-about-video",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            self.displayed_frame = frame_id;
        }
        self.texture
            .as_ref()
            .expect("About video texture should be initialized")
    }
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
            about_open: false,
            about_video: None,
            about_video_error: None,
        }
    }

    pub(crate) fn open_about(&mut self) {
        self.about_open = true;
        if self.about_video.is_none() && self.about_video_error.is_none() {
            match AboutVideo::decode() {
                Ok(video) => self.about_video = Some(video),
                Err(error) => self.about_video_error = Some(error),
            }
        }
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
