//! Push-to-talk: hold Left Ctrl + Left Alt (Left Control + Left Option on macOS) to talk to Aloe.
//!
//! No hotkey API can bind a chord made only of modifiers, and none can tell left from right, so
//! this watches the keyboard itself: a low-level hook on Windows, a listen-only event tap on macOS.
//! Both only observe — every key still reaches the app it was meant for.
//!
//! The platform side turns raw key transitions into `Input`s on a channel. One thread runs them
//! through `Gesture`, which decides what the user meant, and that is the only part with any logic.

use std::{
    collections::HashSet,
    sync::mpsc,
    time::{Duration, Instant},
};

use tokio::sync::mpsc::UnboundedSender;

use crate::config::VOICE_ARM_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    LeftCtrl,
    LeftAlt,
    /// Esc on its own dismisses the voice overlay. It still reaches the app in front, as every key does.
    Escape,
    /// Any other key, by platform key code. Right Alt is one: AltGr arrives as Left Ctrl + Right
    /// Alt, so typing an AltGr character never looks like the chord.
    Other(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Down(Key),
    Up(Key),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The chord has been held long enough: start listening.
    Start,
    /// Released after `Start`: what was said is done.
    Stop,
    /// Another key joined the chord after `Start` — it was a shortcut, not speech. Discard.
    Cancel,
    /// Pressed and let go before `Start`: talk hands-free, or stop Aloe talking.
    Tap,
    /// Esc on its own, with no chord involved.
    Escape,
}

/// Stands in for Esc's own key code when Esc is counted as "some other key" for the chord.
const ESCAPE_AS_OTHER: u32 = 0xE5C;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Armed(Instant),
    Recording,
    /// Something other than the chord was involved. Nothing more happens until both are let go.
    Spoiled,
}

pub struct Gesture {
    ctrl: bool,
    alt: bool,
    others: HashSet<u32>,
    phase: Phase,
    arm: Duration,
    /// Whether a key is physically down right now, asked of the OS. The hook can miss a release
    /// (it never sees keys meant for an elevated window, a UAC prompt or the lock screen), and one
    /// missed release left that key "held" here forever: every chord after it was spoiled.
    is_held: fn(Key) -> bool,
}

impl Gesture {
    pub fn new(arm: Duration, is_held: fn(Key) -> bool) -> Self {
        Self { ctrl: false, alt: false, others: HashSet::new(), phase: Phase::Idle, arm, is_held }
    }

    /// Drops keys this thinks are held but the OS says are not, before a press that could start
    /// something. A recording whose release went missing ends with a Stop instead of hanging on.
    fn forget_missed_releases(&mut self) -> Option<HotkeyEvent> {
        let is_held = self.is_held;
        self.others.retain(|&code| is_held(if code == ESCAPE_AS_OTHER { Key::Escape } else { Key::Other(code) }));
        self.ctrl &= is_held(Key::LeftCtrl);
        self.alt &= is_held(Key::LeftAlt);
        if self.ctrl || self.alt || !self.others.is_empty() {
            return None;
        }
        let missed = match self.phase {
            Phase::Recording => Some(HotkeyEvent::Stop),
            _ => None,
        };
        self.phase = Phase::Idle;
        missed
    }

