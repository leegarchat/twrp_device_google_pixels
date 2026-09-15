/* snapshot.c — ramdisk snapshot built into recovery-init-stub.
 *
 * C port of include/ramdisk_snapshot/src/main.rs. Same manifest format,
 * same metadata policy (live lstat wins, manifest values on fallback),
 * same vendor_boot sysfs scan + mknod fallback.
 *
 * Raw syscalls only (open/read/write, no stdio): keeps the static binary
 * free of the stdio/locale footprint. Silent by design (no logging):
 * failures surface as the return code only.
 */

#include "snapshot.h"

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/types.h>
#include <unistd.h>

#define SNAP_VENDOR_BOOT "/dev/vendor_boot_snapshot"

/* Silent build: all diagnostic logging compiles out. Failures surface
 * as the snapshot_run return code only. */
#define snap_log(...) \
    do {              \
    } while (0)

/* mkdir -p with per-level chmod (mirrors Rust mkdirs). */
static void snap_mkdirs(const char* path, unsigned mode) {
    char tmp[1024];
    size_t len = strlen(path);
    if (len == 0 || len >= sizeof(tmp)) return;
    memcpy(tmp, path, len + 1);
    for (char* s = tmp + 1; *s; s++) {
        if (*s == '/') {
            *s = '\0';
            mkdir(tmp, 0755);
            chmod(tmp, (mode_t)mode);
            *s = '/';
        }
    }
    mkdir(tmp, 0755);
    chmod(tmp, (mode_t)mode);
}

static void snap_ensure_parent(const char* path) {
    const char* slash = strrchr(path, '/');
    if (slash == NULL || slash == path) return;
    char tmp[1024];
    size_t len = (size_t)(slash - path);
    if (len >= sizeof(tmp)) return;
    memcpy(tmp, path, len);
    tmp[len] = '\0';
    snap_mkdirs(tmp, 0755);
}

