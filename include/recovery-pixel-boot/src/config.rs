//! config — /pixelrunatboot.json reader (std only, no serde).
//!
//! Build-time merged map {codename: {...}} from families/*/family.json +
//! devices/*/pixel.json. Device section: family/soc strings, touch module
//! list, partition base names, haptics sysfs path, props map.
//! The parser below handles exactly this schema: objects with string keys,
//! string values, arrays of strings, nested objects one level deep.

use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct DeviceConfig {    pub family: String,
    pub soc_family: String,
    pub touch_modules: Vec<String>,
    pub part_touch: String,
    pub part_vendor: String,
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
    /// Object whose values are strings, string arrays, or one-level string maps.
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
    })
}

pub fn load_device_config(code: &str) -> Result<DeviceConfig, String> {
    load_device_config_from(Path::new(CONFIG_PATH), code)
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
}
