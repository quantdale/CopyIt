//! Clipboard abstraction for protected-copy lifetime.
//! Keeps unsafe/platform code isolated. Production on Windows uses a small
//! Win32 backend (sequence-number guarded clear); tests and non-Windows use a
//! deterministic fake with the same protocol.
//!
//! NOTE: the two backends are selected by `cfg`; in any given build exactly one is
//! constructed by `platform_default`, so the other is dead code in that target.
//! The dead-code lint is therefore expected and intentionally silenced here.
#![allow(dead_code)]

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

/// How long a protected copy stays on the clipboard before being cleared
/// if still current. Matches the 30s target in the hardening campaign.
pub const SECURITY_WINDOW_SECS: f64 = 30.0;

pub trait ClipboardBackend {
    fn set_text(&mut self, text: &str) -> io::Result<()>;
    fn clear(&mut self) -> io::Result<()>;
    fn sequence(&self) -> Option<u64>;
}

// ---------------------------------------------------------------------------
// Sim / fake backend (tests, sim harness, non-Windows dev).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SimState {
    text: String,
    seq: u64,
    fail_writes: bool,
}

/// Deterministic in-process clipboard for tests and the simulation harness.
pub struct SimClipboard {
    state: Rc<RefCell<SimState>>,
}

/// Observer handle that shares state with a `SimClipboard`. Cloned handles see
/// the same text/sequence, so tests can assert after the app mutates its backend.
#[derive(Clone)]
pub struct SimHandle {
    state: Rc<RefCell<SimState>>,
}

impl SimClipboard {
    pub fn new() -> (Self, SimHandle) {
        let state = Rc::new(RefCell::new(SimState::default()));
        (
            Self {
                state: state.clone(),
            },
            SimHandle { state },
        )
    }

    pub fn new_shared(handle: SimHandle) -> Self {
        Self {
            state: handle.state.clone(),
        }
    }
}

impl ClipboardBackend for SimClipboard {
    fn set_text(&mut self, text: &str) -> io::Result<()> {
        let mut s = self.state.borrow_mut();
        if s.fail_writes {
            return Err(io::Error::other("simulated clipboard failure"));
        }
        s.text = text.to_string();
        s.seq = s.seq.wrapping_add(1);
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        let mut s = self.state.borrow_mut();
        if s.fail_writes {
            return Err(io::Error::other("simulated clipboard failure"));
        }
        s.text.clear();
        s.seq = s.seq.wrapping_add(1);
        Ok(())
    }

    fn sequence(&self) -> Option<u64> {
        Some(self.state.borrow().seq)
    }
}

impl SimHandle {
    pub fn text(&self) -> String {
        self.state.borrow().text.clone()
    }

    pub fn sequence(&self) -> u64 {
        self.state.borrow().seq
    }

    pub fn overwrite(&self, text: &str) {
        let mut s = self.state.borrow_mut();
        s.text = text.to_string();
        s.seq = s.seq.wrapping_add(1);
    }

    pub fn set_fail_writes(&self, fail: bool) {
        self.state.borrow_mut().fail_writes = fail;
    }
}

// ---------------------------------------------------------------------------
// Windows backend (best-effort, sequence-guarded). Minimal unsafe, isolated.
// Falls back to Sim behavior when not on Windows so `cargo test` never touches
// the real clipboard.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub struct WindowsClipboard;

#[cfg(target_os = "windows")]
impl WindowsClipboard {
    #[allow(dead_code)] // only called via `platform_default`'s Windows path (not the test target)
    fn open() -> io::Result<()> {
        // `OpenClipboard` can fail transiently if another app holds it; retry a few times.
        for attempt in 0..5 {
            let ok = unsafe {
                windows_sys::Win32::System::DataExchange::OpenClipboard(std::ptr::null_mut())
            };
            if ok != 0 {
                return Ok(());
            }
            if attempt + 1 < 5 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "windows")]
impl ClipboardBackend for WindowsClipboard {
    fn set_text(&mut self, text: &str) -> io::Result<()> {
        use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
        use windows_sys::Win32::System::DataExchange::{EmptyClipboard, SetClipboardData};
        use windows_sys::Win32::System::Memory::{
            GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
        };
        use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        Self::open()?;
        let res: io::Result<()> = unsafe {
            if EmptyClipboard() == 0 {
                Err(io::Error::last_os_error())
            } else {
                let bytes = wide.len() * std::mem::size_of::<u16>();
                let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
                if h.is_null() {
                    Err(io::Error::last_os_error())
                } else {
                    let dest = GlobalLock(h);
                    if dest.is_null() {
                        GlobalFree(h);
                        Err(io::Error::last_os_error())
                    } else {
                        std::ptr::copy_nonoverlapping(wide.as_ptr(), dest as *mut u16, wide.len());
                        GlobalUnlock(h);
                        if SetClipboardData(CF_UNICODETEXT as u32, h as HANDLE).is_null() {
                            GlobalFree(h);
                            Err(io::Error::last_os_error())
                        } else {
                            Ok(())
                        }
                    }
                }
            }
        };
        unsafe {
            windows_sys::Win32::System::DataExchange::CloseClipboard();
        }
        res
    }

    fn clear(&mut self) -> io::Result<()> {
        use windows_sys::Win32::System::DataExchange::EmptyClipboard;
        Self::open()?;
        let res = unsafe {
            if EmptyClipboard() == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        unsafe {
            windows_sys::Win32::System::DataExchange::CloseClipboard();
        }
        res
    }

    fn sequence(&self) -> Option<u64> {
        Some(
            unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() }
                as u64,
        )
    }
}

/// Platform default backend. On Windows this is the real Win32 clipboard; on
/// other platforms (and in `cargo test` where touching the real clipboard would
/// be flaky) it is a deterministic SimClipboard.
pub fn platform_default() -> Box<dyn ClipboardBackend> {
    #[cfg(target_os = "windows")]
    {
        // In unit tests, avoid touching the real clipboard by default; test helpers
        // install a SimClipboard explicitly. Use cfg(test) to force Sim in tests.
        #[cfg(test)]
        {
            let (sim, _) = SimClipboard::new();
            Box::new(sim)
        }
        #[cfg(not(test))]
        {
            Box::new(WindowsClipboard)
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let (sim, _) = SimClipboard::new();
        Box::new(sim)
    }
}
