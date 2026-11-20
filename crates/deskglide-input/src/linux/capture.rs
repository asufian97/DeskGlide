//! Linux `evdev` input capture (works identically on Wayland and X11).

use std::collections::HashSet;
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use deskglide_proto::{InputEvent, KeyState, MouseButton, ScreenEdge, WHEEL_DETENT};
use evdev::{Device, EventSummary, KeyCode, RelativeAxisCode, SynchronizationCode};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::hotkey::{Hotkey, HotkeyStateMachine};
use crate::{CaptureEvent, InputCapture};

const VIRTUAL_PREFIX: &str = "deskglide-virtual-";

pub struct LinuxCapture {
    rx: mpsc::Receiver<CaptureEvent>,
    remote_flag: Arc<AtomicBool>,
    desired_active: Arc<AtomicBool>,
    switch_edge: Arc<AtomicU8>,
    wake_write_fd: i32,
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LinuxCapture {
    pub fn start(hotkey: Hotkey, switch_edge: ScreenEdge) -> Result<Self> {
        let mut devices = discover_physical_devices()?;
        if devices.is_empty() {
            anyhow::bail!(
                "no readable keyboard/mouse devices found in /dev/input/event*; \
                 run scripts/linux/setup-uinput.sh and re-login (or check 'input' group membership)"
            );
        }

        for dev in &devices {
            dev.set_nonblocking(true)
                .with_context(|| format!("failed to set nonblocking on {:?}", dev.name()))?;
        }

        let mut pipe_fds = [0i32; 2];
        if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("failed to create wakeup pipe");
        }
        let wake_read_fd = pipe_fds[0];
        let wake_write_fd = pipe_fds[1];
        unsafe {
            libc::fcntl(wake_read_fd, libc::F_SETFL, libc::O_NONBLOCK);
            libc::fcntl(wake_write_fd, libc::F_SETFL, libc::O_NONBLOCK);
        }

        let (tx, rx) = mpsc::channel(4096);
        let remote_flag = Arc::new(AtomicBool::new(false));
        let desired_active = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::new(AtomicBool::new(false));

        let remote_thread = Arc::clone(&remote_flag);
        let desired_thread = Arc::clone(&desired_active);
        let stop_thread = Arc::clone(&stop_flag);
        let (ready_tx, ready_rx) = std_mpsc::sync_channel::<Result<()>>(1);

        let thread = thread::Builder::new()
            .name("deskglide-evdev-capture".into())
            .spawn(move || {
                let _ = ready_tx.send(Ok(()));
                run_evdev_loop(
                    &mut devices,
                    wake_read_fd,
                    hotkey,
                    tx,
                    remote_thread,
                    desired_thread,
                    stop_thread,
                );
                unsafe {
                    libc::close(wake_read_fd);
                }
            })
            .context("failed to spawn Linux evdev capture thread")?;

        ready_rx.recv()??;

        Ok(Self {
            rx,
            remote_flag,
            desired_active,
            switch_edge: Arc::new(AtomicU8::new(switch_edge as u8)),
            wake_write_fd,
            stop_flag,
            thread: Some(thread),
        })
    }

    fn wake(&self) {
        let b = [1u8];
        unsafe {
            libc::write(self.wake_write_fd, b.as_ptr().cast(), 1);
        }
    }
}

impl InputCapture for LinuxCapture {
    fn set_remote_active(&self, active: bool) -> Result<()> {
        self.desired_active.store(active, Ordering::SeqCst);
        self.wake();
        Ok(())
    }

    fn set_switch_edge(&self, edge: ScreenEdge) {
        self.switch_edge.store(edge as u8, Ordering::SeqCst);
    }

    fn restore_from_edge(&self, _edge: ScreenEdge) -> Result<()> {
        self.set_remote_active(false)
    }

    fn is_remote_active(&self) -> bool {
        self.remote_flag.load(Ordering::SeqCst)
    }

