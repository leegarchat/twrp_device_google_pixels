> [Русская версия](decrypt_ru.md)

# Data decryption: keymint, weaver, vold

Everything is built from source — there are no prebuilt HALs in the tree. Stock is
needed only as a reference (fstab, cmdline, partition map).

## KeyMint HAL (per-family, from source)

| Family | Type | Why |
|---|---|---|
| `gs201`, `gs101` | `cpp` | C++ HAL against the old Trusty TA |
| `zuma`, `zumapro`, `laguna`, `malibu` | `rust` | Rust HAL against the new TA |

- The type is set by the `keymint` field in `family.json` → `FOX_KEYMINT_TYPE`
  (read by `build.sh`, `device.mk`, the callback via `.build_platform.conf`).
- The callback injects the required service into rc and disables the foreign one; the keymint module
  is built with a separate `mka` **first** (order matters).
- C++ on zuma was tested and rejected (restart loop, TA mismatch);
  Rust-from-source is green (md5 out==device, `User 0 Decrypted`).
- Keymint VINTF manifests — schema 2.0 only (recovery carries libvintf
  8.0; stock schema 9.0 breaks registration of **all** device HALs).
  Versions: zuma/zumapro `IKeyMintDevice v5+RPC v3`, gs201 `v4+v3`,
  malibu/laguna `v5+v3`. The `recovery_available` field for the Rust HAL
  is preserved (otherwise the service does not start in recovery).
- The Health HAL is not pulled in at all (the battery is not serviced in recovery).

## Weaver and Titan M3 (`recovery-tensor-daemon`)

Rust daemon (`include/recovery-tensor-daemon/`: `common/`, `storageproxy/`,
`weaver/`) serves FBE synthetic passwords via weaver slots.

- Titan M (older SoCs): protobuf path, untouched.
- Titan M3 (newer): raw protocol (`weaver/m3.rs`, hdr `0x000e0000`) as
  a fallback with latching in `service.rs`: the first success pins the path.
- On the vold side: parsing of the packed 5-byte `.weaver` file (slot at
  offset 1, big-endian) and a `keySize 0→16` fallback (Titan M3 in recovery
  returns 0; the standard is 16). Covered by unit tests in the daemon.

## Vold multi-device (`MetadataCrypt` + libfstab)

Stock vold handles a single userdata device; on newer Pixels it is a zoned +
alias set. Our patches:

- libfstab: parsing of `device=zoned:` / `device=exp:` / `exp_alias:`,
  the `user_devices` field in fstab.
- `MetadataCrypt`: multi-device mapping, dm names and key subdirectories by
  basename (`zoned_device`), metadata timeout 30→120.
- `partitionmanager.cpp`: `FscryptMountMetadataEncryptedWithTimeout(…, 120)`
  + gate on keymint service state (no service — skip instead of hanging).
- fstabs: pre-rendered for vendor_ramdisk in `families/<fam>/fstab/`,
  recovery fstab is `recovery.fstab`.

## KDF playground (`fbe_kdf/`)

`fox_fbe_kdf <secret-hex> "<pipeline>"` — research into the FBE
synthetic-password KDF: `slice`/`hex`/`unhex`/`ph512`/`ph256`/
`xorhalf`/`sp800` stages left to right, the first successful line of
`system/etc/fox_kdf.conf` wins. Override on device without
rebuilding: `/tmp/fox_kdf.conf` via adb push. This is a research
tool, not the standard decryption path.