    pub fn input(&mut self, input: Input, now: Instant) -> Option<HotkeyEvent> {
        if matches!(input, Input::Down(Key::LeftCtrl | Key::LeftAlt | Key::Escape)) {
            if let Some(missed) = self.forget_missed_releases() {
                return Some(missed);
            }
        }
        match input {
            Input::Down(Key::Escape) => {
                let alone = self.phase == Phase::Idle && !self.ctrl && !self.alt && self.others.is_empty();
                let chord = self.input(Input::Down(Key::Other(ESCAPE_AS_OTHER)), now);
                if alone {
                    Some(HotkeyEvent::Escape)
                } else {
                    chord
                }
            }
            Input::Up(Key::Escape) => self.input(Input::Up(Key::Other(ESCAPE_AS_OTHER)), now),
            // Auto-repeat sends the held key again and again; only the first press counts.
            Input::Down(Key::LeftCtrl) if self.ctrl => None,
            Input::Down(Key::LeftAlt) if self.alt => None,
            Input::Down(Key::LeftCtrl) | Input::Down(Key::LeftAlt) => {
                if input == Input::Down(Key::LeftCtrl) {
                    self.ctrl = true;
                } else {
                    self.alt = true;
                }
                if self.ctrl && self.alt && self.phase == Phase::Idle {
                    // Holding some other key first (Shift, a letter) makes it a different shortcut.
                    self.phase = if self.others.is_empty() { Phase::Armed(now) } else { Phase::Spoiled };
                }
                None
            }
            Input::Down(Key::Other(code)) => {
                self.others.insert(code);
                match self.phase {
                    Phase::Armed(_) => {
                        self.phase = Phase::Spoiled;
                        None
                    }
                    Phase::Recording => {
                        self.phase = Phase::Spoiled;
                        Some(HotkeyEvent::Cancel)
                    }
                    Phase::Idle | Phase::Spoiled => None,
                }
            }
            Input::Up(Key::Other(code)) => {
                self.others.remove(&code);
                None
            }
            Input::Up(key) => {
                if key == Key::LeftCtrl {
                    self.ctrl = false;
                } else {
                    self.alt = false;
                }
                let event = match self.phase {
                    Phase::Armed(_) => Some(HotkeyEvent::Tap),
                    Phase::Recording => Some(HotkeyEvent::Stop),
                    Phase::Idle | Phase::Spoiled => None,
                };
                self.phase = match self.phase {
                    // Spoiled lasts until the chord is fully let go, so releasing just the stray
                    // key with Ctrl+Alt still down can't start a recording halfway through.
                    Phase::Spoiled if self.ctrl || self.alt => Phase::Spoiled,
                    _ => Phase::Idle,
                };
                event
            }
        }
    }

    /// Promotes a held chord to a recording once it has been held for the arm delay.
    pub fn tick(&mut self, now: Instant) -> Option<HotkeyEvent> {
        match self.phase {
            Phase::Armed(since) if now.duration_since(since) >= self.arm => {
                self.phase = Phase::Recording;
                Some(HotkeyEvent::Start)
            }
            _ => None,
        }
    }

    /// When `tick` next needs to run, if it does.
    pub fn deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::Armed(since) => Some(since + self.arm),
            _ => None,
        }
    }
}

/// Starts watching the keyboard. Events go to `events` for as long as the app runs; whether voice
/// is switched on is the receiver's concern, so turning it off and on never re-installs a hook.
pub fn spawn(events: UnboundedSender<HotkeyEvent>) {
    let (inputs, input_rx) = mpsc::channel::<Input>();
    std::thread::Builder::new()
        .name("aloe-voice-gesture".into())
        .spawn(move || run_gesture(input_rx, events))
        .expect("spawn voice gesture thread");
    platform::install(inputs);
}

