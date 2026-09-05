//! A backend without operating system: tests feed events and read what was injected.

use std::sync::{Arc, Mutex};

use tc_core::{Captured, Point};
use tc_protocol::{Message, Screen};

use crate::{Capture, Handler, Health, Inject, InputError};

#[derive(Default)]
struct State {
    handler: Option<Handler>,
    grabbed: bool,
    pointer: Point,
    log: Vec<Message>,
}

/// Feeds events as if they came from the local devices.
#[derive(Clone)]
pub struct VirtualDevice {
    state: Arc<Mutex<State>>,
}

pub struct VirtualCapture {
    state: Arc<Mutex<State>>,
}

pub struct VirtualInject {
    state: Arc<Mutex<State>>,
    screens: Vec<Screen>,
}

pub fn virtual_backend(screens: Vec<Screen>) -> (VirtualCapture, Arc<VirtualInject>, VirtualDevice) {
    let state = Arc::new(Mutex::new(State::default()));
    (
        VirtualCapture { state: state.clone() },
        Arc::new(VirtualInject {
            state: state.clone(),
            screens,
        }),
        VirtualDevice { state },
    )
}

impl VirtualDevice {
    /// Returns whether the local system would have received the event.
    pub fn send(&self, event: Captured) -> bool {
        // The handler may call back into the injector, so it runs without holding the lock
        let handler = self.state.lock().expect("virtual input").handler.take();
        let Some(mut handler) = handler else {
            return true;
        };
        let pass = handler(event);
        self.state.lock().expect("virtual input").handler = Some(handler);
        pass
    }

    pub fn injected(&self) -> Vec<Message> {
        self.state.lock().expect("virtual input").log.clone()
    }

    pub fn grabbed(&self) -> bool {
        self.state.lock().expect("virtual input").grabbed
    }

    pub fn pointer(&self) -> Point {
        self.state.lock().expect("virtual input").pointer
    }
}

impl Capture for VirtualCapture {
    fn start(&mut self, handler: Handler) -> Result<(), InputError> {
        self.state.lock().expect("virtual input").handler = Some(handler);
        Ok(())
    }

    fn stop(&mut self) {
        self.state.lock().expect("virtual input").handler = None;
    }

    fn health(&self) -> Health {
        Health {
            capturing: true,
            problems: Vec::new(),
        }
    }
}

impl Inject for VirtualInject {
    fn grab(&self, park: Point) -> Result<(), InputError> {
        let mut state = self.state.lock().expect("virtual input");
        state.grabbed = true;
        state.pointer = park;
        Ok(())
    }

    fn ungrab(&self, at: Point) -> Result<(), InputError> {
        let mut state = self.state.lock().expect("virtual input");
        state.grabbed = false;
        state.pointer = at;
        Ok(())
    }

    fn inject(&self, message: &Message) -> Result<(), InputError> {
        let mut state = self.state.lock().expect("virtual input");
        if let Message::Enter { x, y } | Message::MouseMove { x, y, .. } = message {
            state.pointer = Point::new(f64::from(*x), f64::from(*y));
        }
        state.log.push(message.clone());
        Ok(())
    }

    fn release_all(&self) {}

    fn screens(&self) -> Vec<Screen> {
        self.screens.clone()
    }
}
