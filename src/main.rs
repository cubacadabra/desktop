mod app;
mod assets;
mod bundled;
#[cfg(target_os = "macos")]
mod macos;
mod menu;
mod network;
mod options;

use app::DesktopApp;
use log::debug;
use options::DesktopError;
use std::{error::Error, fs, path::Path};
use winit::event_loop::{ControlFlow, EventLoop};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SharedUiAction {
    LeaveGame,
    SignIn,
}

fn shared_ui_action(source: &[u8]) -> Option<SharedUiAction> {
    let event = serde_json::from_slice::<serde_json::Value>(source).ok()?;
    if event.get("phase").and_then(serde_json::Value::as_str) != Some("activate") {
        return None;
    }
    match event.get("action").and_then(serde_json::Value::as_str) {
        Some("shared.leave_game") => Some(SharedUiAction::LeaveGame),
        Some("shared.sign_in") => Some(SharedUiAction::SignIn),
        _ => None,
    }
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
    let mut app = match options.package {
        Some(package) => DesktopApp::load(package)?,
        None => DesktopApp::load_bundled()?,
    };
    let event_loop = EventLoop::builder().build()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SharedUiAction, shared_ui_action};

    #[test]
    fn routes_shared_account_actions_on_activation() {
        assert_eq!(
            shared_ui_action(br#"{"action":"shared.sign_in","phase":"activate"}"#),
            Some(SharedUiAction::SignIn)
        );
        assert_eq!(
            shared_ui_action(br#"{"action":"shared.leave_game","phase":"activate"}"#),
            Some(SharedUiAction::LeaveGame)
        );
        assert_eq!(
            shared_ui_action(br#"{"action":"shared.sign_in","phase":"press"}"#),
            None
        );
    }
}
