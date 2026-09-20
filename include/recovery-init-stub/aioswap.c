/* aioswap.c — AIO family swap at stub time (post LGZ unpack, pre init).
 *
 * Raw syscalls + string ops only (same constraints as snapshot.c):
 * fully static, no shared libs, no stdio.
 *
 * Best-effort, never fatal: any failure keeps the zuma placeholders
 * and the boot proceeds. Diagnostics go to /dev/kmsg and
 * /tmp/aio_stub.log, both best-effort.
 */

#include "aioswap.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#define AIO_DEVICES "/system/etc/aio/devices.txt"
#define AIO_FAMILIES "/system/etc/aio/families.txt"
#define AIO_ETC "/system/etc"
#define AIO_LOG "/tmp/aio_stub.log"
#define KMSG "/dev/kmsg"

#define RC_COMMON "/init.recovery.pixel_common.rc"
#define RC_USB "/init.recovery.usb.rc"
/* AIO images ship the rc files with blanked controller markers (the
 * build replaces the 11210000 default with these); the swap fills in
 * the detected family. Never matches real hardware, so an unswapped
 * boot fails visibly instead of silently acting like zuma. */
#define USB_BLANK_BASE "00000000"
#define USB_BLANK "UNKNOWN.dwc3"

#define VINTF_DIR "/vendor/etc/vintf/manifest"
#define HW_DIR "/vendor/bin/hw"
#define KM_RUST "android.hardware.security.keymint-service.rust.trusty"
#define KM_CPP "android.hardware.security.keymint-service.trusty"
#define KM_PROP "ro.recovery.keymint="

static const char* const kFamilies[] = {
    "gs101", "gs201", "zuma", "zumapro", "laguna", "malibu", NULL,
};

static int kmsg_fd = -1;
static int log_fd = -1;

/* write-all, result intentionally unchecked (best-effort logging). */
static void aio_write(int fd, const char* s, size_t len) {
    while (len > 0) {
        ssize_t w = write(fd, s, len);
        if (w < 0) {
            if (errno == EINTR) continue;
            return;
        }
        if (w == 0) return;
        s += w;
        len -= (size_t)w;
    }
}

static void aio_log(const char* s) {
    size_t len = strlen(s);
    /* Prefix once per line batch: caller passes complete lines. */
    if (kmsg_fd < 0) {
        kmsg_fd = open(KMSG, O_WRONLY | O_CLOEXEC);
    }
    if (kmsg_fd >= 0) {
        aio_write(kmsg_fd, "ofx-aio: ", 9);
        aio_write(kmsg_fd, s, len);
    }
    if (log_fd < 0) {
        log_fd = open(AIO_LOG, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0644);
    }
    if (log_fd >= 0) {
        aio_write(log_fd, "ofx-aio: ", 9);
        aio_write(log_fd, s, len);
    }
}

/* Read a small text file into buf (NUL-terminated). Returns bytes or -1. */
static ssize_t read_file(const char* path, char* buf, size_t cap) {
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    ssize_t total = 0;
    if (fd < 0) return -1;
    for (;;) {
        ssize_t r;
        if ((size_t)total >= cap - 1) break;
        r = read(fd, buf + total, cap - 1 - (size_t)total);
        if (r < 0) {
            if (errno == EINTR) continue;
            total = -1;
            break;
        }
        if (r == 0) break;
        total += r;
    }
    close(fd);
    if (total >= 0) buf[total] = '\0';
    return total;
}

/* Copy src -> dst preserving src mode. Returns 0 on success. */
static int copy_file(const char* src, const char* dst) {
    struct stat st;
    int in = -1, out = -1;
    char buf[32768];
    int rc = -1;
    if (stat(src, &st) != 0) return -1;
    in = open(src, O_RDONLY | O_CLOEXEC);
    if (in < 0) return -1;
    out = open(dst, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600);
    if (out < 0) goto out;
    for (;;) {
        ssize_t r = read(in, buf, sizeof(buf));
        ssize_t w;
        char* p;
        if (r < 0) {
            if (errno == EINTR) continue;
            goto out;
        }
        if (r == 0) break;
        p = buf;
        while (r > 0) {
            w = write(out, p, (size_t)r);
            if (w < 0) {
                if (errno == EINTR) continue;
                goto out;
            }
            p += w;
            r -= w;
        }
    }
    rc = 0;
out:
    if (in >= 0) close(in);
    if (out >= 0) close(out);
    if (rc == 0) chmod(dst, st.st_mode & 07777);
    return rc;
}

