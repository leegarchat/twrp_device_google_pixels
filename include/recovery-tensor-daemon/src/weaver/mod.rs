//! Titan M (GSC/Citadel) Weaver proxy.
//!
//! Talks to the secure element over `/dev/gsc0` (`GSC_IOC_GSA_NOS_CALL`,
//! `APP_ID 0x03`, protobuf framing) and publishes the standard
//! `android.hardware.weaver.IWeaver/default` Binder service consumed by
//! Gatekeeper/CE FBE decryption.

mod gsc;
mod m3;
mod proto;
mod service;

/// Runs the weaver service on `dev`. Diverges.
pub fn run(dev: &str) -> ! {
    service::run(dev)
}
