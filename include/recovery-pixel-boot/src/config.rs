//! config — /pixelrunatboot.json reader (std only, no serde).
//!
//! Build-time merged map {codename: {...}} from families/*/family.json +
//! devices/*/pixel.json. Device section: family/soc strings, touch module
//! list, partition base names, haptics sysfs path, fold flag + display
//! geometry, props map.
//! The parser below handles exactly this schema: objects with string keys;
//! string, number, string-array values; nested objects one level deep.

use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct DeviceConfig {    pub family: String,
    pub soc_family: String,
    pub touch_modules: Vec<String>,
    pub part_touch: String,
    pub part_vendor: String,
    /// Partition holding provider modules that touch/camera drivers depend
    /// on but that live outside vendor_dlkm (e.g. "system_dlkm" for
    /// pwrseq-core on 6.12). Empty = no preload stage.
    pub part_sysdlkm: String,
    /// Provider modules to insmod BEFORE the touch matrix (dependency
    /// order, e.g. ["pwrseq-core"] before lwis). Best-effort: absence on
    /// kernels that need nothing (6.1) only warns.
    pub preload_modules: Vec<String>,
    pub cs40l26_pm: String,
    pub props: Vec<(String, String)>,
    /// Thermal zone `type` names for auto mode (default: Tensor BIG names).
    pub thermal_zone_types: Vec<String>,
    /// Representative (non-hotspot) sensor type, preferred over the auto
    /// list (default "soc_therm", like the Android HAL selection).
    pub thermal_soc_type: String,
    /// Exact temp node; empty = auto discovery.
    pub thermal_temp_path: String,
    /// LM3644 I2C devname suffix (default "-0063").
    pub torch_i2c_match: String,
    /// '|' separated devicetree path matches (default "flash|torch").
    pub torch_pinctrl_match: String,
    /// VBUS sysfs candidates for otg-auto (default: 3 known paths).
    pub vbus_paths: Vec<String>,
    /// TCPC driver dir name for otg-patch (default "max77759tcpc").
    pub tcpc_driver: String,
    /// Fold device flag (default false). When true, init detects the hinge
    /// state and applies front/inner display geometry.
    pub is_fold: bool,
    /// Base (cover/front) display virtual canvas (default 1080x2400).
    pub front_display: DisplayGeom,
    /// Inner display virtual canvas for folds; None when absent/incomplete.
    pub inner_display: Option<DisplayGeom>,
    /// Status-bar area height for the OrangeFox theme (default 130;
    /// zumapro devices use 150). Stamped as DOF_STATUS_H at early-init so
    /// data.cpp can drop the compile-time OF_STATUS_H default.
    pub status_h: u32,
    /// Vertical letterbox for the front/cover canvas (default 0 = stock
    /// vertical-stretch behavior): slabs and fold covers use real panel
    /// geometry with 0; tablets stamp 1 explicitly.
    pub progressive_scale: u32,
    /// Vertical letterbox for the fold inner canvas (default 1): inner
    /// displays use a 16:9 virtual canvas with bars on both axes.
    pub inner_progressive_scale: u32,
}

/// Virtual display canvas (letterbox geometry) in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayGeom {
    pub w: u32,
    pub h: u32,
}

pub const CONFIG_PATH: &str = "/pixelrunatboot.json";

