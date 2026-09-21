//! Weaver HAL logic + Binder service glue.
//!
//! [`WeaverHal`] is transport/pure logic and compiles everywhere (only
//! `libc` via [`super::gsc`]). The AIDL service implementation at the bottom
//! is compiled only with the `binder` cargo feature, enabled by the Soong
//! build (`features: ["binder"]` in Android.bp); without it [`run`] falls
//! back to a headless GSC supervisor suitable for host-side development.
//!
//! Titan M3 readiness: key/value sizes are NOT hardcoded. They are learned
//! from the applet's `getConfig` reply, cached, and every `read`/`write`
//! validates incoming buffers strictly against those dynamic parameters.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::gsc::{GscDevice, GscError, APP_ID_WEAVER, APP_SUCCESS, MAX_GSA_NOS_CALL_TRANSFER};
use super::m3;
use super::proto::{
    self, ProtoError, CMD_GET_CONFIG, CMD_READ, CMD_WRITE,
};

/// Cached applet geometry.
#[cfg_attr(not(feature = "binder"), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
pub struct WeaverGeometry {
    pub slots: u32,
    pub key_size: usize,
    pub value_size: usize,
}

/// Outcome of a `read` applet call.
#[cfg_attr(not(feature = "binder"), allow(dead_code))]
#[derive(Debug)]
pub struct ReadOutcome {
    pub value: Vec<u8>,
    pub throttle_ms: u32,
    pub status: ReadStatus,
}

/// Applet-level read status (error field 0/1/2, else FAILED).
#[cfg_attr(not(feature = "binder"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStatus {
    Ok,
    IncorrectKey,
    Throttle,
    Failed,
}

/// Weaver failure.
#[cfg_attr(not(feature = "binder"), allow(dead_code))]
#[derive(Debug)]
pub enum WeaverError {
    Gsc(GscError),
    Proto(ProtoError),
    /// Titan M3 raw-struct codec failure (see [`super::m3`]).
    M3(&'static str),
    /// Applet rejected the command (`call_status` != APP_SUCCESS).
    Chip(u32),
    BadSlot(i32),
    BadKeySize { expected: usize, got: usize },
    BadValueSize { expected: usize, got: usize },
}

impl std::fmt::Display for WeaverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WeaverError::Gsc(e) => write!(f, "gsc transport: {e}"),
            WeaverError::Proto(e) => write!(f, "protobuf: {e}"),
            WeaverError::M3(e) => write!(f, "titan m3 raw: {e}"),
            WeaverError::Chip(s) => write!(f, "applet call_status=0x{s:x}"),
            WeaverError::BadSlot(s) => write!(f, "invalid slot {s}"),
            WeaverError::BadKeySize { expected, got } => {
                write!(f, "bad key size {got}, expected {expected}")
            }
            WeaverError::BadValueSize { expected, got } => {
                write!(f, "bad value size {got}, expected {expected}")
            }
        }
    }
}

impl std::error::Error for WeaverError {}

impl From<GscError> for WeaverError {
    fn from(e: GscError) -> Self {
        WeaverError::Gsc(e)
    }
}

impl From<ProtoError> for WeaverError {
    fn from(e: ProtoError) -> Self {
        WeaverError::Proto(e)
    }
}

/// Thread-safe Weaver front-end over one [`GscDevice`].
///
/// Titan M3 (malibu) fallback: protobuf is always tried first; a protobuf
/// decode failure on an otherwise accepted applet reply latches raw M3
/// mode for the process (see [`super::m3`]). Titan M behavior is unchanged.
pub struct WeaverHal {
    gsc: GscDevice,
    cached: Mutex<Option<WeaverGeometry>>,
    m3: AtomicBool,
}

// `read`/`write` are only driven by the Binder glue; the headless fallback
// exercises `get_config` alone, so allow the unused paths there.
#[cfg_attr(not(feature = "binder"), allow(dead_code))]
impl WeaverHal {
    pub fn open(dev: &str) -> Result<WeaverHal, WeaverError> {
        Ok(WeaverHal {
            gsc: GscDevice::open(dev)?,
            cached: Mutex::new(None),
            m3: AtomicBool::new(false),
        })
    }

    fn is_m3(&self) -> bool {
        self.m3.load(Ordering::SeqCst)
    }

    fn latch_m3(&self) {
        self.m3.store(true, Ordering::SeqCst);
    }

    fn check_status(status: u32) -> Result<(), WeaverError> {
        if status != APP_SUCCESS {
            return Err(WeaverError::Chip(status));
        }
        Ok(())
    }

