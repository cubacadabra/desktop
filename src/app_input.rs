use super::DesktopApp;
use winit::{
    event::{ElementState, KeyEvent, MouseButton},
    keyboard::{KeyCode, PhysicalKey},
};

impl DesktopApp {
    pub(super) fn handle_key(&mut self, event: KeyEvent) {
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

    pub(super) fn handle_mouse(&mut self, state: ElementState, button: MouseButton) {
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
