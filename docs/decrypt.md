> [Русская версия](decrypt_ru.md)

# Data decryption: KeyMint, weaver, vold

`Guide v3 · 2026-10-26 · crypto from source`

Everything is built from source - there are **no prebuilt HALs** in the tree. Stock is needed only as a reference (fstab, cmdline, partition map).

## KeyMint HAL - which one decrypts you

| Family | Type | Why |
|---|---|---|
| `gs101`, `gs201` | `cpp` | C++ HAL against the old Trusty TA |
| `zuma`, `zumapro`, `laguna`, `malibu` | `rust` | Rust HAL against the new TA |

- The type is set by the `keymint` field in `family.json`. The build ships **both** HALs in the universal payload; at install time the swap (`include/aio/aio_swap.sh`, also in the desktop flow) keeps the target family's HAL + VINTF manifest, drops the sibling, and enforces `ro.recovery.keymint=<rust|cpp>`. Whatever HAL remains that doesn't match the Trusty TA exits harmlessly at runtime.
- Both KeyMint modules are built with a separate `mka` **first** (order matters - the recovery callback stages the just-built binaries from `$PRODUCT_OUT`).
- C++ on zuma was tested and **rejected** (restart loop, TA mismatch). Rust-from-source is green (`md5 out==device`, `User 0 Decrypted`).

<details>
<summary>🔌 VINTF manifests - the schema that breaks everything</summary>

- Recovery carries libvintf 8.0; stock schema 9.0 **breaks registration of ALL device HALs** - so KeyMint manifests stay schema 2.0.
- Versions: zuma/zumapro `IKeyMintDevice v5+RPC v3`, gs201 `v4+v3`, malibu/laguna `v5+v3`. The `recovery_available` flag lives in the Soong `.bp` files (KeyMint/keymaster/weaver `Android.bp`), not in the VINTF manifest - it is what lets the service start in recovery.

</details>

<details>
<summary>🔐 Weaver + Titan (`recovery-tensor-daemon`)</summary>

- Rust daemon (`include/recovery-tensor-daemon/`: `common/`, `storageproxy/`, `weaver/`) serves FBE synthetic passwords via weaver slots. Multicall: `storageproxy` (Trusty TIPC + UFS RPMB over SG_IO; RPMB node auto-discovered, cached in `/tmp/.rpmb_sg_dev`) + `weaver` (Titan M2/M3 → `android.hardware.weaver.IWeaver/default`) + `run` (both in parallel threads) + `titan-wipe`.
- **Titan M2** (field mapping, Pixel 6-9: gs101/gs201/zuma/zumapro): protobuf path, untouched. **Titan M3** (field mapping, Pixel 10/11: laguna/malibu): raw protocol (`weaver/m3.rs`, hdr `0x000e0000`) as a latched fallback - the first success pins the path (`service.rs`). Applet geometry (slots, key/value sizes) is **not** hardcoded: learned from the applet's `getConfig` reply, cached, every read/write validated strictly against it.
- `titan-wipe` (Format Data route (b) only): mirrors the stock factory-reset `WipeTitanM` - Nugget `NUKE_FROM_ORBIT` (`APP_ID_NUGGET 0x00`, param `0x0005`, magic `0xc05fefee`), up to 5 attempts, same call sequence as fastbootd's `doOemSpecificErase`. No per-generation branch (M1/M2/M3 share the Nugget protocol; transport already selects `/dev/citadel0` vs `/dev/gsc0` at open). The `--check` firmware-version read is a probe only (Dauntless-prefixed = M2 family), it gates nothing. The caller stops `recovery_weaver` before `--erase` and restarts it after (`/dev/gsc0` serves one caller).
- Vold side: parsing of the packed 5-byte `.weaver` file (slot at offset 1, big-endian) and a `keySize 0→16` fallback (Titan M3 in recovery returns 0; the standard is 16). Covered by unit tests in the daemon.
- No Titan wipe in normal decrypt - `titan-wipe` runs only inside the Format Data route (b), see [diagnostics](diagnostics.md).

</details>

<details>
<summary>💾 Vold multi-device (`MetadataCrypt` + libfstab)</summary>

- Stock vold handles a single userdata device; on newer Pixels it is a zoned + alias set. Our patches add a mirrored parser in `partitionmanager.cpp` (stock `libfstab` untouched - no libfstab triplet in tree): `device=zoned:` / `device=exp:` / `exp_alias:`, the `user_devices` field, dm names and key subdirectories by basename (`zoned_device`), metadata timeout 30→120.
- `partitionmanager.cpp`: `FscryptMountMetadataEncryptedWithTimeout(…, 120)` + gate on KeyMint service state (no service - skip instead of hanging).
- Fstabs: first-stage `vendor_ramdisk` components are **not** built (`FOX_NO_FIRST_STAGE=1`, stock first_stage preserved by the installer); per-family `recovery.fstab` files ship as the `recovery.fstab.<fam>` swap kit (live file is the aio placeholder).

</details>

<details>
<summary>🔋 Battery in recovery - HIDL health HAL</summary>

- Battery is serviced by `android.hardware.health@2.1-service` (generic sysfs-backed, from source, no vendor blobs). It compiles via `PRODUCT_PACKAGES` and is staged into the ramdisk by `fox_build_callback.sh` from `$PRODUCT_OUT` - **not** via `TARGET_RECOVERY_DEVICE_MODULES` (that list silently skips it: no recovery variant upstream). Ordering enforced in `BoardConfig.mk`.
- Legacy sysfs branch is off by purpose (`OF_USE_LEGACY_BATTERY_SERVICES=0`). `flog.sh` now packs a battery + health snapshot into every field log.

</details>

<details>
<summary>🧪 KDF playground (`include/fbe_kdf/`)</summary>

- `fox_fbe_kdf <secret-hex> "<pipeline>"` - research into the FBE synthetic-password KDF: `slice` / `hex` / `unhex` / `ph512` / `ph256` / `xorhalf` / `sp800` stages left to right, first successful line of `system/etc/fox_kdf.conf` wins.
- Override on device without rebuilding: `/tmp/fox_kdf.conf` via adb push. Research tool - **not** the standard decrypt path.

</details>