    /// Queries the applet geometry and refreshes the cache. Protobuf first;
    /// on a protobuf decode failure the Titan M3 raw layout is tried and,
    /// when its header word validates, M3 mode is latched.
    pub fn get_config(&self) -> Result<WeaverGeometry, WeaverError> {
        if self.is_m3() {
            return self.get_config_m3();
        }
        let (reply, status) =
            self.gsc.nos_call(APP_ID_WEAVER, CMD_GET_CONFIG, &[], 64)?;
        Self::check_status(status)?;
        let wire = match proto::parse_get_config(&reply) {
            Ok(wire) => wire,
            Err(proto_err) => match m3::parse_get_config(&reply) {
                Some(m3geo) => {
                    crate::logi!("weaver", "titan M3 raw protocol detected, latching M3 mode");
                    self.latch_m3();
                    return self.cache_geometry(WeaverGeometry {
                        slots: m3geo.slots,
                        key_size: m3geo.key_size,
                        value_size: m3geo.value_size,
                    });
                }
                None => return Err(WeaverError::Proto(proto_err)),
            },
        };
        let geo = WeaverGeometry {
            slots: wire.slots,
            key_size: wire.key_size as usize,
            value_size: wire.value_size as usize,
        };
        self.cache_geometry(geo)
    }

    /// Raw M3 getConfig (M3 mode already latched).
    fn get_config_m3(&self) -> Result<WeaverGeometry, WeaverError> {
        let (reply, status) =
            self.gsc.nos_call(APP_ID_WEAVER, CMD_GET_CONFIG, &[], 64)?;
        Self::check_status(status)?;
        let m3geo = m3::parse_get_config(&reply).ok_or(WeaverError::M3("bad getConfig reply"))?;
        self.cache_geometry(WeaverGeometry {
            slots: m3geo.slots,
            key_size: m3geo.key_size,
            value_size: m3geo.value_size,
        })
    }

    fn cache_geometry(&self, geo: WeaverGeometry) -> Result<WeaverGeometry, WeaverError> {
        *self.cached.lock().map_err(|_| {
            WeaverError::Gsc(GscError::Unsupported("geometry cache lock poisoned"))
        })? = Some(geo);
        crate::logi!(
            "weaver",
            "getConfig: slots={} keySize={} valueSize={}",
            geo.slots,
            geo.key_size,
            geo.value_size
        );
        Ok(geo)
    }

    /// Returns the cached geometry, querying the applet on first use.
    fn geometry(&self) -> Result<WeaverGeometry, WeaverError> {
        if let Some(geo) = self
            .cached
            .lock()
            .map_err(|_| WeaverError::Gsc(GscError::Unsupported("geometry cache lock poisoned")))?
            .as_ref()
            .copied()
        {
            return Ok(geo);
        }
        self.get_config()
    }

    fn check_slot(&self, geo: &WeaverGeometry, slot: i32) -> Result<u32, WeaverError> {
        if slot < 0 {
            return Err(WeaverError::BadSlot(slot));
        }
        let slot_u = slot as u32;
        if geo.slots > 0 && slot_u >= geo.slots {
            return Err(WeaverError::BadSlot(slot));
        }
        Ok(slot_u)
    }

    /// Reads `slot`, requiring `key.len() == key_size`.
    pub fn read(&self, slot: i32, key: &[u8]) -> Result<ReadOutcome, WeaverError> {
        let geo = self.geometry()?;
        let slot_u = self.check_slot(&geo, slot)?;
        if key.len() != geo.key_size {
            return Err(WeaverError::BadKeySize { expected: geo.key_size, got: key.len() });
        }
        if self.is_m3() {
            return self.read_m3(slot, slot_u, key, &geo);
        }
        // Request/response buffers are sized dynamically: no 64/128-byte
        // stack caps, so larger PQC-era keys/values keep working.
        let req = proto::build_read_request(slot_u, key);

        // Titan M3 adaptive buffer: reply sized from the chip geometry
        // (value_size + protobuf overhead), floored at the proven 128 bytes
        // of the reference C++ daemon, and capped so arg_len + reply_cap
        // never exceeds the GSC 4K transfer window (else EINVAL).
        let max_transfer_reply = MAX_GSA_NOS_CALL_TRANSFER.saturating_sub(req.len());
        let reply_cap = (geo.value_size + 64).max(128).min(max_transfer_reply);

        let (reply, status) =
            self.gsc.nos_call(APP_ID_WEAVER, CMD_READ, &req, reply_cap)?;
        Self::check_status(status)?;
        let (error, throttle_ms, value) = proto::parse_read_response(&reply)?;
        let status = match error {
            0 => ReadStatus::Ok,
            1 => ReadStatus::IncorrectKey,
            2 => ReadStatus::Throttle,
            _ => ReadStatus::Failed,
        };
        crate::logi!(
            "weaver",
            "read slot {slot}: error={error} throttle={throttle_ms} value_len={}",
            value.len()
        );
        Ok(ReadOutcome { value, throttle_ms, status })
    }

