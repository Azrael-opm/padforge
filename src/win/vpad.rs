//! Virtual controller output through the ViGEmBus driver (signed, widely
//! installed by Steam / DS4Windows). Reports are only submitted when the state
//! actually changes (the engine de-duplicates), so driver IOCTL traffic is
//! proportional to real input.

use crate::engine::Backend;
use crate::pad::{encode_ds4, encode_x360, ControllerKind, PadState};
use vigem_client::{Client, DS4Report, DualShock4Wired, TargetId, XButtons, XGamepad, Xbox360Wired};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};

pub const VIGEM_URL: &str = "https://github.com/nefarius/ViGEmBus/releases";

enum Target {
    X360(Xbox360Wired<Client>),
    Ds4(DualShock4Wired<Client>),
}

#[derive(Default)]
pub struct VigemBackend {
    target: Option<Target>,
    timer: bool,
}

fn friendly(e: vigem_client::Error) -> String {
    match e {
        vigem_client::Error::BusNotFound => format!("ViGEmBus driver not found. Install it from {VIGEM_URL} (v1.22.0) and restart PadForge."),
        vigem_client::Error::BusVersionMismatch => "ViGEmBus version mismatch: update the driver to v1.22.0.".into(),
        other => format!("ViGEmBus error: {other}"),
    }
}

impl Backend for VigemBackend {
    fn pad_connect(&mut self, kind: ControllerKind) -> Result<(), String> {
        self.pad_disconnect();
        let client = Client::connect().map_err(friendly)?;
        let target = match kind {
            ControllerKind::Xbox360 => {
                let mut t = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
                t.plugin().map_err(friendly)?;
                t.wait_ready().map_err(friendly)?;
                Target::X360(t)
            }
            ControllerKind::Ds4 => {
                let mut t = DualShock4Wired::new(client, TargetId::DUALSHOCK4_WIRED);
                t.plugin().map_err(friendly)?;
                t.wait_ready().map_err(friendly)?;
                Target::Ds4(t)
            }
        };
        self.target = Some(target);
        Ok(())
    }

    fn pad_disconnect(&mut self) {
        match self.target.take() {
            Some(Target::X360(mut t)) => {
                let _ = t.update(&XGamepad::default());
                let _ = t.unplug();
            }
            Some(Target::Ds4(mut t)) => {
                let _ = t.update(&DS4Report::default());
                let _ = t.unplug();
            }
            None => {}
        }
    }

    fn pad_connected(&self) -> bool {
        self.target.is_some()
    }

    fn pad_send(&mut self, s: &PadState) -> Result<(), String> {
        match &mut self.target {
            Some(Target::X360(t)) => {
                let r = encode_x360(s);
                let g = XGamepad {
                    buttons: XButtons(r.buttons),
                    left_trigger: r.left_trigger,
                    right_trigger: r.right_trigger,
                    thumb_lx: r.thumb_lx,
                    thumb_ly: r.thumb_ly,
                    thumb_rx: r.thumb_rx,
                    thumb_ry: r.thumb_ry,
                };
                t.update(&g).map_err(friendly)
            }
            Some(Target::Ds4(t)) => {
                let r = encode_ds4(s);
                let rep = DS4Report {
                    thumb_lx: r.thumb_lx,
                    thumb_ly: r.thumb_ly,
                    thumb_rx: r.thumb_rx,
                    thumb_ry: r.thumb_ry,
                    buttons: r.buttons,
                    special: r.special,
                    trigger_l: r.trigger_l,
                    trigger_r: r.trigger_r,
                };
                t.update(&rep).map_err(friendly)
            }
            None => Err("virtual controller not connected".into()),
        }
    }

    fn set_high_res_timer(&mut self, on: bool) {
        // 1 ms scheduler granularity only while something is animating
        // (mouse decay / turbo / macros); released as soon as we go idle.
        if on != self.timer {
            self.timer = on;
            unsafe {
                if on {
                    timeBeginPeriod(1);
                } else {
                    timeEndPeriod(1);
                }
            }
        }
    }
}

impl Drop for VigemBackend {
    fn drop(&mut self) {
        self.pad_disconnect();
        self.set_high_res_timer(false);
    }
}