    fn events_rx(&mut self) -> &mut mpsc::Receiver<CaptureEvent> {
        &mut self.rx
    }
}

impl Drop for LinuxCapture {
    fn drop(&mut self) {
        self.desired_active.store(false, Ordering::SeqCst);
        self.stop_flag.store(true, Ordering::SeqCst);
        self.wake();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        unsafe {
            libc::close(self.wake_write_fd);
        }
    }
}

fn discover_physical_devices() -> Result<Vec<Device>> {
    let mut out = Vec::new();
    for (path, dev) in evdev::enumerate() {
        let name = dev.name().unwrap_or("unnamed");
        if name.starts_with(VIRTUAL_PREFIX) {
            continue;
        }
        let is_keyboard = dev.supported_keys().is_some_and(|keys| {
            keys.contains(KeyCode::KEY_A) || keys.contains(KeyCode::KEY_ENTER)
        });
        let is_mouse = dev.supported_relative_axes().is_some_and(|axes| {
            axes.contains(RelativeAxisCode::REL_X) && axes.contains(RelativeAxisCode::REL_Y)
        });
        if is_keyboard || is_mouse {
            info!(?path, name, is_keyboard, is_mouse, "watching Linux input device");
            out.push(dev);
        }
    }
    Ok(out)
}

struct GrabGuard {
    grabbed: bool,
}

impl GrabGuard {
    fn new() -> Self {
        Self { grabbed: false }
    }

    fn set_grab(&mut self, devices: &mut [Device], grab: bool) {
        if self.grabbed == grab {
            return;
        }
        for dev in devices {
            let res = if grab { dev.grab() } else { dev.ungrab() };
            if let Err(e) = res {
                warn!(name = ?dev.name(), grab, error = %e, "evdev grab state change failed");
            }
        }
        self.grabbed = grab;
        debug!(grab, "updated Linux evdev exclusive grab");
    }
}

