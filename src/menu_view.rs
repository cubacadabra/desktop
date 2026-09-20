use super::{GameRequest, PlayerMenu, WebRequest};

impl PlayerMenu {
    pub(super) fn show(&mut self, ui: &mut egui::Ui, current_game_id: &str) {
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
                        super::Screen::Home => self.show_home(ui, current_game_id),
                        super::Screen::Catalog => self.show_catalog(ui),
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