/* Replace all occurrences of `from` with `to` in the file at path.
 * Returns replacements made, or -1 on I/O error. */
static int sed_file(const char* path, const char* from, const char* to) {
    /* rc files are small (usb.rc ~19K); 128K cap is generous. */
    static char in[131072];
    static char out[147456];
    ssize_t len = read_file(path, in, sizeof(in));
    size_t flen = strlen(from);
    size_t tlen = strlen(to);
    size_t i = 0, o = 0;
    int n = 0;
    struct stat st;
    int fd;
    if (len < 0 || flen == 0) return -1;
    if (stat(path, &st) != 0) return -1;
    while (i < (size_t)len) {
        if (i + flen <= (size_t)len && memcmp(in + i, from, flen) == 0) {
            if (o + tlen >= sizeof(out) - 1) return -1;
            memcpy(out + o, to, tlen);
            o += tlen;
            i += flen;
            n++;
        } else {
            if (o + 1 >= sizeof(out) - 1) return -1;
            out[o++] = in[i++];
        }
    }
    out[o] = '\0';
    if (n == 0) return 0;
    fd = open(path, O_WRONLY | O_TRUNC | O_CLOEXEC);
    if (fd < 0) return -1;
    {
        size_t left = o;
        char* p = out;
        while (left > 0) {
            ssize_t w = write(fd, p, left);
            if (w < 0) {
                if (errno == EINTR) continue;
                close(fd);
                return -1;
            }
            p += w;
            left -= (size_t)w;
        }
    }
    close(fd);
    chmod(path, st.st_mode & 07777);
    return n;
}

/* /proc is not mounted yet at stub time. Mount it on demand for
 * cmdline reads, then unmount before handing off to init so the real
 * init sees pristine state. Only unmounts what we mounted ourselves. */
static int proc_ours = 0;

static void ensure_proc(void) {
    char probe[16];
    if (read_file("/proc/cmdline", probe, sizeof(probe)) >= 0) return;
    if (mount("proc", "/proc", "proc", MS_NOSUID | MS_NOEXEC | MS_NODEV,
              NULL) == 0) {
        proc_ours = 1;
    }
}

static void release_proc(void) {
    if (proc_ours) {
        umount("/proc");
        proc_ours = 0;
    }
}

/* <key><value> from /proc/cmdline (caller ensures /proc). */
static int cmdline_value(const char* key, char* out, size_t cap) {
    char cmd[4096];
    ssize_t len;
    char* p;
    size_t n;
    size_t klen = strlen(key);
    len = read_file("/proc/cmdline", cmd, sizeof(cmd));
    if (len <= 0) return -1;
    p = strstr(cmd, key);
    if (p == NULL) return -1;
    p += klen;
    n = 0;
    while (*p != '\0' && *p != ' ' && *p != '\t' && *p != '\n' && n + 1 < cap) {
        out[n++] = *p++;
    }
    out[n] = '\0';
    return n > 0 ? 0 : -1;
}

static int cmdline_hardware(char* out, size_t cap) {
    return cmdline_value("androidboot.hardware=", out, cap);
}

/* devices.txt lookup: "device:family" per line. Returns 0, fam set. */
static int map_device_family(const char* device, char* fam, size_t cap) {
    static char map[8192];
    char* line;
    if (read_file(AIO_DEVICES, map, sizeof(map)) < 0) return -1;
    line = map;
    while (*line != '\0') {
        char* eol = strchr(line, '\n');
        char* sep;
        if (eol != NULL) *eol = '\0';
        sep = strchr(line, ':');
        if (sep != NULL && strncmp(line, device, (size_t)(sep - line)) == 0 &&
            device[sep - line] == '\0') {
            size_t n = strlen(sep + 1);
            if (n == 0 || n >= cap) return -1;
            memcpy(fam, sep + 1, n + 1);
            return 0;
        }
        if (eol == NULL) break;
        line = eol + 1;
    }
    return -1;
}

/* families.txt lookup: "fam:keymint=<rust|cpp>:usbctrl=<base>.dwc3|".
 * keymint/usbctrl buffers filled (usbctrl may end up empty). */
