//! State shared between the input-hook thread, the engine thread and the GUI.
//! The hook's hot path only touches atomics: no locks, no allocation.

use crate::keys::{InputId, NUM_INPUTS};
use crate::pad::{ControllerKind, PadState};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering::*};
use std::sync::Mutex;

const WORDS: usize = NUM_INPUTS / 64;

#[derive(Default)]
pub struct Stats {
    pub events: AtomicU64,
    pub reports: AtomicU64,
    pub dropped: AtomicU64,
    /// Hook timestamp -> report submitted to the driver, microseconds.
    pub last_latency_us: AtomicU32,
    pub max_latency_us: AtomicU32,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub pad: PadState,
    pub controller: ControllerKind,
    pub pad_connected: bool,
    pub pad_error: Option<String>,
    pub user_enabled: bool,
    pub active: bool,
    pub suspended_reason: Option<String>,
    pub profile: String,
    pub foreground_exe: String,
    pub active_layers: Vec<String>,
    pub mouse_source: &'static str,
    pub config_errors: Vec<String>,
}

pub struct Shared {
    block: [AtomicU64; WORDS],
    /// Emulation active: hooks forward events and honour the block table.
    pub active: AtomicBool,
    /// Block mouse movement and forward deltas (mouse bound to a stick).
    pub capture_mouse: AtomicBool,
    pub recenter_cursor: AtomicBool,
    pub toggle_input: AtomicU16,
    /// GUI asks the hook to capture the next pressed input.
    pub capture_next: AtomicBool,
    /// Captured input id + 1 (0 = nothing yet).
    pub captured: AtomicU32,
    /// 0 = auto, 1 = raw, 2 = hook.
    pub mouse_source: AtomicU8,
    pub stats: Stats,
    pub status: Mutex<Status>,
    pub quit: AtomicBool,
}

impl Default for Shared {
    fn default() -> Self {
        Shared {
            block: Default::default(),
            active: AtomicBool::new(false),
            capture_mouse: AtomicBool::new(false),
            recenter_cursor: AtomicBool::new(true),
            toggle_input: AtomicU16::new(0x77), // F8
            capture_next: AtomicBool::new(false),
            captured: AtomicU32::new(0),
            mouse_source: AtomicU8::new(0),
            stats: Stats::default(),
            status: Mutex::new(Status::default()),
            quit: AtomicBool::new(false),
        }
    }
}

impl Shared {
    #[inline]
    pub fn is_blocked(&self, id: InputId) -> bool {
        let i = id as usize;
        if i >= NUM_INPUTS {
            return false;
        }
        self.block[i / 64].load(Relaxed) & (1u64 << (i % 64)) != 0
    }

    pub fn publish_block(&self, bits: &[u64; WORDS]) {
        for (a, v) in self.block.iter().zip(bits.iter()) {
            a.store(*v, Relaxed);
        }
    }

    pub fn clear_block(&self) {
        for a in &self.block {
            a.store(0, Relaxed);
        }
    }

    pub fn record_latency(&self, us: u32) {
        self.stats.last_latency_us.store(us, Relaxed);
        self.stats.max_latency_us.fetch_max(us, Relaxed);
    }

    pub fn status_mut<R>(&self, f: impl FnOnce(&mut Status) -> R) -> R {
        let mut g = self.status.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut g)
    }

    pub fn status_snapshot(&self) -> Status {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub type BlockBits = [u64; WORDS];

#[inline]
pub fn set_bit(bits: &mut BlockBits, id: InputId) {
    let i = id as usize;
    if i < NUM_INPUTS {
        bits[i / 64] |= 1u64 << (i % 64);
    }
}
