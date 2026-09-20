use super::DesktopApp;
use log::error;
use winit::{
    application::ApplicationHandler,
    event::{MouseScrollDelta, WindowEvent},
    event_loop::ActiveEventLoop,
};

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
        if !self.in_game
            && let (Some(menu), Some(window)) = (&mut self.menu, &self.window)
        {
            menu.on_window_event(window, &event);
        }
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
            WindowEvent::KeyboardInput { event, .. } if self.in_game => self.handle_key(event),
            WindowEvent::CursorMoved { position, .. } if self.in_game => {
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
            WindowEvent::MouseInput { state, button, .. } if self.in_game => {
                self.handle_mouse(state, button)
            }
            WindowEvent::MouseWheel { delta, .. } if self.in_game => {
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
