use super::{DesktopApp, PackageContent};
use crate::{assets, bundled, network, options::DesktopError};
use cubacadabra_client::ClientSession;
use log::info;
use std::error::Error;
use std::time::Instant;

impl DesktopApp {
    pub(super) fn install_bundled_game(&mut self, game_id: &str) -> Result<(), Box<dyn Error>> {
        let package = bundled::PACKAGES
            .iter()
            .find(|package| package.id == game_id)
            .ok_or_else(|| DesktopError(format!("the bundled game {game_id} was not found")))?;
        self.install_package(PackageContent {
            id: package.id.to_owned(),
            manifest: package.manifest.to_owned(),
            script: package.script.to_owned(),
            atlas: assets::load_bundled(package.files, package.manifest)?,
            models: assets::load_bundled_models(package.files, package.manifest)?,
        })
    }

    pub(super) fn install_remote_package(
        &mut self,
        package: network::RemoteGamePackage,
    ) -> Result<(), Box<dyn Error>> {
        let atlas = assets::load_from_files(&package.files, &package.manifest)?;
        let models = assets::load_models_from_files(&package.files, &package.manifest)?;
        self.install_package(PackageContent {
            id: package.id,
            manifest: package.manifest,
            script: package.script,
            atlas,
            models,
        })
    }

    pub(super) fn install_package(
        &mut self,
        package: PackageContent,
    ) -> Result<(), Box<dyn Error>> {
        let auth_session = self.network.auth_session();
        self.client.transport_disconnected();
        self.network.disconnect();

        let mut client = ClientSession::load(&package.manifest, &package.script)?;
        info!("installing game package: game_id={}", client.game_id());
        let _ = client.engine_mut().set_username_value(&self.username);
        if auth_session.is_some() {
            client.engine_mut().set_authenticated_value(true);
        }
        let network = network::BackendClient::new(client.game_id()).map_err(DesktopError)?;
        if let Some(session) = auth_session {
            network.set_auth_session(session);
        }

        self.client = client;
        self.network = network;
        self.package_name = package.id;
        self.atlas = package.atlas;
        self.models = package.models;
        if let Some(window) = &self.window {
            window.set_title(&format!("Cubacadabra — {}", self.package_name));
        }
        self.update_viewport();
        if let Some(renderer) = &mut self.renderer {
            renderer.clear_world_meshes();
            for model in &self.models {
                renderer
                    .register_world_mesh(&model.id, &model.bytes)
                    .map_err(|error| {
                        DesktopError(format!("world model {} was rejected: {error}", model.id))
                    })?;
            }
            if let Some(atlas) = &self.atlas {
                if !renderer.set_package_image_atlas(
                    atlas.width,
                    atlas.height,
                    &atlas.pixels,
                    atlas.regions.clone(),
                ) {
                    return Err(Box::new(DesktopError(
                        "the game's image atlas could not be uploaded".into(),
                    )));
                }
            } else if !renderer.set_package_image_atlas(
                1,
                1,
                &[0, 0, 0, 0],
                std::collections::BTreeMap::new(),
            ) {
                return Err(Box::new(DesktopError(
                    "the game's image atlas could not be cleared".into(),
                )));
            }
        }
        self.last_frame = Instant::now();
        self.clear_input();
        self.in_game = true;
        self.client.request_transport();
        self.dispatch_actions();
        if let Some(menu) = &mut self.menu {
            menu.back_to_home();
            menu.set_game_loading(None);
        }
        Ok(())
    }
}
