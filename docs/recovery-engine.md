> [Русская версия](recovery-engine_ru.md)

# Rust engine `recovery-pixel-boot`

`Guide v3 · 2026-10-26 · include/recovery-pixel-boot/`

One static multicall binary (only `std` + `libc`) that does the whole device bring-up. Iron rule: **Rust never execs external binaries except via the single choke point** `stage.rs` invoking `pixelrunatboot.sh` stages (where `resetprop`, `bootctl`, `siw`/`iw`, `unzip`, `mount`, `setenforce` live). The one `fork()` outside stages is the Magisk-link mirror daemon in `boot.rs` (no exec). Protocol: argv stage + args, results on stdout, diagnostics in `/tmp/recovery.log`, exit 0 = success. Every engine line is also mirrored to `/dev/kmsg` (`ko_picker::log_msg` → recovery.log + dmesg).

## `init` - device identity (early-init exec)

Runs **before** TWRP latches geometry (`twrp.cpp SetDefaultValues` / `data.cpp`) and before minui opens DRM, so the first frame is already correct. Log: `/dev/logs/runatinit.log`.

1. Log kernel (`/proc/sys/kernel/osrelease`), `ro.hardware`, slot suffix.
2. Resolve device code - order `ro.product.device` → `ro.product.name` → `ro.hardware` (`resolve_device_code`). Family-level hardware (`"malibu"`) borrows the first section of its family so the swap below still fires.
3. `props-apply <family> k=v...` stage (`resetprop` + `setenforce 0`; pairs from `/pixelrunatboot.json`, family-merged at build time).
4. Re-resolve the code - applied props may have corrected `ro.product.device` (bootloader `ro.hardware` can be family-level). Only the flags swap below uses the corrected code.
5. **AIO second-pass family swap** (the C stub `aioswap.c` is the first pass): any live file still carrying the `AIO LIVE placeholder` marker is replaced from its kit - `/system/etc/recovery.fstab.<fam>`, `/system/etc/twrp.flags.<fam>`, `/system/etc/recovery.wipe.<fam>`. Idempotent per file (`already-swapped` / `swapped` / `no-kit` tokens in the log). The swap-kit manifest lives in `/system/etc/aio/families.txt` (`<fam>:keymint=:usbctrl=:usbpath=`), the device map in `/system/etc/aio/devices.txt`.
6. Display geometry: hinge probe (EV_SW `SW_LID`/`SW_TABLET_MODE` over `/dev/input/event*`; unknown/absent = safe `ClosedCover`), then `DOF_SCREEN_W/H` + `DOF_PROGRESSIVE_SCALE` (+ `DOF_THEME` when `wide_theme`/`inner_theme` set, + `DOF_STATUS_H` always, + `DOF_MAX_BRIGHTNESS` when `max_brightness` > 0). Plain `setprop` - no `resetprop` fork needed (not `ro.*`).
7. Device override: `/system/etc/<device>.twrp.flags` (from `devices/<codename>/`, optional - none ship today) wins over the family default.
8. `fix_twrp_flags`: (a) UFS platform fallback - repoints baked `/dev/block/platform/<addr>.ufs/` prefixes absent on this device to the live `*ufs*` dir (existing dirs never rewritten); (b) by-name prune - drops lines whose partition exists neither as `<part>` nor `<part>_a` under `/dev/block/platform/*/by-name` (`symlink_metadata`: by-name entries dangle while ueventd coldboot runs; skipped entirely when by-name isn't populated yet).
9. `Magisk-*.zip` scan in `/system/bin` → `magiskboot-unpack` stage (or `no Magisk zip, skipping extraction` warning).
10. `setprop servicemanager.ready true`. (No zip-restore step: the solid UCOMP02 cluster ingests `*.zip` at build time, `lgz decompress` restores them; the old `/lgz_zip_manifest.txt` flow is dead.)

<details>
<summary>⚙️ Other subcommands - what runs when</summary>

| Subcommand | Trigger | What it does |
|---|---|---|
| `boot` | `exec` on `on boot` (init waits - modules before GUI) | see `boot` order below |
| `otg-patch` | service `otg_enable` | USB-gate check first (blocked → `sys.usb.patch_dwc3=0`, device mode); then per-family patch, sets `sys.usb.patch_dwc3=1` |
| `otg-auto` | `patch_dwc3=1` trigger | Per-family VBUS daemon, never returns; gate re-checked (blocked → parks on an hourly sleep, device mode) |
| `setup-temp` | `exec` on `on init` + refresh from `boot` | `/dev/thermal_cpu` symlink: exact path → `soc_therm` → auto list → zone0 |
| `torch on\|off` | `OF_FL_PATH1=cmd:/system/bin/recovery-pixel-boot torch` from GUI (`vendorsetup.sh:171`) | LM3644: registered-I2C client discovery (`-0063` suffix, `I2C_SLAVE_FORCE` via `i2c_transact`) → generic LWIS DT fallback (`google,lwis-i2c-device`, e.g. malibu/laguna and gs101/raven when GPIO discovery misses) → GPIO v1 uAPI; native ioctls, no forks (the `USBSW 0x93←0x09` write belongs to the TCPC patch path in `usb`/`i2c.rs`, not torch) |
| `dm-state` | debugging | Refresh `/tmp/fox_dm_state` once + print |
| `dm-watch` | daemon from init rc | Snapshot-only dm truth watch (never maps/unmaps) |
| `install-rom <args>` | kept alias (thin wrapper, `install.rs`) | `twinstall` execs `/system/bin/fox-hsinstall` directly, not via this alias (status-fd pipe inherited); exit 2/5 keep the stock-path contract |

</details>

<details>
<summary>🚀 `boot` order - the full sequence (boot.rs)</summary>

1. Busybox dumped to `/dev/.fox_bb/busybox` (applets survive package-manager replaces); `chmod 777` everything in `/system/bin`.
2. Slot detect: `ro.boot.slot_suffix`, fallback `slot-detect` stage (`bootctl get-current-slot` → suffix, `0`→`_a` / `1`→`_b`).
3. `susfs_fix`: `ko_try_load("susfs_rename_fix", /proc/susfs_rename_fix)` - fast-symlink rename panic fix (proc trace = success, monolith = ignore).
4. Device config load (unknown codename = empty, skip like the old `*` branch, never fatal) + firmware and touch, both gated on `touch_modules` non-empty: `fw-fetch <part_vendor> <sfx> <slot>` (siw|iw, both slots), then **touch** (`boot.rs` single gate).
5. Touch, 5 sources in order, strictly sequential (concurrent ko-fetch streams would clobber shared `/dev/ko_stage` + `/dev/stage_*.img`): `touch_unpack` (`/<code>.lgz` from the payload → populates the OFOX prebuilt dir) → stock `vendor_dlkm` current slot → opposite slot → `/system/modules_touch` fallback → live `/lib/modules` (exact-vermagic, bypasses first-stage init blocklist via direct `finit_module`) → OFOX-supplied prebuilt (`/system/lib64/modules/touch`, best score first). Provider preload (`part_sysdlkm` + `preload_modules`, e.g. `pwrseq-core` before `lwis`) runs first when configured - no device configures it today, the path only warns.
6. Force fallback: strict `finit_module(flags=0)` everywhere, except one `IGNORE_VERMAGIC/MODVERSIONS` (`0x3`) pass over the best OFOX-supplied candidate on total strict failure (own `.ko` only - kprobe/procfs ABI-stable across 6.1.x; **never** for stock vendor modules). `EEXIST` = success everywhere.
7. Verdict: total miss on every source raises `fox.force_hw_gui=1` (GUI polls it via the `fox.force_hw_gui` hook, switches to VOL+/VOL-/POWER keys + modal - `setprop`, never the ORS FIFO) - **but only on proven-modular trees** (cached monolith verdict). Monolithic/partial (Sultan-style) tolerates the miss: drivers may be built in under another name.
8. RAM diet: `prune_touch_modules` drops unpacked snapshots this boot did NOT inject (keeps the `/<code>.lgz` archive - reflash re-appends it), then thermal refresh (`setup_temp` re-run: zone topology changed once `vendor_dlkm` drivers landed).
9. `usb_modules` (11 devices: gs101/laguna/malibu sets; zuma/gs201/zumapro need nothing - the Samsung stack autoloads from the ramdisk): same ko-fetch mechanism from `part_usb` (default `vendor_dlkm`); miss degrades OTG to device mode with a warning.
10. Backlight nudge: any `/sys/class/backlight/*` node reading below 10% of max is raised to 50% (late panel probe vs TWRP one-shot discovery; no-op when TWRP already set a sane value).
11. dm truth: `ensure_keep_mapped(part_touch, sfx, slot)` leaves the kept LP node (`vendor_dlkm`, `vendor`) for TWRP + `dm::refresh("boot")` publishes `/tmp/fox_dm_state`.
12. Torch node note (`/dev/lwis-flash-lm3644` present or LWIS-DT direct on malibu/laguna), `meta-fix` stage (`/metadata/ota` cleanup), Magisk link: `Magisk-*.zip` copied to `/FFiles/OF_Magisk/` (+ `/sdcard/Fox/FoxFiles/` now and forever via a forked daemon every 2 s once `/data` mounts).

</details>

<details>
<summary>🎯 `.ko` scoring (ko_picker.rs) - the formula</summary>

- Environment from `uname -r` (`6.1.157-android14-11` → major/minor/patch) + pagesize via `sysconf` + Android API from uname or SDK prop (`33`→13 … `36`→16).
- Filename + dir tokens: `android<N>`, `X.Y`, full `X.Y.Z`, bare patch dir (`touch/6.1/157/…`) only after an `X.Y` level was seen in path order.
- **Hard filter:** pagesize - `16k` candidate on 4K kernel (or reverse) is rejected outright.
- Score: kernel-branch match **1000** (mismatch 10, last resort) + patch exact **300** (else proximity ≤200, nearer wins) + API **200** + file/dir consistency **50** + `4k`/`16k` dir **50**. Patch mismatch is never a reject (6.1.x cross-loads field-proven: `.145` module on `.157` kernel).
- Presence = `/proc/modules` **or** `/sys/module` (built-ins never appear in the former); dash/underscore spellings unify. Loading is always raw `finit_module` syscall (bionic has no wrapper at API 34).
- **Monolith, 3 levels, cached per boot:** no `/proc/modules` (true monolith) → empty lsmod → canary probe: `empty_checker_module` from `/system/lib64/modules/probe/` (same kver/android/pagesize layout), verdict = the **trace** `/proc/empty_check_gki`, not the return code (Sultan fakes RC=0 and empty lsmod). Custom modules (shims, susfs) are ignored on monolith; the touch branch tolerates misses there.
- Per-boot injection record `/tmp/fox_ko_loaded` (deduped, dash/underscore unified) - dependents gate on "already injected" without re-running the chain.

</details>

<details>
<summary>🔌 USB - two mechanisms, one predicate (usb/)</summary>

- Family identity: `ro.recovery.soc_family` (stamped at early-init) wins, `ro.hardware` codename table is the fallback (`aio`/empty = `Unknown` → stub path, never force host).
- **Exynos generations** (zuma/gs101/gs201/zumapro): `otg_host_shim` (kprobe on `dwc3_otg_host_ready`, replaces the absent AoC probe) + `OTG_ID` + `CHARGER_MODE` VBUS force, VBUS daemon.
- **Google-silicon generations** (laguna/malibu): `aoc_vote_shim` (`USB_DR_EL` election `cast_vote("AOC",1,1)` - no vendor staging anywhere); the election self-switches, the daemon only keeps the vote + verifies + maintains `/dev/block/otg-usb`.
- **Success predicate** (never role alone): downstream `role == host` **and** a source power_supply `online == 1` **and** `/dev/block/otg-usb` resolving.
- `zuma` is the lean TEST-branch (R11 algorithm): 6.1 = kprobe shim + force-load (prebuilt stamped 6.1.176 vs 6.1.157 device kernel - own-shim exception); 6.12 = native `usb_role_switch` path, no shim by design (the AOC check the shim patched is simply gone there). Details that stayed from field experience: UDC detach with `"\n"` (0-byte writes never reach configfs), `ffs.ready` written once, explicit UDC rebind on the device switch (init triggers are edge-based), stable `/dev/block/otg-usb` symlink, `usb power_supply` = VBUS sensor, `CHARGER_MODE` = gvotable election.
- `laguna` (Tensor G5): UDC unbind + role-switch `host` + gvotable `CHARGER_MODE` VBUS force, VBUS-gated daemon. `google-role-sw` election is observed but **not** depended on (`role` sticks while Type-C `data_role` stays `[device]` - decorative). Field-proven on mustang CP3A: 6.6 stable + 6.12 beta, same manual path. Fox (G5+): the port controller is SPMI (`spmi-max77759tcpc`), not I2C - the I2C scan misses by design, the stock driver self-manages the data path, recognizing the probed SPMI device counts as success.
- `malibu` (Tensor G6): same manual path + `aoc_vote_shim` (`USB_DR_EL` vote, asserted once `/proc/aoc_vote_ready=1`); DWC3 controller entry preferred (`a200000.usb3-role-switch`). No I2C TCPC patch (SPMI bus).
- Failure mode everywhere: well-behaved device (adb safe), never a pretend-host.

</details>

<details>
<summary>🛡️ USB guard - why host mode sometimes stays off (usb/gate.rs)</summary>

- A newer-than-firmware kernel **panics** when the custom OTG stack force-switches VBUS to host (seen in the field). Before any shim load or power write, the gate compares the running kernel's patch level against firmware modules' `vermagic` triplets read from raw `.ko` bytes (1 MiB cap, `vermagic=` needle - the kernel never reports what a loaded module was built for), majority vote over ≤32 modules (stock trees carry strays, e.g. one 6.1.75 among 6.1.157).
- **Empty lsmod** (nothing version-sensitive in play) = vacuous, proceed. **Monolithic kernel** (no module syscalls) = nothing to inject, proceed. **No readable stock `.ko`** = cannot prove foolishness - proceed with a loud note (fail-open). **Majority firmware patch == running patch** = proceed. Otherwise **BLOCK**: no services, no VBUS writes - plain device mode (adb safe).
- Scan order: gs101 `/lib/modules` first (first-stage unpacked the `vendor_boot` dlkm fragment), others `vendor_dlkm` first; `/system/**` is never scanned (own shims are cross-version by design). Re-checked in `otg-auto`: a hand-set `patch_dwc3=1` on a gated tree parks the daemon instead of driving VBUS.
- The gate reads patch triplets off disk; it complements (never replaces) the loader's own `finit_module` verdicts.

</details>

<details>
<summary>🔥🌡️ Torch + thermal + dm truth</summary>

- **Torch** (`torch.rs`, `i2c.rs`): fully native, no `gpioset`/`i2cset` forks. (1) registered I2C client (`/sys/bus/i2c/devices/*<match>`, default `-0063`, `I2C_SLAVE_FORCE`, `USBSW 0x93←0x09`); (2) LWIS fallback (malibu/laguna `google,lwis-i2c-device` DT node); (3) GPIO v1 uAPI (locally defined - libc has no gpio ioctls). `OF_FL_PATH1=cmd:…` hook from the GUI.
- **Thermal** (`temp.rs`): the CPU BIG zone is a hotspot (~90 °C on a cold device), so the TWRP widget must not point at it - HAL-style representative sensor (`soc_therm`) instead of any average; zone numbering shifts across Tensor generations but the named sensor is stable. Exact-path mode when `thermal_temp_path` set, else auto over `thermal_zone_types`; re-run from `boot` after `vendor_dlkm` lands.
- **dm truth** (`dm.rs`): `/tmp/fox_dm_state` (`<name> <dm-target>` per line, `# v1 epoch=` header) - same lifetime as the dm namespace (tmpfs). `dm-watch` polls `/dev/block/mapper` and rewrites on change, **snapshot-only by design**: it never maps/unmaps, so it can't resurrect a node `Unmap_Super_Devices` just destroyed at format time. `KEEP_PARTS = ["vendor_dlkm", "vendor"]` (first-stage never maps them; without a kept node `/vendor_dlkm` can never mount - the gs101 raven field case). TWRP needs no new logic: stable nodes + truth file are enough. Shell stages keep refreshing as a trap-on-EXIT fallback.

</details>

<details>
<summary>🐚 Shell stages - the full list (pixelrunatboot.sh)</summary>

- `props-apply <family> k=v...` (`resetprop` × N → stdout count; `setenforce 0`); `slot-detect` (`bootctl`, prints `_a`/`_b`); `dm-map <part> <sfx> <slot>` (map one LP node for TWRP); `ko-fetch` (`siw read` → `iw read` with map+mount+loop-connect fallback, both slots; depth guard: `lib/modules/*.ko` only - pagesize subdirs shadowed flat modules with foreign CRCs on 6.12); `fw-fetch` (firmware → `/vendor/firmware`); `magiskboot-unpack` (`unzip` → `busybox` into `/system/bin`); `meta-fix` (waits for the block node, cleans `/metadata/ota`); `touch-unpack <code>` (`/<code>.lgz` → `/`, bad codename rejected).
- Helpers: `_siw_stream` (one global namespace - all vars `local`, the old clobber staged the image into itself), `_iw_extract` (bare recursive `iw read -f` + grep filter), `_siw_map`/`_siw_unmap` (only own mappings ever unmapped - foreign first-stage/TWRP/lptools nodes are read-only), `_siw_map_copy`/`_siw_connect_copy` (loop-connect read-only mount for non-`.ko` files), `_dm_target`/`_keep_listed`/`_keep_node_mapped`/`_dm_snapshot` (mapper truth + keep-list, mirrored by the Rust `dm` module).
- `siw`/`iw`/`lptools_new` are static arm64 utilities shipped in the ramdisk. Stages use `siw read` → `iw read` (streaming reads without mounting) + DM mapping via `siw map` with settle-wait and siw-UUID ownership; `lptools_new` ships as a fallback binary but no stage invokes it. Mount-hunt exists only for non-`.ko` files (aocd/libs) that ko-fetch cannot carry.

</details>

<details>
<summary>⚡ High-speed install (`fox-hsinstall`) + live slot switch</summary>

- Opt-in, default off, A/B devices only. Two toggles: **High-speed flash** on the zip flash-confirm page, and since beta_1_2 also on the **sideload** screen (before that HS refused sideload packages by design - exit 5, stock path; the sideload pipe is now tuned for USB: sequential reads, reused fd, deeper readahead, so the cable is the limit, not the engine). `twinstall` detects a payload-based OTA and runs `recovery-pixel-boot install-rom <zip>` over the same updater pipe protocol (`progress` / `set_progress` / `ui_print`); stock `update_engine` stays the fallback. Field-measured on shiba, ~3.3 GB EvoX zip: HS sideload 3:05 vs 8:30 stock, local HS install 0:50 vs 3:50 stock.
- Engine verdicts: `pre-device` mismatch vs `ro.product.device` = hard error, never falls back. Incremental OTA = exit 2, stock path. Missing helper `siw` = exit 5, stock path (`lptools_new` ships in the ramdisk but no code path requires it). Encrypted devices used to fail HS - fixed, they flash now (tester tag test_9_6).
- Flow: `siw info -f json` live state → phase-2 fit ladder (`FITS_AS_IS → KILL_ONE → KILL_TWO → KILL_MANY → WIPE_ALL → IMPOSSIBLE`, kills largest-first from the current slot) → backup critical weights 2+4 to `/tmp` (super/modem excluded) → single-threaded group rebuild via `siw` (unmap, remove + create in both metadata slots, map writable) → batches 1→2→3→4 with LPT order, per-thread sha256 + barrier per batch → on failure finish the batch, roll back (skipped for the weight-1 super batch, nothing critical backed up yet), exit 1 (the engine itself never switches the slot) → `fsync` + `sync`. Never formats `/data`: the downgrade-format trigger is deliberately not copied.
- Progress you actually see: short Step 1 - Step 4 lines instead of verify spam (errors stay fully visible); the live bar fills bands 65/75/85/95 and each quiet tail reports `verifying <partition>` instead of a frozen bar.
- After HS flash the installer **always sets the fresh slot active** (tester tag test_9_9: the slot used to stay put and the phone dropped to bootloader; found on Pixel 6, could hit any device by the code).
- `fox-hsinstall` is cargo-built (`include/source/`, outside Soong), static prebuilt in `recovery/root`. LP surgery goes through forked `/system/bin/siw` - the kernel guard silently drops raw super writes from any other binary, so direct `liblp` writes are impossible by design. Codecs are pure Rust (`lzma-rust2` + `ruzstd` + `crabz2`). Progress over stdout (updater pipe: staged user lines, `set_progress`, `log` diagnostics) relayed live to the GUI console.
- **Live slot switch** (`tw_live_switch` toggle, default off, inert; sits on install options + sideload page): kills the reboot between ROM and addons - the session moves to the fresh slot as the install finishes, so a **ROM → GApps → Magisk** queue flashes in one go (GApps land in the fresh system, Magisk patches the fresh boot; same for a kernel after the ROM). On = auto-switch, no reboot; off = classic flow (flash, then reboot to switch). Success shows a "slot switched live, flash the next ZIP without reboot" message (before that it was silent).
- Slot discipline with the toggle on: the target is normalized before every flash - while on A it always flashes to B, never into the booted slot, so a queued addon chain stays consistent; a flash aimed at the booted slot is switched back to the booted slot with a warning and continued (file manager and sideload); the engine itself refuses the booted slot as a backstop; external slot changes are picked up automatically. Internals: reconcile with `bootctl` at install start, snapshot the booted truth at startup (`bootctl` is spoofable), re-bind spoof files, map the new slot via `siw` before rescan can drop objects. Guard trips are normalized instead of refusing a second flash. DFE-compatible source/target slot markers. Mount-before-refresh in the live path. Install screen toggle order today: Auto reflash, High-speed flash, Live switch, then signature check and unmounts.

</details>
