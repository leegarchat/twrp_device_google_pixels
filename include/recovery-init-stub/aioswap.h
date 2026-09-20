/* aioswap.h — AIO family swap at stub time (post LGZ unpack, pre init).
 *
 * The universal image ships family-neutral placeholders (zuma) plus a
 * per-family swap kit. After the cluster is unpacked and before the real
 * init parses any rc file, this detects the device from the kernel
 * cmdline, resolves its SoC family and swaps the family files into
 * place: recovery.fstab / twrp.flags / recovery.wipe, the DWC3 USB
 * controller address in the recovery rc files, KeyMint manifests and
 * binaries, and the ro.recovery.keymint selector prop.
 *
 * Best-effort, never fatal: any failure keeps the placeholders and the
 * boot proceeds. Diagnostics go to /dev/kmsg (best-effort) and
 * /tmp/aio_stub.log (picked up by flog.sh).
 */
#pragma once

void aioswap_run(void);