fn run_gesture(inputs: mpsc::Receiver<Input>, events: UnboundedSender<HotkeyEvent>) {
    let mut gesture = Gesture::new(Duration::from_millis(VOICE_ARM_MS), platform::is_held);
    loop {
        let received = match gesture.deadline() {
            Some(deadline) => inputs.recv_timeout(deadline.saturating_duration_since(Instant::now())),
            None => inputs.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        let event = match received {
            Ok(input) => gesture.input(input, Instant::now()),
            Err(mpsc::RecvTimeoutError::Timeout) => gesture.tick(Instant::now()),
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        if let Some(event) = event {
            if events.send(event).is_err() {
                return;
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{Input, Key};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        OnceLock,
    };
    use windows_sys::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_ESCAPE, VK_LCONTROL, VK_LMENU},
            WindowsAndMessaging::{
                CallNextHookEx, GetMessageW, SetWindowsHookExW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
                WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
            },
        },
    };

    use crate::config::debug_log;

    /// An unassigned virtual-key code. Tapping it before Alt comes up means Alt was "used with
    /// another key", so the app in front doesn't take the release as a request to open its menu
    /// bar (which would also steal its own keyboard focus). PowerToys does the same.
    const VK_MASK_ALT_MENU: u16 = 0xE8;

    static INPUTS: OnceLock<Sender<Input>> = OnceLock::new();
    // Only touched on the hook thread; atomics so they can be statics.
    static CTRL_DOWN: AtomicBool = AtomicBool::new(false);
    static ALT_DOWN: AtomicBool = AtomicBool::new(false);
    /// Both were down together since Alt went down: Alt's release needs the menu masked.
    static CHORD_SEEN: AtomicBool = AtomicBool::new(false);

    /// The keyboard's real state, whichever window has focus. Other keys are tracked by vkCode.
    pub fn is_held(key: Key) -> bool {
        let vk = match key {
            Key::LeftCtrl => VK_LCONTROL as i32,
            Key::LeftAlt => VK_LMENU as i32,
            Key::Escape => VK_ESCAPE as i32,
            Key::Other(code) => code as i32,
        };
        unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
    }

    pub fn install(inputs: Sender<Input>) {
        if INPUTS.set(inputs).is_err() {
            return;
        }
        std::thread::Builder::new()
            .name("aloe-voice-hook".into())
            .spawn(|| unsafe {
                // A low-level hook is called on the thread that installed it, from inside its
                // message loop, so this thread must keep pumping messages for as long as it lives.
                let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(std::ptr::null()), 0);
                if hook.is_null() {
                    debug_log("voice", "hook_error", std::io::Error::last_os_error().to_string());
                    return;
                }
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {}
            })
            .expect("spawn voice hook thread");
    }

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let info = &*(lparam as *const KBDLLHOOKSTRUCT);
            // Aloe's own synthetic typing (and the mask key below) must never count as the user.
            if info.flags & LLKHF_INJECTED == 0 {
                let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                let up = matches!(wparam as u32, WM_KEYUP | WM_SYSKEYUP);
                if down || up {
                    observe(info.vkCode, down);
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    /// Runs inside the hook, which Windows times out and silently removes if it is slow: a
    /// channel send and, at most, one SendInput.
    fn observe(vk: u32, down: bool) {
        let key = match vk as u16 {
            VK_LCONTROL => Key::LeftCtrl,
            VK_LMENU => Key::LeftAlt,
            VK_ESCAPE => Key::Escape,
            _ => Key::Other(vk),
        };
        match (key, down) {
            (Key::LeftCtrl, _) => CTRL_DOWN.store(down, Ordering::SeqCst),
            (Key::LeftAlt, true) => ALT_DOWN.store(true, Ordering::SeqCst),
            (Key::LeftAlt, false) => {
                ALT_DOWN.store(false, Ordering::SeqCst);
                if CHORD_SEEN.swap(false, Ordering::SeqCst) {
                    mask_alt_menu();
                }
            }
            _ => {}
        }
        if CTRL_DOWN.load(Ordering::SeqCst) && ALT_DOWN.load(Ordering::SeqCst) {
            CHORD_SEEN.store(true, Ordering::SeqCst);
        }
        if let Some(inputs) = INPUTS.get() {
            let _ = inputs.send(if down { Input::Down(key) } else { Input::Up(key) });
        }
    }

    fn mask_alt_menu() {
        let key = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_MASK_ALT_MENU, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        let inputs = [key(0), key(KEYEVENTF_KEYUP)];
        unsafe {
            SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    //! A listen-only Quartz event tap. It needs Input Monitoring permission; without it the tap
    //! can't be created, and the Settings pane says so (`input_monitoring_allowed`).

    use super::{Input, Key};
    use std::{
        ffi::c_void,
        sync::{
            atomic::{AtomicPtr, AtomicU64, Ordering},
            mpsc::Sender,
            OnceLock,
        },
    };

    use crate::config::debug_log;

    type CFMachPortRef = *mut c_void;
    type CFRunLoopSourceRef = *mut c_void;
    type CFRunLoopRef = *mut c_void;
    type CFStringRef = *const c_void;
    type CGEventRef = *mut c_void;
    type CGEventTapProxy = *mut c_void;
    type TapCallback = extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

    const KCG_SESSION_EVENT_TAP: u32 = 1;
    const KCG_HEAD_INSERT_EVENT_TAP: u32 = 0;
    const KCG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
    const KCG_EVENT_KEY_DOWN: u32 = 10;
    const KCG_EVENT_KEY_UP: u32 = 11;
    const KCG_EVENT_FLAGS_CHANGED: u32 = 12;
    const KCG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    const KCG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
    const KCG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
    const KVK_ESCAPE: u32 = 53;

    /// Device-dependent modifier bits (IOKit's NX_DEVICE*KEYMASK): the only place the flags say
    /// which side was pressed. Left Control and Left Option are the chord; the rest are other keys.
    const LEFT_CTRL: u64 = 0x0000_0001;
    const LEFT_ALT: u64 = 0x0000_0020;
    const OTHER_MODIFIERS: [u64; 6] = [
        0x0000_0002, // left shift
        0x0000_0004, // right shift
        0x0000_0008, // left command
        0x0000_0010, // right command
        0x0000_0040, // right option
        0x0000_2000, // right control
    ];
    /// Keeps modifier "key codes" clear of real key codes, which are below 0x80.
    const MODIFIER_CODE_BASE: u32 = 0x1_0000;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(tap: u32, place: u32, options: u32, events_of_interest: u64, callback: TapCallback, user_info: *mut c_void) -> CFMachPortRef;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        fn CGEventGetFlags(event: CGEventRef) -> u64;
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFRunLoopCommonModes: CFStringRef;
        fn CFMachPortCreateRunLoopSource(allocator: *const c_void, port: CFMachPortRef, order: isize) -> CFRunLoopSourceRef;
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
        fn CFRunLoopRun();
    }

    static INPUTS: OnceLock<Sender<Input>> = OnceLock::new();
    static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static LAST_FLAGS: AtomicU64 = AtomicU64::new(0);

    /// The tap's flag edges stand in for key state here, so trust what it tracked.
    pub fn is_held(_key: Key) -> bool {
        true
    }

    pub fn input_monitoring_allowed() -> bool {
        unsafe { CGPreflightListenEventAccess() }
    }

    pub fn install(inputs: Sender<Input>) {
        if INPUTS.set(inputs).is_err() {
            return;
        }
        std::thread::Builder::new()
            .name("aloe-voice-tap".into())
            .spawn(|| unsafe {
                if !CGPreflightListenEventAccess() {
                    // Shows the system prompt once; the tap works from the next launch after the
                    // user allows it in System Settings › Privacy & Security › Input Monitoring.
                    CGRequestListenEventAccess();
                    debug_log("voice", "input_monitoring_denied", "push-to-talk is off until Input Monitoring is allowed");
                    return;
                }
                let mask = (1u64 << KCG_EVENT_KEY_DOWN) | (1u64 << KCG_EVENT_KEY_UP) | (1u64 << KCG_EVENT_FLAGS_CHANGED);
                let tap = CGEventTapCreate(
                    KCG_SESSION_EVENT_TAP,
                    KCG_HEAD_INSERT_EVENT_TAP,
                    KCG_EVENT_TAP_OPTION_LISTEN_ONLY,
                    mask,
                    tap_callback,
                    std::ptr::null_mut(),
                );
                if tap.is_null() {
                    debug_log("voice", "tap_error", "CGEventTapCreate returned null");
                    return;
                }
                TAP.store(tap, Ordering::SeqCst);
                let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
                CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap, true);
                CFRunLoopRun();
            })
            .expect("spawn voice tap thread");
    }

    extern "C" fn tap_callback(_proxy: CGEventTapProxy, kind: u32, event: CGEventRef, _user: *mut c_void) -> CGEventRef {
        let send = |input| {
            if let Some(inputs) = INPUTS.get() {
                let _ = inputs.send(input);
            }
        };
        unsafe {
            match kind {
                // macOS switches off a tap it thinks is too slow, or while secure input is on.
                KCG_EVENT_TAP_DISABLED_BY_TIMEOUT | KCG_EVENT_TAP_DISABLED_BY_USER_INPUT => {
                    let tap = TAP.load(Ordering::SeqCst);
                    if !tap.is_null() {
                        CGEventTapEnable(tap, true);
                    }
                }
                KCG_EVENT_KEY_DOWN | KCG_EVENT_KEY_UP => {
                    let code = CGEventGetIntegerValueField(event, KCG_KEYBOARD_EVENT_KEYCODE) as u32;
                    let key = if code == KVK_ESCAPE { Key::Escape } else { Key::Other(code) };
                    send(if kind == KCG_EVENT_KEY_DOWN { Input::Down(key) } else { Input::Up(key) });
                }
                KCG_EVENT_FLAGS_CHANGED => {
                    let flags = CGEventGetFlags(event);
                    let before = LAST_FLAGS.swap(flags, Ordering::SeqCst);
                    let edge = |mask: u64, key: Key| {
                        let (was, is) = (before & mask != 0, flags & mask != 0);
                        if was != is {
                            send(if is { Input::Down(key) } else { Input::Up(key) });
                        }
                    };
                    edge(LEFT_CTRL, Key::LeftCtrl);
                    edge(LEFT_ALT, Key::LeftAlt);
                    for mask in OTHER_MODIFIERS {
                        edge(mask, Key::Other(MODIFIER_CODE_BASE | mask as u32));
                    }
                }
                _ => {}
            }
        }
        event
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::{Input, Key};
    use std::sync::mpsc::Sender;

    pub fn is_held(_key: Key) -> bool {
        true
    }

    pub fn install(_inputs: Sender<Input>) {
        crate::config::debug_log("voice", "unsupported", "push-to-talk is only available on Windows and macOS");
    }
}

/// Whether the OS lets Aloe see the chord at all. Only macOS ever says no (Input Monitoring).
pub fn hotkey_allowed() -> bool {
    #[cfg(target_os = "macos")]
    return platform::input_monitoring_allowed();
    #[cfg(not(target_os = "macos"))]
    return cfg!(windows);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARM: Duration = Duration::from_millis(200);

    fn ms(start: Instant, n: u64) -> Instant {
        start + Duration::from_millis(n)
    }

    #[test]
    fn hold_then_release_records() {
        let t = Instant::now();
        let mut g = Gesture::new(ARM, |_| true);
        assert_eq!(g.input(Input::Down(Key::LeftCtrl), t), None);
        assert_eq!(g.input(Input::Down(Key::LeftAlt), ms(t, 10)), None);
        assert_eq!(g.tick(ms(t, 100)), None);
        assert_eq!(g.deadline(), Some(ms(t, 210)));
        assert_eq!(g.tick(ms(t, 210)), Some(HotkeyEvent::Start));
        assert_eq!(g.input(Input::Down(Key::LeftAlt), ms(t, 300)), None, "auto-repeat is ignored");
        assert_eq!(g.input(Input::Up(Key::LeftAlt), ms(t, 900)), Some(HotkeyEvent::Stop));
        assert_eq!(g.input(Input::Up(Key::LeftCtrl), ms(t, 910)), None);
    }

    #[test]
    fn short_press_is_a_tap() {
        let t = Instant::now();
        let mut g = Gesture::new(ARM, |_| true);
        g.input(Input::Down(Key::LeftAlt), t);
        g.input(Input::Down(Key::LeftCtrl), ms(t, 5));
        assert_eq!(g.input(Input::Up(Key::LeftCtrl), ms(t, 80)), Some(HotkeyEvent::Tap));
        assert_eq!(g.tick(ms(t, 500)), None);
        assert_eq!(g.deadline(), None);
    }

    #[test]
    fn shortcut_key_inside_arm_delay_never_starts() {
        let t = Instant::now();
        let mut g = Gesture::new(ARM, |_| true);
        g.input(Input::Down(Key::LeftCtrl), t);
        g.input(Input::Down(Key::LeftAlt), ms(t, 5));
        assert_eq!(g.input(Input::Down(Key::Other(0x54)), ms(t, 60)), None);
        assert_eq!(g.tick(ms(t, 500)), None);
        assert_eq!(g.input(Input::Up(Key::Other(0x54)), ms(t, 600)), None);
        assert_eq!(g.tick(ms(t, 900)), None, "still spoiled while the chord is held");
        assert_eq!(g.input(Input::Up(Key::LeftAlt), ms(t, 950)), None);
        assert_eq!(g.input(Input::Up(Key::LeftCtrl), ms(t, 960)), None);
        // A fresh hold works again.
        g.input(Input::Down(Key::LeftCtrl), ms(t, 1000));
        g.input(Input::Down(Key::LeftAlt), ms(t, 1001));
        assert_eq!(g.tick(ms(t, 1201)), Some(HotkeyEvent::Start));
    }

    #[test]
    fn key_while_recording_cancels() {
        let t = Instant::now();
        let mut g = Gesture::new(ARM, |_| true);
        g.input(Input::Down(Key::LeftCtrl), t);
        g.input(Input::Down(Key::LeftAlt), t);
        assert_eq!(g.tick(ms(t, 200)), Some(HotkeyEvent::Start));
        assert_eq!(g.input(Input::Down(Key::Other(0x2E)), ms(t, 700)), Some(HotkeyEvent::Cancel));
        assert_eq!(g.input(Input::Up(Key::LeftAlt), ms(t, 800)), None, "no Stop after a Cancel");
    }

    #[test]
    fn escape_alone_dismisses_but_inside_the_chord_cancels() {
        let t = Instant::now();
        let mut g = Gesture::new(ARM, |_| true);
        assert_eq!(g.input(Input::Down(Key::Escape), t), Some(HotkeyEvent::Escape));
        assert_eq!(g.input(Input::Down(Key::Escape), t), None, "auto-repeat while held");
        assert_eq!(g.input(Input::Up(Key::Escape), t), None);

        g.input(Input::Down(Key::LeftCtrl), t);
        g.input(Input::Down(Key::LeftAlt), t);
        assert_eq!(g.tick(ms(t, 200)), Some(HotkeyEvent::Start));
        assert_eq!(g.input(Input::Down(Key::Escape), ms(t, 300)), Some(HotkeyEvent::Cancel));
    }

    #[test]
    fn altgr_never_arms() {
        // AltGr = Left Ctrl + Right Alt; Right Alt is just another key.
        let t = Instant::now();
        let mut g = Gesture::new(ARM, |_| true);
        g.input(Input::Down(Key::LeftCtrl), t);
        g.input(Input::Down(Key::Other(0xA5)), t);
        assert_eq!(g.deadline(), None);
        assert_eq!(g.tick(ms(t, 1000)), None);
    }

    #[test]
    fn a_missed_release_does_not_spoil_every_chord_after_it() {
        // Only the chord keys are really down; the Shift below was released where the hook
        // could not see it.
        let t = Instant::now();
        let mut g = Gesture::new(ARM, |key| matches!(key, Key::LeftCtrl | Key::LeftAlt));
        g.input(Input::Down(Key::Other(0xA0)), t);
        g.input(Input::Down(Key::LeftCtrl), ms(t, 100));
        g.input(Input::Down(Key::LeftAlt), ms(t, 101));
        assert_eq!(g.tick(ms(t, 301)), Some(HotkeyEvent::Start));
    }

    #[test]
    fn a_recording_whose_release_went_missing_stops_on_the_next_press() {
        let t = Instant::now();
        let mut g = Gesture::new(ARM, |_| true);
        g.input(Input::Down(Key::LeftCtrl), t);
        g.input(Input::Down(Key::LeftAlt), t);
        assert_eq!(g.tick(ms(t, 200)), Some(HotkeyEvent::Start));
        // Both were let go unseen.
        g.is_held = |_| false;
        assert_eq!(g.input(Input::Down(Key::LeftCtrl), ms(t, 5000)), Some(HotkeyEvent::Stop));
        g.is_held = |key| matches!(key, Key::LeftCtrl | Key::LeftAlt);
        g.input(Input::Up(Key::LeftCtrl), ms(t, 5050));
        g.input(Input::Down(Key::LeftCtrl), ms(t, 6000));
        g.input(Input::Down(Key::LeftAlt), ms(t, 6001));
        assert_eq!(g.tick(ms(t, 6201)), Some(HotkeyEvent::Start));
    }

    #[test]
    fn other_key_held_first_spoils_the_chord() {
        let t = Instant::now();
        let mut g = Gesture::new(ARM, |_| true);
        g.input(Input::Down(Key::Other(0xA0)), t); // shift
        g.input(Input::Down(Key::LeftCtrl), t);
        g.input(Input::Down(Key::LeftAlt), t);
        assert_eq!(g.tick(ms(t, 1000)), None);
    }
}