static int read_family_info(const char* fam, char* keymint, size_t kcap,
                            char* usbctrl, size_t ucap) {
    static char info[4096];
    char* line;
    if (read_file(AIO_FAMILIES, info, sizeof(info)) < 0) return -1;
    line = info;
    while (*line != '\0') {
        char* eol = strchr(line, '\n');
        char* km;
        char* uc;
        size_t flen = strlen(fam);
        if (eol != NULL) *eol = '\0';
        if (strncmp(line, fam, flen) == 0 && line[flen] == ':') {
            km = strstr(line, ":keymint=");
            uc = strstr(line, ":usbctrl=");
            if (km == NULL) return -1;
            km += 9;
            {
                char* end = strchr(km, ':');
                size_t n = end != NULL ? (size_t)(end - km) : strlen(km);
                if (n == 0 || n >= kcap) return -1;
                memcpy(keymint, km, n);
                keymint[n] = '\0';
            }
            usbctrl[0] = '\0';
            if (uc != NULL) {
                size_t n = strlen(uc + 9);
                if (n >= ucap) n = ucap - 1;
                memcpy(usbctrl, uc + 9, n);
                usbctrl[n] = '\0';
            }
            return 0;
        }
        if (eol == NULL) break;
        line = eol + 1;
    }
    return -1;
}

static int has_suffix_xml(const char* fam) {
    char path[256];
    struct stat st;
    snprintf(path, sizeof(path), "%s/keymint.%s.xml", VINTF_DIR, fam);
    return stat(path, &st) == 0;
}

