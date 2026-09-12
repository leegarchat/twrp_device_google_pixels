//! Unified logging for recovery-tensor-daemon.
//!
//! Recovery has no logd, so everything goes to stderr with a stable prefix:
//!   [recovery-tensor-daemon][<tag>][I/E] message
//! `logd!` is compiled out in release builds to keep the recovery log clean.

/// Info-level log to stderr.
#[macro_export]
macro_rules! logi {
    ($tag:expr, $($arg:tt)*) => {{
        eprintln!(
            "[recovery-tensor-daemon][{}][I] {}",
            $tag,
            format!($($arg)*)
        );
    }};
}

/// Error-level log to stderr.
#[macro_export]
macro_rules! loge {
    ($tag:expr, $($arg:tt)*) => {{
        eprintln!(
            "[recovery-tensor-daemon][{}][E] {}",
            $tag,
            format!($($arg)*)
        );
    }};
}

/// Debug-level log, compiled out in release builds.
#[macro_export]
macro_rules! logd {
    ($tag:expr, $($arg:tt)*) => {{
        #[cfg(debug_assertions)]
        eprintln!(
            "[recovery-tensor-daemon][{}][D] {}",
            $tag,
            format!($($arg)*)
        );
        #[cfg(not(debug_assertions))]
        let _ = $tag;
    }};
}