fn run_evdev_loop(
    devices: &mut [Device],
    wake_read_fd: i32,
    hotkey: Hotkey,
    tx: mpsc::Sender<CaptureEvent>,
    remote_flag: Arc<AtomicBool>,
    desired_active: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
) {
    let mut hotkey_sm = HotkeyStateMachine::new(hotkey);
    let mut grab_guard = GrabGuard::new();
    let mut down_keys: HashSet<u16> = HashSet::new();

    let mut pollfds: Vec<libc::pollfd> = Vec::with_capacity(devices.len() + 1);
    pollfds.push(libc::pollfd {
        fd: wake_read_fd,
        events: libc::POLLIN,
        revents: 0,
    });
    for dev in devices.iter() {
        pollfds.push(libc::pollfd {
            fd: dev.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
    }

    while !stop_flag.load(Ordering::SeqCst) {
        let rc = unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, -1) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::Interrupted {
                continue;
            }
            warn!(error = %err, "poll() on evdev fds failed");
            break;
        }

        if (pollfds[0].revents & libc::POLLIN) != 0 {
            let mut drain = [0u8; 64];
            unsafe {
                while libc::read(wake_read_fd, drain.as_mut_ptr().cast(), drain.len()) > 0 {}
            }
            let want = desired_active.load(Ordering::SeqCst);
            if want != hotkey_sm.is_remote_active() {
                hotkey_sm.set_remote_active(want);
                remote_flag.store(want, Ordering::SeqCst);
                grab_guard.set_grab(devices, want);
                if !want {
                    down_keys.clear();
                }
            }
        }

        let mut pending_grab_change: Option<bool> = None;

        for dev in devices.iter_mut() {
            let dev_name = dev.name().unwrap_or("unnamed").to_string();
            let has_hi_res_v = dev
                .supported_relative_axes()
                .is_some_and(|a| a.contains(RelativeAxisCode::REL_WHEEL_HI_RES));
            let has_hi_res_h = dev
                .supported_relative_axes()
                .is_some_and(|a| a.contains(RelativeAxisCode::REL_HWHEEL_HI_RES));

            let mut rel_x = 0i32;
            let mut rel_y = 0i32;

            loop {
                match dev.fetch_events() {
                    Ok(events) => {
                        for ev in events {
                            match ev.destructure() {
                                EventSummary::RelativeAxis(_, axis, val) => {
                                    if !hotkey_sm.is_remote_active() {
                                        continue;
                                    }
                                    match axis {
                                        RelativeAxisCode::REL_X => rel_x = rel_x.saturating_add(val),
                                        RelativeAxisCode::REL_Y => rel_y = rel_y.saturating_add(val),
                                        RelativeAxisCode::REL_WHEEL_HI_RES => {
                                            let _ = tx.try_send(CaptureEvent::Input(
                                                InputEvent::MouseScroll { dx: 0, dy: val },
                                            ));
                                        }
                                        RelativeAxisCode::REL_HWHEEL_HI_RES => {
                                            let _ = tx.try_send(CaptureEvent::Input(
                                                InputEvent::MouseScroll { dx: val, dy: 0 },
                                            ));
                                        }
                                        RelativeAxisCode::REL_WHEEL if !has_hi_res_v => {
                                            let _ = tx.try_send(CaptureEvent::Input(
                                                InputEvent::MouseScroll {
                                                    dx: 0,
                                                    dy: val.saturating_mul(WHEEL_DETENT),
                                                },
                                            ));
                                        }
                                        RelativeAxisCode::REL_HWHEEL if !has_hi_res_h => {
                                            let _ = tx.try_send(CaptureEvent::Input(
                                                InputEvent::MouseScroll {
                                                    dx: val.saturating_mul(WHEEL_DETENT),
                                                    dy: 0,
                                                },
                                            ));
                                        }
                                        _ => {}
                                    }
                                }
                                EventSummary::Synchronization(_, SynchronizationCode::SYN_REPORT, _) => {
                                    if (rel_x != 0 || rel_y != 0) && hotkey_sm.is_remote_active() {
                                        let _ = tx.try_send(CaptureEvent::Input(
                                            InputEvent::MouseMove {
                                                dx: rel_x,
                                                dy: rel_y,
                                            },
                                        ));
                                    }
                                    rel_x = 0;
                                    rel_y = 0;
                                }
                                EventSummary::Key(_, key, value) => {
                                    let code = key.code();
                                    if let Some(button) = MouseButton::from_evdev(code) {
                                        if hotkey_sm.is_remote_active() {
                                            let _ = tx.try_send(CaptureEvent::Input(
                                                InputEvent::MouseButton {
                                                    button,
                                                    pressed: value != 0,
                                                },
                                            ));
                                        }
                                        continue;
                                    }
                                    let Some(key_state) = KeyState::from_u8(value as u8) else {
                                        continue;
                                    };
                                    match key_state {
                                        KeyState::Press => {
                                            down_keys.insert(code);
                                        }
                                        KeyState::Release => {
                                            down_keys.remove(&code);
                                        }
                                        KeyState::Repeat => {}
                                    }
                                    let decision = hotkey_sm.on_key(code, key_state);
                                    if let Some(active) = decision.new_remote_active {
                                        desired_active.store(active, Ordering::SeqCst);
                                        remote_flag.store(active, Ordering::SeqCst);
                                        pending_grab_change = Some(active);
                                        let _ = tx.try_send(CaptureEvent::ActiveChanged(active));
                                        continue;
                                    }
                                    if decision.swallow && hotkey_sm.is_remote_active() {
                                        let _ = tx.try_send(CaptureEvent::Input(InputEvent::Key {
                                            code,
                                            state: key_state,
                                        }));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) => {
                        warn!(name = %dev_name, error = %e, "evdev fetch_events error");
                        break;
                    }
                }
            }
        }

        if let Some(active) = pending_grab_change {
            grab_guard.set_grab(devices, active);
        }
    }

    // Never exit leaving physical input devices exclusively grabbed.
    grab_guard.set_grab(devices, false);
}