void aioswap_run(void) {
    char device[64];
    char fam[32];
    char keymint[16];
    char usbctrl[32];
    int i;
    aio_log("stub swap start\n");
    ensure_proc();
    /* Test hook first: ofx_swap=<family> forces the branch (manual
     * testing on zuma hardware included). Loudly logged. */
    if (cmdline_value("ofx_swap=", fam, sizeof(fam)) == 0) {
        for (i = 0; kFamilies[i] != NULL; i++) {
            if (strcmp(fam, kFamilies[i]) != 0) continue;
            {
                char msg[128];
                snprintf(msg, sizeof(msg),
                         "OVERRIDE family=%s (test hook, detection skipped)\n",
                         fam);
                aio_log(msg);
            }
            goto swapped_family;
        }
        aio_log("OVERRIDE value unknown, falling back to detection\n");
    }
    if (cmdline_hardware(device, sizeof(device)) != 0) {
        release_proc();
        aio_log("FALLBACK: no androidboot.hardware, keeping placeholders\n");
        return;
    }
    {
        char msg[128];
        snprintf(msg, sizeof(msg), "hardware=%s\n", device);
        aio_log(msg);
    }
    if (map_device_family(device, fam, sizeof(fam)) != 0) {
        release_proc();
        aio_log("FALLBACK: device not in map, keeping placeholders\n");
        return;
    }
    {
        char msg[128];
        snprintf(msg, sizeof(msg), "family=%s\n", fam);
        aio_log(msg);
    }
swapped_family:
    release_proc();
    if (read_family_info(fam, keymint, sizeof(keymint), usbctrl,
                         sizeof(usbctrl)) != 0) {
        aio_log("FALLBACK: family not in manifest, keeping placeholders\n");
        return;
    }
    {
        char msg[128];
        snprintf(msg, sizeof(msg), "keymint=%s usbctrl=%s\n", keymint,
                 usbctrl[0] != '\0' ? usbctrl : "(default)");
        aio_log(msg);
    }

    /* fstab / flags / wipe: <name>.<fam> -> live file. */
    {
        static const char* const names[] = {
            "recovery.fstab", "twrp.flags", "recovery.wipe", NULL,
        };
        int k;
        for (k = 0; names[k] != NULL; k++) {
            char src[256];
            char dst[256];
            snprintf(src, sizeof(src), "%s/%s.%s", AIO_ETC, names[k], fam);
            snprintf(dst, sizeof(dst), "%s/%s", AIO_ETC, names[k]);
            if (copy_file(src, dst) == 0) {
                char msg[256];
                snprintf(msg, sizeof(msg), "swap %s <- %s\n", names[k], fam);
                aio_log(msg);
            } else {
                char msg[256];
                snprintf(msg, sizeof(msg), "no kit %s.%s, placeholder kept\n",
                         names[k], fam);
                aio_log(msg);
            }
        }
    }

    /* USB controller in rc files (parsed by init AFTER us). The build
     * blanks the default to UNKNOWN markers; empty manifest usbctrl
     * means the 11210000 default. Always written: an unswapped boot
     * must fail visibly, never silently act like zuma. */
    {
        const char* uc = usbctrl[0] != '\0' ? usbctrl : "11210000.dwc3";
        char base[32];
        const char* dot = strchr(uc, '.');
        size_t n = dot != NULL ? (size_t)(dot - uc) : strlen(uc);
        int r1, r2;
        if (n >= sizeof(base)) n = sizeof(base) - 1;
        memcpy(base, uc, n);
        base[n] = '\0';
        {
            /* sed twice: "<base>.usb" then full controller name. */
            char from_usb[48];
            char to_usb[48];
            snprintf(from_usb, sizeof(from_usb), "%s.usb", USB_BLANK_BASE);
            snprintf(to_usb, sizeof(to_usb), "%s.usb", base);
            r1 = sed_file(RC_COMMON, from_usb, to_usb);
            r2 = sed_file(RC_COMMON, USB_BLANK, uc);
            {
                char msg[128];
                snprintf(msg, sizeof(msg), "usbctrl pixel_common: %d+%d\n",
                         r1 < 0 ? -1 : r1, r2 < 0 ? -1 : r2);
                aio_log(msg);
            }
            r1 = sed_file(RC_USB, from_usb, to_usb);
            r2 = sed_file(RC_USB, USB_BLANK, uc);
            {
                char msg[128];
                snprintf(msg, sizeof(msg), "usbctrl usb.rc: %d+%d\n",
                         r1 < 0 ? -1 : r1, r2 < 0 ? -1 : r2);
                aio_log(msg);
            }
        }
    }

    /* KeyMint: drop sibling fragments, keep one binary. Only when the
     * suffixed swap-kit layout is present (AIO); per-family builds with
     * a single keymint.xml are untouched. */
    if (has_suffix_xml(fam)) {
        int dropped = 0;
        for (i = 0; kFamilies[i] != NULL; i++) {
            if (strcmp(kFamilies[i], fam) != 0) {
                char path[256];
                snprintf(path, sizeof(path), "%s/keymint.%s.xml", VINTF_DIR,
                         kFamilies[i]);
                if (unlink(path) == 0) dropped++;
            }
        }
        {
            char msg[64];
            snprintf(msg, sizeof(msg), "vintf pruned, dropped=%d\n", dropped);
            aio_log(msg);
        }
        {
            char rust[256];
            char cpp[256];
            struct stat st;
            int have_rust, have_cpp;
            snprintf(rust, sizeof(rust), "%s/%s", HW_DIR, KM_RUST);
            snprintf(cpp, sizeof(cpp), "%s/%s", HW_DIR, KM_CPP);
            have_rust = stat(rust, &st) == 0;
            have_cpp = stat(cpp, &st) == 0;
            if (have_rust && have_cpp) {
                if (strcmp(keymint, "cpp") == 0) {
                    unlink(rust);
                    aio_log("keymint binary kept: cpp\n");
                } else {
                    unlink(cpp);
                    aio_log("keymint binary kept: rust\n");
                }
            } else {
                aio_log("single keymint binary, kept\n");
            }
        }
    }

    /* Selector prop for the gated keymint triggers in pixel_common.rc. */
    {
        static const char* const props[] = {
            "/prop.default", "/default.prop", NULL,
        };
        int k;
        for (k = 0; props[k] != NULL; k++) {
            static char cur[65536];
            if (read_file(props[k], cur, sizeof(cur)) < 0) continue;
            if (strstr(cur, KM_PROP) != NULL) {
                aio_log("keymint prop already set\n");
                break;
            }
            {
                int fd = open(props[k], O_WRONLY | O_APPEND | O_CLOEXEC);
                if (fd >= 0) {
                    char line[64];
                    snprintf(line, sizeof(line), "%s%s\n", KM_PROP, keymint);
                    aio_write(fd, line, strlen(line));
                    close(fd);
                    aio_log("keymint prop set\n");
                }
            }
            break;
        }
    }

    aio_log("stub swap done\n");
}
