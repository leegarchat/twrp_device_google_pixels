//! payload-hs: local-file OTA payload probe + parallel fetch.
//! Mirrors tools/payload_range_fetch.py wire logic, no network, no C code.

pub mod extract;
pub mod layout;
pub mod manifest;
pub mod pb;
pub mod zipx;