    /// Raw M3 read (M3 mode already latched via [`Self::get_config`]).
    fn read_m3(
        &self,
        slot: i32,
        slot_u: u32,
        key: &[u8],
        geo: &WeaverGeometry,
    ) -> Result<ReadOutcome, WeaverError> {
        let req = m3::build_read_request(slot_u, key);
        let max_transfer_reply = MAX_GSA_NOS_CALL_TRANSFER.saturating_sub(req.len());
        let reply_cap = (geo.value_size + 64).max(64).min(max_transfer_reply);
        let (reply, status) =
            self.gsc.nos_call(APP_ID_WEAVER, CMD_READ, &req, reply_cap)?;
        Self::check_status(status)?;
        let (error, throttle_ms, value) = m3::parse_read_response(&reply, geo.value_size)
            .ok_or(WeaverError::M3("bad read reply"))?;
        let status = match error {
            0 => ReadStatus::Ok,
            1 => ReadStatus::IncorrectKey,
            2 => ReadStatus::Throttle,
            _ => ReadStatus::Failed,
        };
        crate::logi!(
            "weaver",
            "read slot {slot}: error={error} throttle={throttle_ms} value_len={}",
            value.len()
        );
        Ok(ReadOutcome { value, throttle_ms, status })
    }

    /// Overwrites `slot`, requiring exact key/value sizes.
    pub fn write(&self, slot: i32, key: &[u8], value: &[u8]) -> Result<(), WeaverError> {
        let geo = self.geometry()?;
        let slot_u = self.check_slot(&geo, slot)?;
        if key.len() != geo.key_size {
            return Err(WeaverError::BadKeySize { expected: geo.key_size, got: key.len() });
        }
        if value.len() != geo.value_size {
            return Err(WeaverError::BadValueSize {
                expected: geo.value_size,
                got: value.len(),
            });
        }
        if self.is_m3() {
            let req = m3::build_write_request(slot_u, key, value);
            let (_, status) = self.gsc.nos_call(APP_ID_WEAVER, CMD_WRITE, &req, 0)?;
            Self::check_status(status)?;
            crate::logi!("weaver", "write slot {slot}: ok");
            return Ok(());
        }
        let req = proto::build_write_request(slot_u, key, value);
        let (_, status) = self.gsc.nos_call(APP_ID_WEAVER, CMD_WRITE, &req, 0)?;
        Self::check_status(status)?;
        crate::logi!("weaver", "write slot {slot}: ok");
        Ok(())
    }
}

// --- Binder service (Soong build only) ---

/// AIDL instance name, identical to the C++ daemon's `descriptor + "/default"`.
#[cfg(feature = "binder")]
const WEAVER_INSTANCE: &str = "android.hardware.weaver.IWeaver/default";
/// Mirrors `IWeaver.STATUS_FAILED`.
#[cfg(feature = "binder")]
const STATUS_FAILED: i32 = 1;

#[cfg(feature = "binder")]
mod binder_glue {
    use super::{ReadStatus, WeaverHal, STATUS_FAILED, WEAVER_INSTANCE};

    use android_hardware_weaver::aidl::android::hardware::weaver::IWeaver::BnWeaver;
    use android_hardware_weaver::aidl::android::hardware::weaver::IWeaver::IWeaver;
    use android_hardware_weaver::aidl::android::hardware::weaver::WeaverConfig::WeaverConfig as AidlConfig;
    use android_hardware_weaver::aidl::android::hardware::weaver::WeaverReadResponse::WeaverReadResponse as AidlResp;
    use android_hardware_weaver::aidl::android::hardware::weaver::WeaverReadStatus::WeaverReadStatus as AidlStatus;

    pub struct BinderWeaver {
        hal: WeaverHal,
    }

    impl binder::Interface for BinderWeaver {}

