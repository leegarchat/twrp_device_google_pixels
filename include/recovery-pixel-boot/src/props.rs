//! props — getprop/setprop without forking.
//!
//! Reads via bionic `__system_property_get`, writes via
//! `__system_property_set`. `ro.*` keys are read-only through the API;
//! use the `pixelrunatboot.sh props-apply` stage (resetprop) for those.

use std::ffi::CString;

#[cfg(target_os = "android")]
extern "C" {
    fn __system_property_get(name: *const libc::c_char, value: *mut libc::c_char) -> libc::c_int;
    fn __system_property_set(key: *const libc::c_char, value: *const libc::c_char) -> libc::c_int;
}

// Host stubs so `cargo build/test` works on glibc (device uses bionic).
#[cfg(not(target_os = "android"))]
unsafe fn __system_property_get(_name: *const libc::c_char, _value: *mut libc::c_char) -> libc::c_int {
    0
}
#[cfg(not(target_os = "android"))]
unsafe fn __system_property_set(_key: *const libc::c_char, _value: *const libc::c_char) -> libc::c_int {
    -1
}

/// PROP_VALUE_MAX == 92 (incl. NUL).
pub fn get_prop(key: &str) -> String {
    let c_key = match CString::new(key) {
        Ok(k) => k,
        Err(_) => return String::new(),
    };
    let mut buf = [0 as libc::c_char; 92];
    // SAFETY: buf is a valid 92-byte out-param; bionic NUL-terminates.
    let len = unsafe { __system_property_get(c_key.as_ptr(), buf.as_mut_ptr()) };
    if len > 0 {
        // SAFETY: CStr reads from the same NUL-terminated buffer the call
        // just filled; no c_char signedness casts on any target.
        let bytes = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_bytes();
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        String::new()
    }
}

/// Set a non-ro property. ro.* writes are rejected: route them through the
/// props-apply stage instead of forking resetprop from Rust.
pub fn set_prop(key: &str, val: &str) -> Result<(), String> {
    if key.starts_with("ro.") {
        return Err(format!("{key} is read-only via API (use props-apply stage)"));
    }
    let c_key = CString::new(key).map_err(|_| "invalid key".to_string())?;
    let c_val = CString::new(val).map_err(|_| "invalid value".to_string())?;
    // SAFETY: both pointers reference live CStrings for the duration of the
    // call; __system_property_set copies the data synchronously.
    let ret = unsafe { __system_property_set(c_key.as_ptr(), c_val.as_ptr()) };
    if ret != 0 {
        return Err(format!(
            "__system_property_set failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
