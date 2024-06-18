//! The Varvara computer system
#![warn(missing_docs)]
use log::warn;
use std::{
    collections::VecDeque,
    io::Write,
    sync::{Arc, Mutex},
};

mod console;
mod controller;
mod datetime;
mod file;
mod mouse;
mod screen;
mod system;

/// Audio handler implementation
mod audio;

pub use audio::CHANNELS as AUDIO_CHANNELS;
pub use audio::SAMPLE_RATE as AUDIO_SAMPLE_RATE;
pub use console::worker as console_worker;
pub use controller::Key;
pub use mouse::MouseState;

use uxn::{Device, Ports, Uxn};

/// Internal events, accumulated by devices then applied to the CPU
struct Event {
    /// Tuple of `(address, value)` to write in in device memory
    pub data: Option<(u8, u8)>,

    /// Vector to trigger
    pub vector: u16,
}

/// Input to [`Varvara::update`], including all incoming events
#[derive(Default)]
pub struct Input {
    /// Current mouse state
    pub mouse: mouse::MouseState,

    /// Keys pressed
    pub pressed: Vec<controller::Key>,

    /// Keys released
    pub released: Vec<controller::Key>,

    /// Incoming console character
    pub console: Option<u8>,
}

/// Output from [`Varvara::update`], which may modify the GUI
pub struct Output<'a> {
    /// Current window size
    pub size: (u16, u16),

    /// Current screen contents, as RGBA values
    pub frame: &'a [u32],

    /// The system's mouse cursor should be hidden
    pub hide_mouse: bool,

    /// Outgoing console characters sent to the `write` port
    pub stdout: Vec<u8>,

    /// Outgoing console characters sent to the `error` port
    pub stderr: Vec<u8>,

    /// Request to exit with the given error code
    pub exit: Option<i32>,
}

impl Output<'_> {
    /// Prints `stdout` and `stderr` to the console
    pub fn print(&self) -> std::io::Result<()> {
        if !self.stdout.is_empty() {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(&self.stdout)?;
            stdout.flush()?;
        }
        if !self.stderr.is_empty() {
            let mut stderr = std::io::stderr().lock();
            stderr.write_all(&self.stderr)?;
            stderr.flush()?;
        }
        Ok(())
    }
}

/// Handle to the Varvara system
pub struct Varvara {
    system: system::System,
    console: console::Console,
    datetime: datetime::Datetime,
    audio: audio::Audio,
    screen: screen::Screen,
    mouse: mouse::Mouse,
    controller: controller::Controller,

    /// Flags indicating if we've already printed a warning about a missing dev
    already_warned: [bool; 16],

    queue: VecDeque<Event>,
}

impl Default for Varvara {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for Varvara {
    fn deo(&mut self, vm: &mut Uxn, target: u8) -> bool {
        match target & 0xF0 {
            system::SystemPorts::BASE => self.system.deo(vm, target),
            console::ConsolePorts::BASE => self.console.deo(vm, target),
            datetime::DatetimePorts::BASE => self.datetime.deo(vm, target),
            screen::ScreenPorts::BASE => self.screen.deo(vm, target),
            mouse::MousePorts::BASE => self.mouse.set_active(),
            controller::ControllerPorts::BASE => (),
            a if audio::AudioPorts::matches(a) => self.audio.deo(vm, target),

            // Default case
            t => self.warn_missing(t),
        }
        !self.system.should_exit()
    }
    fn dei(&mut self, vm: &mut Uxn, target: u8) {
        match target & 0xF0 {
            system::SystemPorts::BASE => self.system.dei(vm, target),
            console::ConsolePorts::BASE => self.console.dei(vm, target),
            datetime::DatetimePorts::BASE => self.datetime.dei(vm, target),
            screen::ScreenPorts::BASE => self.screen.dei(vm, target),
            mouse::MousePorts::BASE => self.mouse.set_active(),
            controller::ControllerPorts::BASE => (),
            a if audio::AudioPorts::matches(a) => self.audio.dei(vm, target),

            // Default case
            t => self.warn_missing(t),
        }
    }
}

impl Varvara {
    /// Builds a new instance of the Varvara peripherals
    pub fn new() -> Self {
        const WIDTH: u16 = 512;
        const HEIGHT: u16 = 320;
        Self {
            console: console::Console::new(),
            system: system::System::default(),
            datetime: datetime::Datetime,
            audio: audio::Audio::new(),
            screen: screen::Screen::new(WIDTH, HEIGHT),
            mouse: mouse::Mouse::new(),
            controller: controller::Controller::new(),

            queue: VecDeque::with_capacity(1),
            already_warned: [false; 16],
        }
    }

    /// Returns the current screen size
    pub fn screen_size(&self) -> (u16, u16) {
        self.screen.size()
    }

    /// Checks whether the SHIFT key is currently down
    pub fn shift_held(&self) -> bool {
        self.controller.shift_held()
    }

    fn warn_missing(&mut self, t: u8) {
        if !self.already_warned[(t >> 4) as usize] {
            warn!("unimplemented device {t:#02x}");
            self.already_warned[(t >> 4) as usize] = true;
        }
    }

    /// Calls the screen vector
    ///
    /// This function must be called at 60 Hz
    pub fn redraw(&mut self, vm: &mut Uxn) {
        let v = self.screen.update(vm);
        vm.run(self, v)
    }

    /// Handles incoming events
    pub fn update(&mut self, vm: &mut Uxn, e: Input) -> Output {
        if let Some(c) = e.console {
            self.console.update(vm, c, &mut self.queue);
        }
        self.audio.update(vm, &mut self.queue);
        self.mouse.update(vm, e.mouse, &mut self.queue);
        for k in &e.pressed {
            self.controller.pressed(vm, *k, &mut self.queue);
        }
        for k in &e.released {
            self.controller.released(vm, *k, &mut self.queue);
        }

        self.process_events(vm);

        Output {
            size: self.screen.size(),
            frame: self.screen.frame(vm),
            hide_mouse: self.mouse.active(),
            stdout: self.console.stdout(),
            stderr: self.console.stderr(),
            exit: self.system.exit(),
        }
    }

    fn process_events(&mut self, vm: &mut Uxn) {
        while let Some(e) = self.queue.pop_front() {
            if let Some((addr, data)) = e.data {
                vm.write_dev_mem(addr, data);
            }
            vm.run(self, e.vector);
        }
    }

    /// Returns a handle to the given audio stream data
    ///
    /// # Panics
    /// There are only four audio streams, so this function panics if `i >= 4`
    pub fn audio_stream(&self, i: usize) -> Arc<Mutex<audio::StreamData>> {
        self.audio.stream(i)
    }
}
