use winit::event_loop::{EventLoop, ActiveEventLoop};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::window::{WindowId, Window};

struct App {
    window: Option<Window>,
}
impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let attrs = Window::default_attributes().with_visible(false);
        self.window = Some(el.create_window(attrs).unwrap());
    }
    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, ev: WindowEvent) {}
}
fn main() {}
