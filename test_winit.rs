use winit::event_loop::{EventLoop, ActiveEventLoop, ControlFlow};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::window::WindowId;

struct App;
impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {}
    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, ev: WindowEvent) {}
}
fn main() {}
