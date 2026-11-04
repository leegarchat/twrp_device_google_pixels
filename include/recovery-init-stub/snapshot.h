/* snapshot.h — ramdisk snapshot built into recovery-init-stub.
 *
 * Faithful C port of include/ramdisk_snapshot (Rust). Reads a manifest
 * (`<type> <octal> <uid> <gid> <path>`, symlinks as
 * `l <perms> <uid> <gid> <path> -> <target>`) and copies every entry into a
 * snapshot dir, then snapshots the vendor_boot block device. Used by
 * reflash_twrp.sh to rebuild the vendor_boot image.
 *
 * Returns 0 if all entries copied, 1 if any entry failed.
 */
#pragma once

int snapshot_run(const char* manifest_path, const char* snap_dir);

/* Block-device pass only (vendor_boot dump). For second-stage entry
 * when UFS drivers are up: skips work if a dump already exists,
 * mounts /proc and /sys on demand (unmounting only what it mounted).
 * Returns 0 if at least one image is present afterwards. */
int snapshot_vendor_boot_only(void);
