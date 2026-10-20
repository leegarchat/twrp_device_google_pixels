//! fox-hsinstall: high-speed OTA installer engine.
//!
//! Exit codes (contract with the recovery installer):
//!   0 = success (slot NOT switched here; caller does bootctl)
//!   1 = install FAILED, rollback of critical partitions done
//!   2 = not a full OTA -> caller must take the stock update_engine path
//!   3 = IMPOSSIBLE fit -> hard error, nothing was written
//!   4 = DEVICE MISMATCH (pre-device vs ro.product.device) -> hard error
//!   5 = helpers missing (siw/lptools_new) -> caller takes stock path
//!   6 = broken invocation (usage) -> hard error, never a fallback
//!
//! Progress protocol (same pipe language the recovery installer parses):
//!   progress <frac> <secs> / set_progress <frac> / ui_print <text>

pub mod backup;
pub mod device;
pub mod live;
pub mod ota;
pub mod rebuild;
pub mod run;
pub mod weights;

use std::io::Write;

pub const EXIT_FAIL: i32 = 1;
pub const EXIT_NOT_FULL: i32 = 2;
pub const EXIT_IMPOSSIBLE: i32 = 3;
pub const EXIT_DEVICE_MISMATCH: i32 = 4;
pub const EXIT_NO_HELPERS: i32 = 5;

/// Line sink: status fd when given (--status-fd N, inherited pipe),
/// stdout otherwise. Mirrors the updater/recovery wire protocol.
pub struct Out {
    f: std::fs::File,
}

impl Out {
    pub fn new(status_fd: Option<i32>) -> Self {
        let f = match status_fd {
            Some(n) => {
                use std::os::unix::io::FromRawFd;
                unsafe { std::fs::File::from_raw_fd(n) }
            }
            None => {
                use std::os::unix::io::FromRawFd;
                unsafe { std::fs::File::from_raw_fd(1) }
            }
        };
        Out { f }
    }
    pub fn line(&mut self, s: &str) {
        let _ = writeln!(self.f, "{s}");
        let _ = self.f.flush();
    }
    pub fn ui_print(&mut self, s: &str) {
        self.line(&format!("ui_print {s}"));
    }
    pub fn set_progress(&mut self, frac: f64) {
        self.line(&format!("set_progress {frac:.4}"));
    }
}
