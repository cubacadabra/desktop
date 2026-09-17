use crate::network::CatalogEntry;
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor, wgpu};
use egui_winit::State as EguiState;
use std::mem;
use winit::{event::WindowEvent, window::Window};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WebRequest {
    Account,
    About,
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

    fn show(&mut self, ui: &mut egui::Ui, current_game_id: &str) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.set_max_width(720.0);
                    ui.add_space(18.0);
                    ui.horizontal(|ui| {
                        ui.heading("CUBACADABRA");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label("●");
                        });
                    });
                    match self.screen {
                        Screen::Home => self.show_home(ui, current_game_id),
                        Screen::Catalog => self.show_catalog(ui),
                    }
                    if let Some(error) = &self.game_error {
                        ui.add_space(12.0);
                        ui.colored_label(egui::Color32::from_rgb(190, 55, 45), error);
                    }
                });
            });
    }

    fn show_home(&mut self, ui: &mut egui::Ui, current_game_id: &str) {
        ui.add_space(46.0);
        ui.heading(egui::RichText::new("Your cubes").size(38.0));
        ui.label("Choose a game to enter its lobby.");
        ui.add_space(30.0);
        section_label(ui, "CUBES");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            let games = [
                ("first-game", "First Game", "Build together in the clearing"),
                (
                    "second-game",
                    "Second Game",
                    "Drop signals in the relay yard",
                ),
            ];
            for (index, (id, title, subtitle)) in games.into_iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                let detail = if id == current_game_id {
                    "Resume this game"
                } else {
                    subtitle
                };
                menu_row(
                    ui,
                    if id == current_game_id { "▶" } else { "◇" },
                    title,
                    detail,
                    self.loading_game_id.is_none(),
                    || self.game_request = Some(GameRequest::Bundled(id.to_owned())),
                );
            }
            ui.separator();
            menu_row(
                ui,
                "＋",
                "List cubes",
                if self.catalog.loading {
                    "Loading uploaded cubes…"
                } else {
                    "Browse uploaded cubes"
                },
                self.loading_game_id.is_none(),
                || self.catalog_requested = true,
            );
        });
        ui.add_space(28.0);
        section_label(ui, "ACCOUNT");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            if let Some(name) = self.signed_in_name.clone() {
                menu_row(
                    ui,
                    "@",
                    &format!("Signed in as {name}"),
                    "Account & avatar on cubacadabra.com",
                    true,
                    || self.web_request = Some(WebRequest::Account),
                );
            } else {
                menu_row(
                    ui,
                    "@",
                    if self.auth_pending {
                        "Signing in…"
                    } else {
                        "Sign in"
                    },
                    "Manage your account",
                    !self.auth_pending,
                    || self.sign_in_requested = true,
                );
            }
        });
        ui.add_space(28.0);
        section_label(ui, "ABOUT");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            menu_row(
                ui,
                "i",
                "About cubacadabra",
                "Learn more on the web",
                true,
                || self.web_request = Some(WebRequest::About),
            );
        });
    }

    fn show_catalog(&mut self, ui: &mut egui::Ui) {
        ui.add_space(28.0);
        ui.horizontal(|ui| {
            if ui.button("‹  Back").clicked() {
                self.back_to_home();
            }
            ui.heading(egui::RichText::new("List cubes").size(30.0));
        });
        ui.add_space(8.0);
        ui.label("Choose an uploaded cube to download and enter.");
        ui.add_space(24.0);
        section_label(ui, "UPLOADED CUBES");
        if self.catalog.loading {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label("Loading cubes…");
            });
            return;
        }
        if let Some(error) = self.catalog.error.clone() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.colored_label(egui::Color32::from_rgb(190, 55, 45), &error);
                if ui.button("Try again").clicked() {
                    self.catalog_requested = true;
                    self.catalog.loading = true;
                    self.catalog.error = None;
                }
            });
            return;
        }
        if self.catalog.entries.is_empty() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label("No uploaded cubes yet.");
            });
            return;
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            let entries = self.catalog.entries.clone();
            for (index, entry) in entries.into_iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                menu_row(
                    ui,
                    "◇",
                    &entry.display_name,
                    &format!("{} · v{}", entry.id, entry.version),
                    self.loading_game_id.is_none(),
                    || self.game_request = Some(GameRequest::Remote(entry.clone())),
                );
            }
        });
    }
}

fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.label(egui::RichText::new(label).small().strong());
    ui.add_space(6.0);
}

fn menu_row(
    ui: &mut egui::Ui,
    symbol: &str,
    title: &str,
    subtitle: &str,
    enabled: bool,
    mut on_click: impl FnMut(),
) {
    let response = ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(symbol).size(20.0).strong());
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(title).strong());
                ui.label(egui::RichText::new(subtitle).small());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("›");
            });
        })
        .response
        .interact(egui::Sense::click())
    });
    if response.inner.clicked() {
        on_click();
    }
}
