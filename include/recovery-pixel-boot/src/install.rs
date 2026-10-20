//! install-rom — thin entry point for the high-speed OTA engine.
//!
//! Called as `recovery-pixel-boot install-rom <fox-hsinstall args…>` by
//! the installer (C++) and by shell stages. Forwards everything to the
//! static `/system/bin/fox-hsinstall` binary (cargo-built, ships via the
//! build callback) and exits with its code, so the caller's fallback
//! contract (2/5 = stock update_engine path) stays intact.
//!
//! The status-fd pipe (when given) is inherited straight through: fds
//! created without O_CLOEXEC survive both execs, no forwarding needed.

use std::process::Command;

const ENGINE: &str = "/system/bin/fox-hsinstall";

/// argv after `install-rom`, forwarded verbatim. Never returns.
pub fn run_install_rom(args: &[String]) -> ! {
    let mut cmd = Command::new(ENGINE);
    cmd.args(args);
    match cmd.status() {
        Ok(st) => std::process::exit(st.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("recovery-pixel-boot install-rom: cannot run {ENGINE}: {e}");
            // Treat a missing engine like missing helpers: stock path.
            std::process::exit(5);
        }
    }
}