/// Compiled defaults for path-ish keys: used when the JSON omits them,
/// so old configs keep working and new devices only override deltas.
pub fn default_thermal_zone_types() -> Vec<String> {
    ["BIG", "CLUSTER2", "CLUSTER_BIG", "cpu_big", "CPU-Big", "prime", "PRIME"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub fn default_vbus_paths() -> Vec<String> {
    [
        "/sys/class/power_supply/usb/online",
        "/sys/class/power_supply/usb/present",
        "/sys/class/power_supply/usb-charger/online",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[derive(Debug)]
struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

#[derive(Debug, Clone)]
enum Val {
    Str(String),
    Arr(Vec<String>),
    Obj(Vec<(String, Val)>),
    Num(i64),
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&self) -> Result<u8, String> {
        self.b.get(self.i).copied().ok_or_else(|| "unexpected EOF".to_string())
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        if self.peek()? == c {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}'", c as char))
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?;
        let mut s = String::new();
        loop {
            match self.b.get(self.i).copied() {
                None => return Err("unterminated string".to_string()),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(s);
                }
                Some(b'\\') => {
                    self.i += 1;
                    match self.b.get(self.i).copied() {
                        None => return Err("bad escape".to_string()),
                        Some(e) => {
                            self.i += 1;
                            match e {
                                b'"' => s.push('"'),
                                b'\\' => s.push('\\'),
                                b'/' => s.push('/'),
                                b'n' => s.push('\n'),
                                b't' => s.push('\t'),
                                b'r' => s.push('\r'),
                                b'b' => s.push('\x08'),
                                b'f' => s.push('\x0c'),
                                b'u' => {
                                    if self.i + 4 > self.b.len() {
                                        return Err("bad \\u escape".to_string());
                                    }
                                    let hex = std::str::from_utf8(&self.b[self.i..self.i + 4])
                                        .map_err(|_| "bad \\u escape".to_string())?;
                                    let cp = u32::from_str_radix(hex, 16)
                                        .map_err(|_| "bad \\u escape".to_string())?;
                                    s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                                    self.i += 4;
                                }
                                _ => return Err("bad escape".to_string()),
                            }
                        }
                    }
                }
                Some(c) => {
                    self.i += 1;
                    s.push(c as char);
                }
            }
        }
    }
    fn number(&mut self) -> Result<i64, String> {
        let start = self.i;
        if self.peek()? == b'-' {
            self.i += 1;
        }
        let digits = self.i;
        while matches!(self.b.get(self.i).copied(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == digits {
            return Err("bad number".to_string());
        }
        std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| "bad number".to_string())?
            .parse::<i64>()
            .map_err(|_| "bad number".to_string())
    }
    fn array_of_strings(&mut self) -> Result<Vec<String>, String> {
        self.eat(b'[')?;
        let mut out = Vec::new();
        loop {
            self.ws();
            if self.peek()? == b']' {
                self.i += 1;
                return Ok(out);
            }
            out.push(self.string()?);
            self.ws();
            match self.peek()? {
                b',' => {
                    self.i += 1;
                }
                b']' => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => return Err("expected , or ]".to_string()),
            }
        }
    }
    /// Object whose values are strings, string arrays, numbers, or
    /// one-level maps of those (device sections + props + display geometry).
    fn object(&mut self) -> Result<Vec<(String, Val)>, String> {
        self.eat(b'{')?;
        let mut out = Vec::new();
        loop {
            self.ws();
            if self.peek()? == b'}' {
                self.i += 1;
                return Ok(out);
            }
            let k = self.string()?;
            self.ws();
            self.eat(b':')?;
            self.ws();
            let v = match self.peek()? {
                b'"' => Val::Str(self.string()?),
                b'[' => Val::Arr(self.array_of_strings()?),
                // Full recursion: device sections hold arrays AND the props
                // map; non-string props values are filtered at extraction.
                b'{' => Val::Obj(self.object()?),
                b'0'..=b'9' | b'-' => Val::Num(self.number()?),
                _ => return Err(format!("bad value for key {k}")),
            };
            out.push((k, v));
            self.ws();
            match self.peek()? {
                b',' => {
                    self.i += 1;
                }
                b'}' => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => return Err("expected , or }}".to_string()),
            }
        }
    }
}

fn get_str(pairs: &[(String, Val)], key: &str) -> String {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| match v {
            Val::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn get_arr(pairs: &[(String, Val)], key: &str) -> Vec<String> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| match v {
            Val::Arr(a) => Some(a.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn get_int(pairs: &[(String, Val)], key: &str) -> i64 {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| match v {
            Val::Num(n) => Some(*n),
            Val::Str(s) => s.parse::<i64>().ok(),
            _ => None,
        })
        .unwrap_or_default()
}

/// True when the section defines `key` at all (distinguishes an explicit
/// 0 from a missing key for the progressive-scale flags).
fn has_key(pairs: &[(String, Val)], key: &str) -> bool {
    pairs.iter().any(|(k, _)| k == key)
}

fn get_obj(pairs: &[(String, Val)], key: &str) -> Vec<(String, Val)> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| match v {
            Val::Obj(o) => Some(o.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Display geometry from a {"w": N, "h": M} object; None when incomplete.
fn get_display_geom(pairs: &[(String, Val)], key: &str) -> Option<DisplayGeom> {
    let obj = get_obj(pairs, key);
    if obj.is_empty() {
        return None;
    }
    let w = get_int(&obj, "w");
    let h = get_int(&obj, "h");
    if w <= 0 || h <= 0 {
        return None;
    }
    Some(DisplayGeom { w: w as u32, h: h as u32 })
}

/// Load one device section from a config file.
pub fn load_device_config_from(path: &Path, code: &str) -> Result<DeviceConfig, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
    };
    p.ws();
    let top = p.object()?;
    p.ws();
    if p.i != p.b.len() {
        return Err("trailing data after top-level object".to_string());
    }
    let (_, sec) = top
        .iter()
        .find(|(k, _)| k == code)
        .ok_or_else(|| format!("no section for device {code}"))?;
    let pairs = match sec {
        Val::Obj(o) => o,
        _ => return Err(format!("bad section for device {code}")),
    };
    let props = pairs
        .iter()
        .find(|(k, _)| k == "props")
        .and_then(|(_, v)| match v {
            Val::Obj(o) => Some(
                o.iter()
                    .filter_map(|(k, v)| match v {
                        Val::Str(s) => Some((k.clone(), s.clone())),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    Ok(DeviceConfig {
        family: get_str(pairs, "family"),
        soc_family: get_str(pairs, "soc_family"),
        touch_modules: get_arr(pairs, "touch_modules"),
        part_touch: get_str(pairs, "part_touch"),
        part_vendor: get_str(pairs, "part_vendor"),
        part_sysdlkm: get_str(pairs, "part_sysdlkm"),
        preload_modules: get_arr(pairs, "preload_modules"),
        cs40l26_pm: get_str(pairs, "cs40l26_pm"),
        props,
        thermal_zone_types: {
            let v = get_arr(pairs, "thermal_zone_types");
            if v.is_empty() {
                default_thermal_zone_types()
            } else {
                v
            }
        },
        thermal_soc_type: {
            let v = get_str(pairs, "thermal_soc_type");
            if v.is_empty() {
                "soc_therm".to_string()
            } else {
                v
            }
        },
        thermal_temp_path: get_str(pairs, "thermal_temp_path"),
        torch_i2c_match: {
            let v = get_str(pairs, "torch_i2c_match");
            if v.is_empty() {
                "-0063".to_string()
            } else {
                v
            }
        },
        torch_pinctrl_match: {
            let v = get_str(pairs, "torch_pinctrl_match");
            if v.is_empty() {
                "flash|torch".to_string()
            } else {
                v
            }
        },
        vbus_paths: {
            let v = get_arr(pairs, "vbus_paths");
            if v.is_empty() {
                default_vbus_paths()
            } else {
                v
            }
        },
        tcpc_driver: {
            let v = get_str(pairs, "tcpc_driver");
            if v.is_empty() {
                "max77759tcpc".to_string()
            } else {
                v
            }
        },
        is_fold: get_int(pairs, "is_fold") != 0,
        front_display: get_display_geom(pairs, "front_display")
            .unwrap_or(DisplayGeom { w: 1080, h: 2400 }),
        inner_display: get_display_geom(pairs, "inner_display"),
        status_h: {
            let v = get_int(pairs, "status_h");
            if v > 0 { v as u32 } else { 130 }
        },
        progressive_scale: if has_key(pairs, "progressive_scale")
            && get_int(pairs, "progressive_scale") == 1
        {
            1
        } else {
            0
        },
        inner_progressive_scale: if has_key(pairs, "inner_progressive_scale")
            && get_int(pairs, "inner_progressive_scale") == 0
        {
            0
        } else {
            1
        },
    })
}

pub fn load_device_config(code: &str) -> Result<DeviceConfig, String> {
    load_device_config_from(Path::new(CONFIG_PATH), code)
}

/// Family-level fallback for SoC-named hardware.
///
/// Tensor G6 bootloaders report the SoC ("malibu") instead of the
/// codename ("grizzly") in androidboot.hardware, and no per-device
/// section exists for a family name. Exact section first; otherwise the
/// first section whose `family` matches `code` (touch/display geometry
/// may be approximate for the exact unit, but fstab/flags/USB/keymint
/// are family-exact — enough for decrypt + ADB + backup).
/// Path-parameterized core of [`load_device_config_fallback`] (testable).
fn load_device_config_fallback_from(path: &Path, code: &str) -> Result<DeviceConfig, String> {
    match load_device_config_from(path, code) {
        Ok(c) => Ok(c),
        Err(_) => {
            if code.is_empty() {
                return Err("empty device code".to_string());
            }
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            let mut p = Parser { b: text.as_bytes(), i: 0 };
            p.ws();
            let top = p.object().map_err(|e| e.to_string())?;
            for (dev, _) in top.iter().filter(|(k, _)| !k.starts_with('_')) {
                if let Ok(c) = load_device_config_from(path, dev) {
                    if c.family == code {
                        return Ok(c);
                    }
                }
            }
            Err(format!("no section for device {code}"))
        }
    }
}

pub fn load_device_config_fallback(code: &str) -> Result<DeviceConfig, String> {
    load_device_config_fallback_from(Path::new(CONFIG_PATH), code)
}



/// Top-level device keys (skips `_families` bookkeeping).
pub fn list_devices() -> Vec<String> {
    let text = match std::fs::read_to_string(CONFIG_PATH) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
    };
    p.ws();
    match p.object() {
        Ok(top) => top
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| !k.starts_with('_'))
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn device_section_exists(code: &str) -> bool {
    !code.is_empty() && list_devices().iter().any(|k| k == code)
}

/// Authoritative device identity for flags/modules/tables.
///
/// `ro.hardware` comes from the bootloader/first-stage and may be
/// family-level; product-level props (set by our own props-apply or stock)
/// name the exact codename. First non-empty value with a config section
/// wins; otherwise the raw hardware value (callers treat unknown as skip).
pub fn resolve_device_code() -> String {
    for key in ["ro.product.device", "ro.product.name", "ro.hardware"] {
        let v = crate::props::get_prop(key);
        if !v.is_empty() && device_section_exists(&v) {
            return v;
        }
    }
    crate::props::get_prop("ro.hardware")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "shiba": {
        "family": "zuma",
        "soc_family": "zuma",
        "touch_modules": ["sec_touch", "ftm5"],
        "part_touch": "vendor_dlkm",
        "part_vendor": "vendor",
        "cs40l26_pm": "/sys/x/power/control",
        "props": {"ro.product.model": "Pixel 8", "weird": "a=b c\"d\\e\nf"}
      }
    }"#;

    fn load_fixture() -> DeviceConfig {
        // Unique dir per CALLER (tests run in parallel threads): embed a
        // per-call nonce so concurrent fixtures never share a directory.
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let n = NONCE.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "fox_test_cfg_{}_{}_{:?}",
            std::process::id(),
            n,
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("pixelrunatboot.json");
        std::fs::write(&f, FIXTURE).unwrap();
        let c = load_device_config_from(&f, "shiba").unwrap();
        let _ = std::fs::remove_dir_all(&d);
        c
    }

    #[test]
    fn section_parses() {
        let c = load_fixture();
        assert_eq!(c.family, "zuma");
        assert_eq!(c.touch_modules, vec!["sec_touch", "ftm5"]);
        assert_eq!(c.part_touch, "vendor_dlkm");
        assert_eq!(c.cs40l26_pm, "/sys/x/power/control");
        assert_eq!(c.props.len(), 2);
        assert_eq!(
            c.props.iter().find(|(k, _)| k == "ro.product.model").unwrap().1,
            "Pixel 8"
        );
    }

    #[test]
    fn numbers_and_display_geometry() {
        let d = std::env::temp_dir().join(format!("fox_test_num_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("c.json");
        std::fs::write(
            &f,
            r#"{"comet": {
              "family": "zumapro",
              "is_fold": 1,
              "front_display": {"w": 1080, "h": 2424},
              "inner_display": {"w": 2076, "h": 2152},
              "props": {}
            },
            "shiba": {
              "family": "zuma",
              "is_fold": 0,
              "front_display": {"w": 1080, "h": 2400},
              "props": {}
            },
            "lynx": {"family": "gs201", "props": {}}}"#,
        )
        .unwrap();
        let c = load_device_config_from(&f, "comet").unwrap();
        assert!(c.is_fold);
        assert_eq!(c.front_display, DisplayGeom { w: 1080, h: 2424 });
        assert_eq!(c.inner_display, Some(DisplayGeom { w: 2076, h: 2152 }));
        let s = load_device_config_from(&f, "shiba").unwrap();
        assert!(!s.is_fold);
        assert_eq!(s.front_display, DisplayGeom { w: 1080, h: 2400 });
        assert_eq!(s.inner_display, None);
        // Legacy file without the new keys: defaults, still parses.
        let l = load_device_config_from(&f, "lynx").unwrap();
        assert!(!l.is_fold);
        assert_eq!(l.front_display, DisplayGeom { w: 1080, h: 2400 });
        assert_eq!(l.inner_display, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn status_h_parses_with_default() {
        let d = std::env::temp_dir().join(format!("fox_test_statush_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("c.json");
        std::fs::write(
            &f,
            r#"{"comet": {"family": "zumapro", "status_h": 150, "props": {}},
            "shiba": {"family": "zuma", "props": {}}}"#,
        )
        .unwrap();
        assert_eq!(load_device_config_from(&f, "comet").unwrap().status_h, 150);
        // Missing key keeps the legacy compile-time default (130).
        assert_eq!(load_device_config_from(&f, "shiba").unwrap().status_h, 130);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn progressive_scale_parses_with_defaults() {
        let d = std::env::temp_dir().join(format!("fox_test_pscale_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("c.json");
        std::fs::write(
            &f,
            r#"{"shiba": {"family": "zuma", "progressive_scale": 0, "props": {}},
            "comet": {"family": "zumapro", "progressive_scale": 1, "inner_progressive_scale": 1, "props": {}},
            "lynx": {"family": "gs201", "props": {}}}"#,
        )
        .unwrap();
        // Explicit values win.
        assert_eq!(load_device_config_from(&f, "shiba").unwrap().progressive_scale, 0);
        assert_eq!(load_device_config_from(&f, "comet").unwrap().progressive_scale, 1);
        assert_eq!(load_device_config_from(&f, "comet").unwrap().inner_progressive_scale, 1);
        // Missing keys: front defaults to stock stretch (0), inner to bars (1).
        assert_eq!(load_device_config_from(&f, "lynx").unwrap().progressive_scale, 0);
        assert_eq!(load_device_config_from(&f, "lynx").unwrap().inner_progressive_scale, 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn escapes_round_trip() {
        let c = load_fixture();
        let w = c.props.iter().find(|(k, _)| k == "weird").unwrap().1.clone();
        assert_eq!(w, "a=b c\"d\\e\nf");
    }

    #[test]
    fn unknown_device_errors() {
        let d = std::env::temp_dir().join(format!("fox_test_cfg2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("c.json");
        std::fs::write(&f, FIXTURE).unwrap();
        assert!(load_device_config_from(&f, "nope").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resolve_with_empty_props_falls_back() {
        // Host bionic stubs return "" for every prop: resolve returns the
        // raw (empty) hardware value without panicking; callers skip.
        assert_eq!(resolve_device_code(), "");
        assert!(!device_section_exists(""));
    }

    #[test]
    fn malformed_errors() {
        let d = std::env::temp_dir().join(format!("fox_test_cfg3_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (i, bad) in ["{".to_string(), "{\"a\":}".to_string(), "{\"a\": [1]}".to_string()]
            .iter()
            .enumerate()
        {
            let f = d.join(format!("b{i}.json"));
            std::fs::write(&f, bad).unwrap();
            assert!(load_device_config_from(&f, "a").is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn family_name_falls_back_to_first_family_section() {
        let d = std::env::temp_dir().join(format!("fox_test_famfb_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("c.json");
        std::fs::write(
            &f,
            r#"{"grizzly": {"family": "malibu", "props": {}},
            "shiba": {"family": "zuma", "props": {}}}"#,
        )
        .unwrap();
        // Exact codename still wins.
        assert_eq!(load_device_config_fallback_from(&f, "grizzly").unwrap().family, "malibu");
        // SoC name borrows the family's first section.
        assert_eq!(load_device_config_fallback_from(&f, "malibu").unwrap().family, "malibu");
        // Unknown stays an error; empty stays an error.
        assert!(load_device_config_fallback_from(&f, "nope").is_err());
        assert!(load_device_config_fallback_from(&f, "").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