static int snap_copy_file(const char* src, const char* dst) {
    int in = open(src, O_RDONLY | O_CLOEXEC);
    if (in < 0) {
        snap_log(1, "[SNAPSHOT] copy(%s) -> %s: open src: %s\n", src, dst, strerror(errno));
        return 0;
    }
    int out = open(dst, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
    if (out < 0) {
        snap_log(1, "[SNAPSHOT] copy(%s) -> %s: open dst: %s\n", src, dst, strerror(errno));
        close(in);
        return 0;
    }
    char buf[65536];
    int ok = 1;
    for (;;) {
        ssize_t r = read(in, buf, sizeof(buf));
        if (r < 0) {
            snap_log(1, "[SNAPSHOT] copy(%s) -> %s: %s\n", src, dst, strerror(errno));
            ok = 0;
            break;
        }
        if (r == 0) break;
        size_t off = 0;
        while (off < (size_t)r) {
            ssize_t w = write(out, buf + off, (size_t)r - off);
            if (w <= 0) {
                snap_log(1, "[SNAPSHOT] copy(%s) -> %s: %s\n", src, dst, strerror(errno));
                ok = 0;
                break;
            }
            off += (size_t)w;
        }
        if (!ok) break;
    }
    close(in);
    close(out);
    return ok;
}

/* Metadata policy: live lstat wins, manifest values on fallback. */
static void snap_apply_metadata(const char* src, const char* dst, unsigned m_mode,
                                unsigned m_uid, unsigned m_gid, int is_link) {
    struct stat st;
    memset(&st, 0, sizeof(st));
    int use_live = (lstat(src, &st) == 0);
    unsigned uid = use_live ? st.st_uid : m_uid;
    unsigned gid = use_live ? st.st_gid : m_gid;
    if (is_link || (use_live && S_ISLNK(st.st_mode))) {
        lchown(dst, uid, gid);
    } else {
        chown(dst, uid, gid);
        chmod(dst, (mode_t)(use_live ? (st.st_mode & 07777) : (m_mode & 07777)));
    }
    if (use_live) {
        struct timespec times[2];
        times[0].tv_sec = st.st_atime;
        times[0].tv_nsec = 0;
        times[1].tv_sec = st.st_mtime;
        times[1].tv_nsec = 0;
        utimensat(AT_FDCWD, dst, times, AT_SYMLINK_NOFOLLOW);
    }
}

/* Read a whole small file (procfs/sysfs/debug). Returns length, -1 on error. */
static ssize_t snap_read_file(const char* path, char* buf, size_t cap) {
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return -1;
    size_t total = 0;
    for (;;) {
        if (total >= cap) break;
        ssize_t r = read(fd, buf + total, cap - total);
        if (r < 0) {
            close(fd);
            return -1;
        }
        if (r == 0) break;
        total += (size_t)r;
    }
    close(fd);
    return (ssize_t)total;
}

/* ------------------------------------------------------------------ */
/* debug dumps: compiled out (silent build)                              */
/* ------------------------------------------------------------------ */

static void snap_path_join(char* out, size_t outsz, const char* a, const char* b) {
    size_t i = 0;
    while (*a != '\0' && i + 1 < outsz) out[i++] = *a++;
    if (i + 1 < outsz) out[i++] = '/';
    while (*b != '\0' && i + 1 < outsz) out[i++] = *b++;
    out[i] = '\0';
}

/* ------------------------------------------------------------------ */
/* vendor_boot snapshot                                                */
/* ------------------------------------------------------------------ */

static const char* snap_find_block(const char* partname) {
    static char dev_path[512];
    DIR* d = opendir("/sys/class/block");
    if (d == NULL) return NULL;
    char sub[640];
    struct dirent* e;
    const char* found = NULL;
    while ((e = readdir(d)) != NULL) {
        if (e->d_name[0] == '.') continue;
        snap_path_join(sub, sizeof(sub), "/sys/class/block", e->d_name);
        size_t base = strlen(sub);
        if (base + 8 >= sizeof(sub)) continue;
        memcpy(sub + base, "/uevent", 8);
        char uevent[2048];
        ssize_t r = snap_read_file(sub, uevent, sizeof(uevent) - 1);
        if (r < 0) continue;
        uevent[r] = '\0';
        /* Match a PARTNAME=<partname> line exactly. */
        int matched = 0;
        char* line = uevent;
        while (*line != '\0') {
            char* nl = strchr(line, '\n');
            if (nl != NULL) *nl = '\0';
            if (!strncmp(line, "PARTNAME=", 9) && !strcmp(line + 9, partname)) matched = 1;
            if (nl == NULL) break;
            line = nl + 1;
            if (matched) break;
        }
        if (!matched) continue;
        snap_path_join(dev_path, sizeof(dev_path), "/dev/block", e->d_name);
        if (access(dev_path, F_OK) == 0) {
            found = dev_path;
            break;
        }
        memcpy(sub + base, "/dev", 5);
        char devnum[64];
        r = snap_read_file(sub, devnum, sizeof(devnum) - 1);
        if (r > 0) {
            unsigned maj = 0, min = 0;
            char* colon = strchr(devnum, ':');
            if (colon != NULL) {
                *colon = '\0';
                maj = (unsigned)strtoul(devnum, NULL, 10);
                min = (unsigned)strtoul(colon + 1, NULL, 10);
                snap_mkdirs("/dev/block", 0755);
                if (mknod(dev_path, S_IFBLK | 0600, makedev(maj, min)) == 0) {
                    snap_log(0, "[SNAPSHOT] node %s was missing, recreated via mknod %u:%u\n",
                             dev_path, maj, min);
                    found = dev_path;
                    break;
                }
            }
        }
        break;
    }
    closedir(d);
    return found;
}

/* Handles `androidboot.slot_suffix="_a"` (bootconfig) and `=..._a`
 * (cmdline) spellings. Returns 1 and fills out on success. */
static int snap_slot_from_text(const char* text, const char* key, char* out, size_t outsz) {
    const char* pos = strstr(text, key);
    if (pos == NULL) return 0;
    const char* v = pos + strlen(key);
    while (*v == '=' || *v == ' ' || *v == '"') v++;
    size_t n = 0;
    while (v[n] != '\0' && v[n] != '"' && v[n] != '\n' && v[n] != ' ' && n + 1 < outsz) {
        out[n] = v[n];
        n++;
    }
    out[n] = '\0';
    return n > 0;
}

static void snap_get_boot_slot(char* out, size_t outsz) {
    out[0] = '\0';
    char buf[8192];
    ssize_t r = snap_read_file("/proc/bootconfig", buf, sizeof(buf) - 1);
    if (r > 0) {
        buf[r] = '\0';
        if (snap_slot_from_text(buf, "androidboot.slot_suffix", out, outsz)) return;
    }
    r = snap_read_file("/proc/cmdline", buf, sizeof(buf) - 1);
    if (r > 0) {
        buf[r] = '\0';
        if (snap_slot_from_text(buf, "androidboot.slot_suffix", out, outsz)) return;
    }
}

static void snap_snapshot_vendor_boot(void) {
    snap_log(0, "[SNAPSHOT] Backing up vendor_boot...\n");
    snap_mkdirs(SNAP_VENDOR_BOOT, 0755);

    char slot[16];
    snap_get_boot_slot(slot, sizeof(slot));
    if (slot[0] == '\0') {
        snap_log(0, "[SNAPSHOT] No slot suffix found; guessing from /dev/block/by-name/boot_*\n");
        if (access("/dev/block/by-name/boot_b", F_OK) == 0) {
            strcpy(slot, "_b");
        } else if (access("/dev/block/by-name/boot_a", F_OK) == 0) {
            strcpy(slot, "_a");
        }
    }

    const char* slots[2];
    int nslots = 0;
    if (slot[0] == '\0') {
        snap_log(0, "[SNAPSHOT] Slot still unknown; dumping both _a and _b\n");
        slots[0] = "_a";
        slots[1] = "_b";
        nslots = 2;
    } else {
        slots[0] = slot;
        nslots = 1;
    }

    int i;
    for (i = 0; i < nslots; i++) {
        char partname[32];
        size_t pi = 0;
        const char* pvb = "vendor_boot";
        while (*pvb != '\0' && pi + 1 < sizeof(partname)) partname[pi++] = *pvb++;
        const char* ps = slots[i];
        while (*ps != '\0' && pi + 1 < sizeof(partname)) partname[pi++] = *ps++;
        partname[pi] = '\0';
        snap_log(0, "[SNAPSHOT] Looking for %s via sysfs...\n", partname);
        const char* dev = snap_find_block(partname);
        if (dev == NULL) {
            snap_log(1, "[SNAPSHOT] partition %s not found in sysfs\n", partname);
            continue;
        }
        char dst[512];
        snap_path_join(dst, sizeof(dst), SNAP_VENDOR_BOOT,
                       nslots == 1 ? "vendor_boot.img" : partname);
        if (nslots != 1) {
            /* dst is "<snapdir>/vendor_boot_a" — needs the .img suffix. */
            size_t dl = strlen(dst);
            if (dl + 4 < sizeof(dst)) memcpy(dst + dl, ".img", 5);
        }
        snap_log(0, "[SNAPSHOT] Copying %s -> %s\n", dev, dst);
        if (snap_copy_file(dev, dst)) {
            snap_log(0, "[SNAPSHOT] OK: %s snapshotted\n", partname);
        } else {
            snap_log(1, "[SNAPSHOT] FAILED to copy %s\n", dev);
        }
    }
}

/* ------------------------------------------------------------------ */
/* manifest pass                                                       */
/* ------------------------------------------------------------------ */

static int snap_parse_uint(const char* s, unsigned base, unsigned* out) {
    char* end = NULL;
    errno = 0;
    unsigned long v = strtoul(s, &end, (int)base);
    if (errno != 0 || end == s || *end != '\0') return 0;
    *out = (unsigned)v;
    return 1;
}

static unsigned g_ok = 0, g_fail = 0;
static const char* g_snap_dir = "/dev/ramdisk_snapshot";

static void snap_process_entry(int is_link, int is_dir, unsigned mode, unsigned uid,
                               unsigned gid, const char* rel, const char* link_target) {
    char src[1024], dst[1024];
    src[0] = '/';
    size_t si = 1;
    while (*rel != '\0' && si + 1 < sizeof(src)) src[si++] = *rel++;
    src[si] = '\0';
    snap_path_join(dst, sizeof(dst), g_snap_dir, src + 1);

    if (is_link) {
        snap_ensure_parent(dst);
        unlink(dst);
        if (symlink(link_target, dst) == 0) {
            snap_apply_metadata(src, dst, mode, uid, gid, 1);
            g_ok++;
        } else {
            snap_log(1, "[SNAPSHOT] symlink %s -> %s: %s\n", dst, link_target, strerror(errno));
            g_fail++;
        }
        return;
    }

    if (src[1] == '\0') {
        snap_mkdirs(dst, mode);
        g_ok++;
        return;
    }

    if (is_dir) {
        snap_mkdirs(dst, mode);
        snap_apply_metadata(src, dst, mode, uid, gid, 0);
        g_ok++;
    } else {
        snap_ensure_parent(dst);
        if (snap_copy_file(src, dst)) {
            snap_apply_metadata(src, dst, mode, uid, gid, 0);
            g_ok++;
        } else {
            g_fail++;
        }
    }
}

/* Tokenize head into exactly 5 whitespace-separated tokens (in place). */
static int snap_split5(char* head, char* tok[5]) {
    int n = 0;
    char* p = head;
    while (*p == ' ' || *p == '\t') p++;
    while (*p != '\0') {
        if (n >= 5) return 0; /* more than 5 tokens */
        tok[n++] = p;
        while (*p != '\0' && *p != ' ' && *p != '\t') p++;
        if (*p == '\0') break;
        *p++ = '\0';
        while (*p == ' ' || *p == '\t') p++;
    }
    return n == 5;
}

static void snap_manifest_line(char* line) {
    char* arrow = strstr(line, " -> ");
    if (arrow != NULL) {
        *arrow = '\0';
        const char* target = arrow + 4;
        char* tok[5];
        if (!snap_split5(line, tok) || strcmp(tok[0], "l") != 0 || target[0] == '\0') {
            g_fail++;
            return;
        }
        unsigned mode, uid, gid;
        if (!snap_parse_uint(tok[1], 8, &mode) || !snap_parse_uint(tok[2], 10, &uid) ||
            !snap_parse_uint(tok[3], 10, &gid)) {
            g_fail++;
            return;
        }
        const char* rel = tok[4];
        while (*rel == '/') rel++;
        snap_process_entry(1, 0, mode, uid, gid, rel, target);
        return;
    }

    char* tok[5];
    if (!snap_split5(line, tok) || (strcmp(tok[0], "d") != 0 && strcmp(tok[0], "f") != 0)) {
        g_fail++;
        return;
    }
    unsigned mode, uid, gid;
    if (!snap_parse_uint(tok[1], 8, &mode) || !snap_parse_uint(tok[2], 10, &uid) ||
        !snap_parse_uint(tok[3], 10, &gid)) {
        g_fail++;
        return;
    }
    const char* rel = tok[4];
    while (*rel == '/') rel++;
    /* Manifest root entry ("/" -> empty rel after trim, or literal "/"). */
    if (rel[0] == '\0' || !strcmp(rel, "/")) {
        snap_mkdirs(g_snap_dir, mode);
        g_ok++;
        return;
    }
    snap_process_entry(0, tok[0][0] == 'd', mode, uid, gid, rel, NULL);
}

static void snap_manifest_pass(const char* manifest_path) {
    /* Manifest is ~28KB; single read keeps this stdio-free. */
    static char text[262144];
    ssize_t r = snap_read_file(manifest_path, text, sizeof(text) - 1);
    if (r < 0) {
        snap_log(1, "[SNAPSHOT] Cannot open manifest %s: %s\n", manifest_path, strerror(errno));
        g_fail++;
        return;
    }
    if ((size_t)r >= sizeof(text) - 1) {
        snap_log(1, "[SNAPSHOT] Manifest too large, refusing\n");
        g_fail++;
        return;
    }
    text[r] = '\0';
    snap_log(0, "[SNAPSHOT] Creating ramdisk snapshot in %s\n", g_snap_dir);
    snap_mkdirs(g_snap_dir, 0755);

    char* line = text;
    while (*line != '\0') {
        char* nl = strchr(line, '\n');
        if (nl != NULL) *nl = '\0';
        size_t len = strlen(line);
        while (len > 0 && line[len - 1] == '\r') line[--len] = '\0';
        if (len > 0 && line[0] != '#') snap_manifest_line(line);
        if (nl == NULL) break;
        line = nl + 1;
    }
    snap_log(0, "[SNAPSHOT] Ramdisk Complete: %u entries copied, %u errors\n", g_ok, g_fail);
}

/* ------------------------------------------------------------------ */

int snapshot_run(const char* manifest_path, const char* snap_dir) {
    g_ok = 0;
    g_fail = 0;
    g_snap_dir = snap_dir;
    snap_manifest_pass(manifest_path);
    snap_snapshot_vendor_boot();
    return g_fail > 0 ? 1 : 0;
}
