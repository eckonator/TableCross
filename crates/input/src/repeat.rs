//! Auto-repeat for held keys on the receiving side. Systems repeat only keys of a real keyboard,
//! not injected ones, and the sender forwards each press once (with the helper it sees no repeats
//! at all). So the receiver repeats a held key itself, like a keyboard would: after the delay and
//! at the rate set on this computer, the newest held key only.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use tc_core::{Point, is_modifier};
use tc_protocol::{Message, Screen};

use crate::{Inject, InputError};

/// Caps, Num and Scroll Lock switch something on and off, Pause neither; they never repeat.
const NO_REPEAT: [u16; 4] = [0x39, 0x47, 0x48, 0x53];

/// The key being repeated and when it repeats next.
#[derive(Default)]
struct Held {
    key: Option<(u16, Instant)>,
    interval: Duration,
    stop: bool,
}

/// Wraps the injection of a system that does not repeat injected keys.
pub(crate) struct Repeating {
    inner: Arc<dyn Inject>,
    held: Arc<(Mutex<Held>, Condvar)>,
}

impl Repeating {
    pub(crate) fn wrap(inner: Arc<dyn Inject>) -> Arc<dyn Inject> {
        let held = Arc::new((Mutex::new(Held::default()), Condvar::new()));
        let (thread_inner, thread_held) = (inner.clone(), held.clone());
        let started = std::thread::Builder::new()
            .name("key-repeat".into())
            .spawn(move || repeat(&*thread_inner, &thread_held));
        if let Err(e) = started {
            tracing::warn!("no key repeat: {e}");
            return inner;
        }
        Arc::new(Self { inner, held })
    }
}

impl Drop for Repeating {
    fn drop(&mut self) {
        self.held.0.lock().expect("repeat lock").stop = true;
        self.held.1.notify_all();
    }
}

fn repeat(inner: &dyn Inject, held: &(Mutex<Held>, Condvar)) {
    let mut state = held.0.lock().expect("repeat lock");
    loop {
        if state.stop {
            return;
        }
        let Some((usage, at)) = state.key else {
            state = held.1.wait(state).expect("repeat lock");
            continue;
        };
        let now = Instant::now();
        if now < at {
            state = held.1.wait_timeout(state, at - now).expect("repeat lock").0;
            continue;
        }
        // Under the lock, so a release cannot slip in between and turn this into a new press
        if let Err(e) = inner.inject(&Message::Key { usage, down: true }) {
            tracing::warn!("key repeat: {e}");
        }
        let next = at + state.interval;
        state.key = Some((usage, next.max(now)));
    }
}

impl Inject for Repeating {
    fn grab(&self, park: Point) -> Result<(), InputError> {
        self.inner.grab(park)
    }

    fn ungrab(&self, at: Point) -> Result<(), InputError> {
        self.inner.ungrab(at)
    }

    fn inject(&self, message: &Message) -> Result<(), InputError> {
        let mut state = self.held.0.lock().expect("repeat lock");
        match *message {
            Message::Key { usage, down: true } if !is_modifier(usage) && !NO_REPEAT.contains(&usage) => {
                state.key = self.inner.key_repeat().map(|(delay, interval)| {
                    state.interval = interval;
                    (usage, Instant::now() + delay)
                });
                self.held.1.notify_all();
            }
            Message::Key { usage, down: false } if state.key.is_some_and(|(held, _)| held == usage) => {
                state.key = None;
            }
            Message::ReleaseAll => state.key = None,
            _ => {}
        }
        self.inner.inject(message)
    }

    fn release_all(&self) {
        let mut state = self.held.0.lock().expect("repeat lock");
        state.key = None;
        self.inner.release_all();
    }

    fn screens(&self) -> Vec<Screen> {
        self.inner.screens()
    }

    fn sizes(&self) -> Vec<(u32, u32)> {
        self.inner.sizes()
    }

    fn key_repeat(&self) -> Option<(Duration, Duration)> {
        self.inner.key_repeat()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_backend;

    #[test]
    fn a_held_key_repeats_until_released() {
        let (_capture, inject, device) = virtual_backend(Vec::new());
        inject.set_key_repeat(Some((Duration::from_millis(60), Duration::from_millis(20))));
        let repeating = Repeating::wrap(inject.clone());
        let a = Message::Key {
            usage: 0x04,
            down: true,
        };
        repeating.inject(&a).unwrap();
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(
            device.injected().iter().filter(|m| **m == a).count(),
            1,
            "not before the delay"
        );
        std::thread::sleep(Duration::from_millis(100));
        repeating
            .inject(&Message::Key {
                usage: 0x04,
                down: false,
            })
            .unwrap();
        let downs = device.injected().iter().filter(|m| **m == a).count();
        assert!((3..=7).contains(&downs), "{downs} presses");
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            device.injected().iter().filter(|m| **m == a).count(),
            downs,
            "stops on release"
        );
    }

    #[test]
    fn modifiers_and_released_everything_do_not_repeat() {
        let (_capture, inject, device) = virtual_backend(Vec::new());
        inject.set_key_repeat(Some((Duration::from_millis(10), Duration::from_millis(10))));
        let repeating = Repeating::wrap(inject.clone());
        let shift = Message::Key {
            usage: 0xE1,
            down: true,
        };
        repeating.inject(&shift).unwrap();
        repeating
            .inject(&Message::Key {
                usage: 0x04,
                down: true,
            })
            .unwrap();
        repeating.release_all();
        std::thread::sleep(Duration::from_millis(60));
        let log = device.injected();
        assert_eq!(log.iter().filter(|m| **m == shift).count(), 1);
        assert_eq!(log.len(), 2, "{log:?}");
    }
}
