//! The Windows low-level keyboard hook.
//!
//! Two rules govern everything in this file.
//!
//! **Never block the input path.** This callback runs for every keystroke on the
//! machine. If it does not return within `LowLevelHooksTimeout` (300 ms by
//! default) Windows silently removes the hook: no error, no crash, the app just
//! quietly stops working. So the callback only ever compares against a snapshot
//! and posts to a channel. Launching the app happens on a worker thread. It also
//! takes locks with `try_read` and gives up instantly rather than waiting, since
//! a missed keypress is far better than a stalled keyboard.
//!
//! **Never persist a keystroke.** Keys that do not match a binding are compared
//! and dropped on the spot. Nothing is written to disk, buffered, or counted.
//! The only key that ever leaves this file is one the user deliberately captured
//! in learn mode, or one that matched a binding they created.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{OnceLock, RwLock};
use std::thread;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::config::Binding;
use crate::keys::{is_modifier, Chord, VK_CONTROL};

/// Stamped onto keys we synthesise ourselves, so the hook ignores its own work.
const INJECT_TAG: usize = 0x6761_7465; // "gate"

const M_CTRL: u8 = 1;
const M_SHIFT: u8 = 2;
const M_ALT: u8 = 4;
const M_WIN: u8 = 8;

static BINDINGS: OnceLock<RwLock<Vec<Binding>>> = OnceLock::new();
static ACTION_TX: OnceLock<Sender<Binding>> = OnceLock::new();
static LEARN_TX: OnceLock<Sender<Chord>> = OnceLock::new();

static ENABLED: AtomicBool = AtomicBool::new(true);
static LEARN_MODE: AtomicBool = AtomicBool::new(false);
/// Which modifiers are currently held, tracked from the hook itself so the
/// callback never has to make a syscall to find out.
static MODS: AtomicU8 = AtomicU8::new(0);
/// The vk of a keydown we swallowed, so its keyup can be swallowed too. A stray
/// keyup with no matching keydown confuses applications. 0 means none.
static SWALLOWED_VK: AtomicU32 = AtomicU32::new(0);
/// How many times the callback has run. A bare count with no key identity, so
/// it is safe to report; it exists to answer "is the hook alive at all".
static SEEN: AtomicU64 = AtomicU64::new(0);

fn bindings() -> &'static RwLock<Vec<Binding>> {
    BINDINGS.get_or_init(|| RwLock::new(Vec::new()))
}

static DEBUG: OnceLock<bool> = OnceLock::new();

/// Diagnostics, off unless GATEDKEY_DEBUG is set.
///
/// This can only ever report modifiers and F13-F24. Ordinary typing is
/// unreportable by construction, not by discipline, so turning diagnostics on
/// can never turn this into a keylogger.
pub fn debug_on() -> bool {
    *DEBUG.get_or_init(|| std::env::var("GATEDKEY_DEBUG").is_ok())
}

fn diagnosable(vk: u32) -> bool {
    is_modifier(vk) || (0x7C..=0x87).contains(&vk)
}

/// Replace the active binding set. Called whenever the user saves.
pub fn set_bindings(new: Vec<Binding>) {
    if let Ok(mut guard) = bindings().write() {
        *guard = new;
    }
}

/// Master on/off. The hook stays installed either way, so toggling can never
/// leave a key half-captured.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// In learn mode every non-modifier keydown is captured and swallowed, and sent
/// to the UI so the user can see what they pressed.
pub fn set_learn_mode(on: bool) {
    LEARN_MODE.store(on, Ordering::Relaxed);
}

