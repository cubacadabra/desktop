use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor, wgpu};
use egui_winit::State as EguiState;
use std::mem;
use winit::{event::WindowEvent, window::Window};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WebRequest {
    BrowseGames,
    Account,
    About,
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
    start_requested: bool,
    sign_in_requested: bool,
    web_request: Option<WebRequest>,
    auth_pending: bool,
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
            start_requested: false,
            sign_in_requested: false,
            web_request: None,
            auth_pending: false,
        }
    }

    pub(crate) fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    pub(crate) fn prepare(&mut self, window: &Window, game_name: &str) -> PreparedMenu {
        let input = self.state.take_egui_input(window);
        let context = self.context.clone();
        let output = context.run_ui(input, |context| {
            egui::CentralPanel::default().show(context, |ui| self.show(ui, game_name));
        });
        self.state
            .handle_platform_output(window, output.platform_output);
        let pixels_per_point = self.context.pixels_per_point();
        let paint_jobs = self
            .context
            .tessellate(output.shapes, pixels_per_point);
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
            self.renderer.update_texture(device, queue, *id, image_delta);
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

    pub(crate) fn take_start_requested(&mut self) -> bool {
        mem::take(&mut self.start_requested)
    }

    pub(crate) fn take_sign_in_requested(&mut self) -> bool {
        mem::take(&mut self.sign_in_requested)
    }

    pub(crate) fn take_web_request(&mut self) -> Option<WebRequest> {
        self.web_request.take()
    }

    pub(crate) fn set_auth_pending(&mut self, pending: bool) {
        self.auth_pending = pending;
    }

    pub(crate) fn set_authenticated(&mut self, user: &crate::network::AuthUser) {
        self.signed_in_name = Some(
            user.username
                .clone()
                .unwrap_or_else(|| user.name.clone()),
        );
    }

    fn show(&mut self, ui: &mut egui::Ui, game_name: &str) {
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
                    ui.add_space(46.0);
                    ui.heading(egui::RichText::new("Your cubes").size(38.0));
                    ui.label("Choose a game to enter its lobby.");
                    ui.add_space(30.0);
                    section_label(ui, "CUBES");
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        menu_row(ui, "01", game_name, "Continue playing this cube", true, || {
                            self.start_requested = true;
                        });
                        ui.separator();
                        menu_row(
                            ui,
                            "+",
                            "Browse games on cubacadabra.com",
                            "Discover more Cubes on the web",
                            true,
                            || self.web_request = Some(WebRequest::BrowseGames),
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
                                if self.auth_pending { "Signing in…" } else { "Sign in" },
                                "Manage your account",
                                !self.auth_pending,
                                || {
                                    self.sign_in_requested = true;
                                },
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
                });
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
