use cubacadabra_app::{
    AppAction, AppModel, AppSnapshot, UsernameValidationError, validate_account_username,
};
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor, wgpu};
use egui_winit::State as EguiState;
use std::mem;
use winit::{event::WindowEvent, window::Window};

pub(crate) struct PreparedMenu {
    paint_jobs: Vec<egui::ClippedPrimitive>,
    screen: ScreenDescriptor,
}

pub(crate) struct PlayerMenu {
    context: egui::Context,
    state: EguiState,
    renderer: Renderer,
    pending_textures_delta: egui::TexturesDelta,
    app: AppModel,
    username: String,
    username_status: Option<String>,
    editing_username: bool,
    start_requested: bool,
    sign_in_requested: bool,
    auth_pending: bool,
    username_changed: Option<String>,
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
        let username = std::env::var("CUBACADABRA_USERNAME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Player".to_owned());
        let mut app = AppModel::default();
        app.dispatch(AppAction::ReplaceSession {
            account_id: std::env::var("CUBACADABRA_ACCOUNT_ID")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            username: Some(username.clone()),
            body_id: None,
            date_of_birth: None,
        });
        Self {
            context,
            state,
            renderer: Renderer::new(
                game_renderer.device(),
                game_renderer.studio_overlay_format(),
                RendererOptions::default(),
            ),
            pending_textures_delta: egui::TexturesDelta::default(),
            app,
            username,
            username_status: None,
            editing_username: false,
            start_requested: false,
            sign_in_requested: false,
            auth_pending: false,
            username_changed: None,
        }
    }

    pub(crate) fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    pub(crate) fn prepare(&mut self, window: &Window, game_name: &str) -> PreparedMenu {
        let input = self.state.take_egui_input(window);
        let context = self.context.clone();
        let output = context.run_ui(input, |context| {
            egui::CentralPanel::default()
                .show(context, |ui| self.show(ui, game_name));
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

    pub(crate) fn take_username_changed(&mut self) -> Option<String> {
        self.username_changed.take()
    }

    pub(crate) fn take_sign_in_requested(&mut self) -> bool {
        std::mem::take(&mut self.sign_in_requested)
    }

    pub(crate) fn set_auth_pending(&mut self, pending: bool) {
        self.auth_pending = pending;
    }

    pub(crate) fn set_authenticated(&mut self, user: &crate::network::AuthUser) {
        let username = user.username.clone().unwrap_or_else(|| user.name.clone());
        self.username = username.clone();
        self.app.dispatch(AppAction::ReplaceSession {
            account_id: Some(user.id.clone()),
            username: Some(username),
            body_id: None,
            date_of_birth: None,
        });
    }

    fn show(&mut self, ui: &mut egui::Ui, game_name: &str) {
        let snapshot = self.app.snapshot();
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
                    });
                    ui.add_space(28.0);
                    section_label(ui, "ACCOUNT");
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        if snapshot.account_id.is_some() {
                            let label = snapshot
                                .profile
                                .username
                                .as_deref()
                                .unwrap_or(&self.username);
                            menu_row(ui, "@", "Change your username", label, true, || {
                                self.editing_username = true;
                                self.username_status = None;
                                self.app.dispatch(AppAction::BeginUsernameEdit {});
                            });
                            ui.separator();
                            menu_row(
                                ui,
                                "♙",
                                "Choose your morph",
                                "Customize your character",
                                false,
                                || {},
                            );
                            ui.separator();
                            menu_row(
                                ui,
                                "!",
                                "Block or unblock players",
                                "Players & safety",
                                false,
                                || {},
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
                    if self.editing_username {
                        self.username_editor(ui, &snapshot);
                    }
                    ui.add_space(28.0);
                    section_label(ui, "ABOUT");
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        menu_row(ui, "i", "About cubacadabra", "Learn more", false, || {});
                    });
                });
            });
    }

    fn username_editor(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        ui.add_space(10.0);
        ui.label("Choose a name other players can find you by.");
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.username)
                .hint_text("Your username")
                .desired_width(300.0),
        );
        if response.changed() {
            self.app.dispatch(AppAction::UsernameChanged {
                value: self.username.clone(),
            });
            self.username_status = None;
        }
        ui.label("Use 2–24 letters, numbers, _ or -.");
        if let Some(status) = &self.username_status {
            ui.colored_label(egui::Color32::from_rgb(190, 70, 60), status);
        } else if let Some(feedback) = &snapshot.profile.username_feedback {
            ui.label(&feedback.message);
        }
        ui.horizontal(|ui| {
            if ui.button("Save username").clicked() {
                match validate_account_username(&self.username) {
                    Ok(username) => {
                        self.username = username;
                        self.username_changed = Some(self.username.clone());
                        self.username_status =
                            Some("Username updated for this session.".to_owned());
                        self.editing_username = false;
                    }
                    Err(error) => self.username_status = Some(username_error(error)),
                }
            }
            if ui.button("Cancel").clicked() {
                self.editing_username = false;
                self.username_status = None;
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

fn username_error(error: UsernameValidationError) -> String {
    error.message().to_owned()
}