/// Install the hook and start the worker threads. Returns false if the hook
/// could not be installed, in which case no binding will ever fire.
///
/// **Must be called from the thread that runs the application's message loop**,
/// which for Tauri is the main thread inside `setup`.
///
/// A low-level hook is dispatched on the thread that installed it, and that
/// thread has to be pumping messages. Installing it on a dedicated worker thread
/// with its own `GetMessage` loop was tried first, and it fails in the worst
/// possible way: `SetWindowsHookExW` returns a valid `HHOOK`, the loop runs, and
/// the callback is simply never invoked. The hook looks installed and does
/// nothing. That cost a long debugging session, so do not "tidy" this back onto
/// its own thread.
///
/// `on_action` fires when a binding matches. `on_learn` fires when a key is
/// captured in learn mode. Both run on worker threads, off the input path.
pub fn start<F, L>(on_action: F, on_learn: L) -> bool
where
    F: Fn(Binding) + Send + 'static,
    L: Fn(Chord) + Send + 'static,
{
    let (action_tx, action_rx) = channel::<Binding>();
    let (learn_tx, learn_rx) = channel::<Chord>();
    let _ = ACTION_TX.set(action_tx);
    let _ = LEARN_TX.set(learn_tx);
    bindings();

    thread::spawn(move || {
        for binding in action_rx {
            on_action(binding);
        }
    });
    thread::spawn(move || {
        for chord in learn_rx {
            on_learn(chord);
        }
    });

    // Liveness reporter, only under GATEDKEY_DEBUG. Prints a count and never a
    // key, so it answers "is the hook alive at all" without being a keylogger.
    // That question is worth being able to answer: a hook can install
    // successfully and still never be invoked.
    if debug_on() {
        thread::spawn(|| {
            let mut last = 0u64;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let now = SEEN.load(Ordering::Relaxed);
                if now != last {
                    eprintln!("gatedkey: hook callbacks seen: {now}");
                    last = now;
                }
            }
        });
    }

    unsafe {
        let hmod = GetModuleHandleW(None)
            .map(|m| HINSTANCE(m.0))
            .unwrap_or_default();
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(hmod), 0) {
            Ok(_) => {
                if debug_on() {
                    eprintln!("gatedkey: keyboard hook installed");
                }
                true
            }
            Err(err) => {
                eprintln!("gatedkey: could not install keyboard hook: {err}");
                false
            }
        }
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    SEEN.fetch_add(1, Ordering::Relaxed);

    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

    // Anything we injected ourselves passes straight through, or the Ctrl tap
    // below would recurse into this function forever.
    if kb.dwExtraInfo == INJECT_TAG {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let msg = wparam.0 as u32;
    let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
    let vk = kb.vkCode;

    if debug_on() && diagnosable(vk) {
        eprintln!(
            "gatedkey: hook saw vk={vk:#X} msg={msg:#X} down={is_down} up={is_up} injected={}",
            kb.flags.0 & 0x10 != 0
        );
    }

    // Modifiers only ever update the held-state mask. They never match on their
    // own, and they must always reach the rest of the system.
    if is_modifier(vk) {
        update_mods(vk, is_down);
        return CallNextHookEx(None, code, wparam, lparam);
    }

    // Swallow the keyup belonging to a keydown we already swallowed. This runs
    // even when disabled, so turning off mid-press cannot strand a keyup.
    if is_up && SWALLOWED_VK.load(Ordering::Relaxed) == vk {
        SWALLOWED_VK.store(0, Ordering::Relaxed);
        return LRESULT(1);
    }

    if !ENABLED.load(Ordering::Relaxed) {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    if is_down {
        let m = MODS.load(Ordering::Relaxed);
        let chord = Chord {
            vk,
            ctrl: m & M_CTRL != 0,
            shift: m & M_SHIFT != 0,
            alt: m & M_ALT != 0,
            win: m & M_WIN != 0,
        };

        if LEARN_MODE.load(Ordering::Relaxed) {
            if let Some(tx) = LEARN_TX.get() {
                let _ = tx.send(chord);
            }
            SWALLOWED_VK.store(vk, Ordering::Relaxed);
            if chord.win {
                break_start_menu();
            }
            return LRESULT(1);
        }

        if debug_on() && diagnosable(vk) {
            let loaded = bindings().try_read().map(|s| s.len());
            eprintln!(
                "gatedkey: keydown vk={vk:#X} mods={m:#X} chord={} bindings_loaded={loaded:?}",
                chord.label()
            );
        }

        // try_read, never read. Waiting on a writer here would stall every
        // keystroke on the machine behind a config save.
        if let Some(binding) = bindings()
            .try_read()
            .ok()
            .and_then(|set| set.iter().find(|b| b.enabled && b.chord == chord).cloned())
        {
            // The clone above allocates, which is normally forbidden here, but it
            // only happens on an actual match: once per deliberate keypress.
            if let Some(tx) = ACTION_TX.get() {
                let _ = tx.send(binding);
            }
            SWALLOWED_VK.store(vk, Ordering::Relaxed);
            if chord.win {
                break_start_menu();
            }
            return LRESULT(1);
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}

fn update_mods(vk: u32, down: bool) {
    let bit = match vk {
        0x11 | 0xA2 | 0xA3 => M_CTRL,
        0x10 | 0xA0 | 0xA1 => M_SHIFT,
        0x12 | 0xA4 | 0xA5 => M_ALT,
        0x5B | 0x5C => M_WIN,
        _ => return,
    };
    if down {
        MODS.fetch_or(bit, Ordering::Relaxed);
    } else {
        MODS.fetch_and(!bit, Ordering::Relaxed);
    }
}

/// Windows opens the Start menu when the Win key is released and nothing was
/// pressed while it was held. We just swallowed the key that was pressed, so
/// from the shell's point of view nothing happened and Start would pop up.
/// Injecting a harmless Ctrl tap breaks that gesture.
unsafe fn break_start_menu() {
    let mut inputs = [INPUT::default(); 2];
    inputs[0].r#type = INPUT_KEYBOARD;
    inputs[0].Anonymous = INPUT_0 {
        ki: KEYBDINPUT {
            wVk: VIRTUAL_KEY(VK_CONTROL as u16),
            dwExtraInfo: INJECT_TAG,
            ..Default::default()
        },
    };
    inputs[1].r#type = INPUT_KEYBOARD;
    inputs[1].Anonymous = INPUT_0 {
        ki: KEYBDINPUT {
            wVk: VIRTUAL_KEY(VK_CONTROL as u16),
            dwFlags: KEYEVENTF_KEYUP,
            dwExtraInfo: INJECT_TAG,
            ..Default::default()
        },
    };
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        MODS.store(0, Ordering::Relaxed);
        SWALLOWED_VK.store(0, Ordering::Relaxed);
    }

    #[test]
    fn modifier_state_tracks_press_and_release() {
        reset();
        update_mods(0xA0, true); // left shift down
        assert_eq!(MODS.load(Ordering::Relaxed) & M_SHIFT, M_SHIFT);
        update_mods(0x5B, true); // left win down
        assert_eq!(MODS.load(Ordering::Relaxed) & M_WIN, M_WIN);
        update_mods(0xA0, false);
        assert_eq!(MODS.load(Ordering::Relaxed) & M_SHIFT, 0);
        assert_eq!(MODS.load(Ordering::Relaxed) & M_WIN, M_WIN);
        reset();
    }

    #[test]
    fn left_and_right_modifiers_set_the_same_bit() {
        reset();
        update_mods(0xA2, true); // left ctrl
        update_mods(0xA3, true); // right ctrl
        update_mods(0xA2, false);
        // Releasing one clears the bit. Tracking each side separately would be
        // more precise, but no chord distinguishes them.
        assert_eq!(MODS.load(Ordering::Relaxed) & M_CTRL, 0);
        reset();
    }

    #[test]
    fn non_modifier_keys_do_not_touch_the_mask() {
        reset();
        update_mods(0x41, true); // 'A'
        assert_eq!(MODS.load(Ordering::Relaxed), 0);
        reset();
    }

    #[test]
    fn enabled_and_learn_flags_toggle() {
        set_enabled(false);
        assert!(!ENABLED.load(Ordering::Relaxed));
        set_enabled(true);
        assert!(ENABLED.load(Ordering::Relaxed));
        set_learn_mode(true);
        assert!(LEARN_MODE.load(Ordering::Relaxed));
        set_learn_mode(false);
        assert!(!LEARN_MODE.load(Ordering::Relaxed));
    }
}