    impl IWeaver for BinderWeaver {
        fn getConfig(&self) -> binder::Result<AidlConfig> {
            self.hal.get_config().map(|g| AidlConfig {
                slots: g.slots as i32,
                keySize: g.key_size as i32,
                valueSize: g.value_size as i32,
            }).map_err(|e| {
                crate::loge!("weaver", "getConfig failed: {e}");
                binder::Status::new_service_specific_error_str(STATUS_FAILED, Some(format!("{e}")))
            })
        }

        fn read(&self, slot_id: i32, key: &[u8]) -> binder::Result<AidlResp> {
            // Like the C++ daemon, transport/applet failures surface as a
            // FAILED response parcel, not as a binder transport error.
            let failed = |timeout: i64| AidlResp {
                timeout,
                value: Vec::new(),
                status: AidlStatus::FAILED,
            };
            match self.hal.read(slot_id, key) {
                Ok(out) => {
                    let status = match out.status {
                        ReadStatus::Ok => AidlStatus::OK,
                        ReadStatus::IncorrectKey => AidlStatus::INCORRECT_KEY,
                        ReadStatus::Throttle => AidlStatus::THROTTLE,
                        ReadStatus::Failed => AidlStatus::FAILED,
                    };
                    let (timeout, value) = match out.status {
                        ReadStatus::Ok => (0, out.value),
                        _ => (out.throttle_ms as i64, Vec::new()),
                    };
                    Ok(AidlResp { timeout, value, status })
                }
                Err(e) => {
                    crate::loge!("weaver", "read slot {slot_id} failed: {e}");
                    Ok(failed(0))
                }
            }
        }

        fn write(&self, slot_id: i32, key: &[u8], value: &[u8]) -> binder::Result<()> {
            self.hal.write(slot_id, key, value).map_err(|e| {
                crate::loge!("weaver", "write slot {slot_id} failed: {e}");
                binder::Status::new_service_specific_error_str(STATUS_FAILED, Some(format!("{e}")))
            })
        }
    }

    /// Registers the service and joins the Binder thread pool. Diverges.
    pub fn serve(hal: WeaverHal) -> ! {
        // Prefetch geometry so key/value sizes are cached before clients arrive.
        if let Err(e) = hal.get_config() {
            crate::loge!("weaver", "initial getConfig failed (will retry per call): {e}");
        }
        binder::ProcessState::start_thread_pool();
        let service = BnWeaver::new_binder(BinderWeaver { hal }, binder::BinderFeatures::default());
        // Titan M3 bringup: servicemanager can be momentarily unreachable
        // right after boot (observed UNKNOWN_ERROR on malibu). The service
        // is oneshot, so a single fatal exit here kills CE decrypt with no
        // retry — poll instead, then join the pool once registered.
        let mut registered = false;
        for attempt in 0..30u32 {
            match binder::add_service(WEAVER_INSTANCE, service.as_binder()) {
                Ok(()) => {
                    registered = true;
                    break;
                }
                Err(e) => {
                    crate::loge!(
                        "weaver",
                        "register attempt {attempt} failed: {e:?}; retrying"
                    );
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
        if !registered {
            crate::loge!("weaver", "failed to register {WEAVER_INSTANCE} after retries");
            std::process::exit(1);
        }
        crate::logi!("weaver", "registered {WEAVER_INSTANCE}");
        binder::ProcessState::join_thread_pool();
        crate::loge!("weaver", "binder thread pool exited unexpectedly");
        std::process::exit(1);
    }
}

/// Headless supervisor for plain cargo builds: verifies the GSC link and
/// keeps the process alive for manual testing. Diverges.
#[cfg(not(feature = "binder"))]
fn serve_headless(hal: WeaverHal) -> ! {
    match hal.get_config() {
        Ok(g) => crate::logi!(
            "weaver",
            "headless: gsc ok (slots={} keySize={} valueSize={})",
            g.slots,
            g.key_size,
            g.value_size
        ),
        Err(e) => crate::loge!("weaver", "headless: initial getConfig failed: {e}"),
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        if let Err(e) = hal.get_config() {
            crate::loge!("weaver", "headless: periodic getConfig failed: {e}");
        }
    }
}

/// Runs the weaver service on `dev`. Diverges.
pub fn run(dev: &str) -> ! {
    crate::logi!("weaver", "starting on {dev}");
    let hal = match WeaverHal::open(dev) {
        Ok(hal) => hal,
        Err(e) => {
            crate::loge!("weaver", "cannot open {dev}: {e}");
            std::process::exit(1);
        }
    };
    #[cfg(feature = "binder")]
    binder_glue::serve(hal);
    #[cfg(not(feature = "binder"))]
    serve_headless(hal);
}
