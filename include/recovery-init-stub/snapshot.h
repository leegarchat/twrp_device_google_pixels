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
