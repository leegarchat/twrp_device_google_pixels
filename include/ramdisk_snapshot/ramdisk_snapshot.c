/*
 * ramdisk_snapshot - Create a full copy of the ramdisk before LGZ decompression
 *
 * Reads a manifest file and copies every listed file, directory, and symlink
 * from the root filesystem to a snapshot directory (/dev/ramdisk_snapshot/).
 * This snapshot preserves the exact ramdisk state (with compressed binaries)
 * so that reflash_twrp.sh can later recreate the vendor_boot cpio from it.
 *
 * It also automatically snapshots the current vendor_boot block device.
 * All logs are duplicated to /dev/vendor_boot_snapshot/logs for early init debugging.
 *
 * Copyright (C) 2024-2026 The OrangeFox Recovery Project
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/sysmacros.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <dirent.h>
#include <stdarg.h>
#include <sys/system_properties.h>

#define DEFAULT_MANIFEST  "/ramdisk_snapshot_manifest.txt"
#define DEFAULT_SNAP_DIR  "/dev/ramdisk_snapshot"
#define VENDOR_BOOT_SNAP  "/dev/vendor_boot_snapshot"
#define BUF_SIZE          65536
#define MAX_PATH          2048

static FILE *log_fp = NULL;

/* Recursively create directories along a path */
static void mkdirs(const char *path, mode_t mode) {
    char tmp[MAX_PATH];
    size_t len = strlen(path);
    if (len >= sizeof(tmp)) return;
    memcpy(tmp, path, len + 1);

    for (char *p = tmp + 1; *p; p++) {
        if (*p == '/') {
            *p = '\0';
            mkdir(tmp, mode);
            *p = '/';
        }
    }
    mkdir(tmp, mode);
}

/* Инициализация логирования в файл */
static void init_logging() {
    mkdirs(VENDOR_BOOT_SNAP, 0755);
    log_fp = fopen(VENDOR_BOOT_SNAP "/logs", "a");
    if (log_fp) {
        fprintf(log_fp, "\n=========================================\n");
        fprintf(log_fp, "--- Starting ramdisk_snapshot ---\n");
        fprintf(log_fp, "=========================================\n");
        fflush(log_fp);
    }
}

/* Кастомная функция для дублирования логов в консоль и файл */
static void log_print(int is_error, const char *fmt, ...) {
    va_list args;
    
    // Вывод в стандартную консоль
    va_start(args, fmt);
    if (is_error) {
        vfprintf(stderr, fmt, args);
    } else {
        vfprintf(stdout, fmt, args);
    }
    va_end(args);

    if (log_fp) {
        va_start(args, fmt);
        vfprintf(log_fp, fmt, args);
        fflush(log_fp);
        va_end(args);
    }
}

static void dump_dir_contents(const char *dir_path) {
    DIR *dir;
    struct dirent *ent;
    log_print(0, "[SNAPSHOT] Содержимое %s:\n", dir_path);
    
    char base_path[MAX_PATH];
    snprintf(base_path, sizeof(base_path), "%s", dir_path);
    size_t len = strlen(base_path);
    if (len > 0 && base_path[len - 1] == '/') {
        base_path[len - 1] = '\0';
    }

    if ((dir = opendir(base_path)) != NULL) {
        while ((ent = readdir(dir)) != NULL) {
            if (strcmp(ent->d_name, ".") == 0 || strcmp(ent->d_name, "..") == 0) continue;

            char full_path[MAX_PATH];
            snprintf(full_path, sizeof(full_path), "%s/%s", base_path, ent->d_name);

            struct stat st;
            if (lstat(full_path, &st) == 0 && S_ISLNK(st.st_mode)) {
                char target[MAX_PATH];
                ssize_t link_len = readlink(full_path, target, sizeof(target) - 1);
                if (link_len != -1) {
                    target[link_len] = '\0';
                    log_print(0, "  - %s -> %s\n", ent->d_name, target);
                } else {
                    log_print(0, "  - %s -> [ОШИБКА ЧТЕНИЯ]\n", ent->d_name);
                }
            } else {
                log_print(0, "  - %s\n", ent->d_name);
            }
        }
        closedir(dir);
    } else {
        log_print(0, "  (Не удалось открыть директорию: %s)\n", strerror(errno));
    }
}

/* --- Функции сбора отладочной информации --- */

static void dump_cmdline() {
    log_print(0, "\n--- DUMPING /proc/cmdline ---\n");
    FILE *f = fopen("/proc/cmdline", "r");
    if (f) {
        char buf[1024];
        while (fgets(buf, sizeof(buf), f)) {
            log_print(0, "%s", buf);
        }
        fclose(f);
        log_print(0, "\n");
    } else {
        log_print(1, "[SNAPSHOT] ОШИБКА: Не удалось открыть /proc/cmdline\n");
    }
    log_print(0, "-----------------------------\n");
}

static void dump_bootconfig() {
    log_print(0, "\n--- DUMPING /proc/bootconfig ---\n");
    FILE *f = fopen("/proc/bootconfig", "r");
    if (f) {
        char buf[1024];
        while (fgets(buf, sizeof(buf), f)) {
            log_print(0, "%s", buf);
        }
        fclose(f);
        log_print(0, "\n");
    } else {
        log_print(1, "[SNAPSHOT] ПРЕДУПРЕЖДЕНИЕ: Не удалось открыть /proc/bootconfig (старый Android?)\n");
    }
    log_print(0, "--------------------------------\n");
}

static void prop_read_callback(void* cookie, const char* name, const char* value, uint32_t serial) {
    (void)cookie; (void)serial;
    log_print(0, "[%s]: [%s]\n", name, value);
}

static void prop_foreach_callback(const prop_info *pi, void *cookie) {
    __system_property_read_callback(pi, prop_read_callback, cookie);
}

static void dump_getprop() {
    log_print(0, "\n--- DUMPING PROPERTIES (Native Bionic) ---\n");
    if (__system_property_foreach(prop_foreach_callback, NULL) != 0) {
        log_print(1, "[SNAPSHOT] ОШИБКА: Не удалось прочитать свойства (сервис еще не запущен?)\n");
    }
    log_print(0, "--------------------------\n");
}

static int find_ufs_platform_path(char *out_path, size_t max_len) {
    DIR *dir = opendir("/dev/block/platform");
    if (dir) {
        struct dirent *ent;
        while ((ent = readdir(dir)) != NULL) {
            if (ent->d_name[0] == '.') continue;
            if (strstr(ent->d_name, "ufs")) {
                snprintf(out_path, max_len, "/dev/block/platform/%s", ent->d_name);
                closedir(dir);
                return 1;
            }
        }
        closedir(dir);
    }
    /* Fallback: GS201 default */
    snprintf(out_path, max_len, "/dev/block/platform/13200000.ufs");
    return 0;
}

static void dump_debug_info() {
    dump_cmdline();
    dump_bootconfig();
    dump_getprop();
    log_print(0, "\n--- DUMPING TARGET DIRECTORIES ---\n");
    char ufs_path[MAX_PATH];
    if (!find_ufs_platform_path(ufs_path, sizeof(ufs_path))) {
        log_print(0, "[SNAPSHOT] Предупреждение: UFS не найден в /dev/block/platform/, используем fallback: %s\n", ufs_path);
    }
    dump_dir_contents(ufs_path);
    log_print(0, "----------------------------------\n\n");
}


/* Ensure parent directory of a given path exists */
static void ensure_parent(const char *path) {
    char parent[MAX_PATH];
    size_t len = strlen(path);
    if (len >= sizeof(parent)) return;
    memcpy(parent, path, len + 1);

    char *slash = strrchr(parent, '/');
    if (slash && slash != parent) {
        *slash = '\0';
        mkdirs(parent, 0755);
    }
}

static void apply_metadata(const char *src, const char *dst, char type, mode_t m_mode, uid_t m_uid, gid_t m_gid) {
    struct stat st;
    if (lstat(src, &st) == 0) {
        if (S_ISLNK(st.st_mode)) {
            lchown(dst, st.st_uid, st.st_gid);
        } else {
            chown(dst, st.st_uid, st.st_gid);
            chmod(dst, st.st_mode & 07777); 
        }
        struct timespec times[2];
        times[0] = st.st_atim; 
        times[1] = st.st_mtim; 
        utimensat(AT_FDCWD, dst, times, AT_SYMLINK_NOFOLLOW);
    } else {
        if (type == 'l') lchown(dst, m_uid, m_gid);
        else {
            chown(dst, m_uid, m_gid);
            chmod(dst, m_mode & 07777);
        }
    }
}

static int copy_file(const char *src, const char *dst) {
    int fd_src = open(src, O_RDONLY);
    if (fd_src < 0) {
        log_print(1, "[SNAPSHOT] open(%s): %s\n", src, strerror(errno));
        return -1;
    }

    int fd_dst = open(dst, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd_dst < 0) {
        log_print(1, "[SNAPSHOT] create(%s): %s\n", dst, strerror(errno));
        close(fd_src);
        return -1;
    }

    char buf[BUF_SIZE];
    ssize_t n;
    while ((n = read(fd_src, buf, sizeof(buf))) > 0) {
        ssize_t off = 0;
        while (off < n) {
            ssize_t w = write(fd_dst, buf + off, n - off);
            if (w < 0) {
                log_print(1, "[SNAPSHOT] write(%s): %s\n", dst, strerror(errno));
                close(fd_src);
                close(fd_dst);
                return -1;
            }
            off += w;
        }
    }

    close(fd_src);
    close(fd_dst);
    return (n < 0) ? -1 : 0;
}

static int get_or_create_block_device(const char *target_partname, char *out_dev_path, size_t max_len) {
    DIR *dir = opendir("/sys/class/block");
    if (!dir) {
        log_print(1, "[SNAPSHOT] ОШИБКА: Не удалось открыть /sys/class/block\n");
        return 0;
    }
    
    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL) {
        if (ent->d_name[0] == '.') continue;
        
        char uevent_path[MAX_PATH];
        snprintf(uevent_path, sizeof(uevent_path), "/sys/class/block/%s/uevent", ent->d_name);
        
        FILE *f = fopen(uevent_path, "r");
        if (f) {
            char line[256];
            int found = 0;
            while (fgets(line, sizeof(line), f)) {
                if (strncmp(line, "PARTNAME=", 9) == 0) {
                    char *pname = line + 9;
                    char *nl = strchr(pname, '\n');
                    if (nl) *nl = '\0';
                    
                    if (strcmp(pname, target_partname) == 0) {
                        found = 1;
                        break;
                    }
                }
            }
            fclose(f);
            
            if (found) {
                snprintf(out_dev_path, max_len, "/dev/block/%s", ent->d_name);
                
                if (access(out_dev_path, F_OK) != 0) {
                    char dev_info_path[MAX_PATH];
                    snprintf(dev_info_path, sizeof(dev_info_path), "/sys/class/block/%s/dev", ent->d_name);
                    FILE *f_dev = fopen(dev_info_path, "r");
                    if (f_dev) {
                        int maj, min;
                        if (fscanf(f_dev, "%d:%d", &maj, &min) == 2) {
                            mkdirs("/dev/block", 0755); // Убедимся, что папка есть
                            if (mknod(out_dev_path, S_IFBLK | 0600, makedev(maj, min)) == 0) {
                                log_print(0, "[SNAPSHOT] Узел %s отсутствовал, успешно создан вручную (mknod %d:%d)\n", out_dev_path, maj, min);
                            } else {
                                log_print(1, "[SNAPSHOT] ОШИБКА mknod для %s: %s\n", out_dev_path, strerror(errno));
                            }
                        }
                        fclose(f_dev);
                    }
                }
                closedir(dir);
                return 1;
            }
        }
    }
    closedir(dir);
    return 0;
}

static void get_boot_slot(char *slot_out, size_t size) {
    slot_out[0] = '\0';
    
    // Сначала ищем в /proc/bootconfig (Android 12+)
    FILE *f = fopen("/proc/bootconfig", "r");
    if (f) {
        char line[4096];
        while (fgets(line, sizeof(line), f)) {
            char *ptr = strstr(line, "androidboot.slot_suffix");
            if (ptr) {
                char *val = strchr(ptr, '=');
                if (val) {
                    val++;
                    while (*val == ' ' || *val == '"') val++;
                    size_t i = 0;
                    while (val[i] && val[i] != '"' && val[i] != '\n' && val[i] != ' ' && i < size - 1) {
                        slot_out[i] = val[i];
                        i++;
                    }
                    slot_out[i] = '\0';
                    fclose(f);
                    return;
                }
            }
        }
        fclose(f);
    }
    
    f = fopen("/proc/cmdline", "r");
    if (f) {
        char cmdline[4096];
        if (fgets(cmdline, sizeof(cmdline), f)) {
            char *ptr = strstr(cmdline, "androidboot.slot_suffix=");
            if (ptr) {
                ptr += 24; 
                size_t i = 0;
                while (ptr[i] && ptr[i] != ' ' && ptr[i] != '\n' && i < size - 1) {
                    slot_out[i] = ptr[i];
                    i++;
                }
                slot_out[i] = '\0';
            }
        }
        fclose(f);
    }
}

static void snapshot_vendor_boot() {
    log_print(0, "[SNAPSHOT] Инициализация резервного копирования vendor_boot...\n");
    mkdirs(VENDOR_BOOT_SNAP, 0755);
    
    char boot_slot[8];
    get_boot_slot(boot_slot, sizeof(boot_slot));
    
    if (strlen(boot_slot) == 0) {
        log_print(0, "[SNAPSHOT] Суффикс не найден в cmdline/bootconfig. Пытаемся угадать по /dev/block/by-name/boot_X...\n");
        if (access("/dev/block/by-name/boot_b", F_OK) == 0) {
            strcpy(boot_slot, "_b");
            log_print(0, "[SNAPSHOT] Найден boot_b. Предполагаем слот _b.\n");
        } else if (access("/dev/block/by-name/boot_a", F_OK) == 0) {
            strcpy(boot_slot, "_a");
            log_print(0, "[SNAPSHOT] Найден boot_a. Предполагаем слот _a.\n");
        }
    }
    
    if (strlen(boot_slot) > 0) {
        char partname[64];
        snprintf(partname, sizeof(partname), "vendor_boot%s", boot_slot);
        
        char dev_path[MAX_PATH] = {0};
        log_print(0, "[SNAPSHOT] Поиск устройства для %s через ядро (sysfs)...\n", partname);
        
        if (get_or_create_block_device(partname, dev_path, sizeof(dev_path))) {
            char dst[MAX_PATH];
            snprintf(dst, sizeof(dst), "%s/vendor_boot.img", VENDOR_BOOT_SNAP);
            log_print(0, "[SNAPSHOT] Найдено: %s. Копируем %s -> %s\n", partname, dev_path, dst);
            
            if (copy_file(dev_path, dst) == 0) {
                log_print(0, "[SNAPSHOT] УСПЕХ: vendor_boot успешно скопирован.\n");
            } else {
                log_print(1, "[SNAPSHOT] ОШИБКА: Сбой при копировании %s!\n", dev_path);
            }
        } else {
            log_print(1, "[SNAPSHOT] ОШИБКА: Не удалось найти раздел %s в sysfs!\n", partname);
        }
    } else {
        log_print(0, "[SNAPSHOT] ПРЕДУПРЕЖДЕНИЕ: Слот всё ещё не определен. Дампим оба варианта (_a и _b)...\n");
        const char *slots[] = {"_a", "_b"};
        int found_any = 0;
        for (int i = 0; i < 2; i++) {
            char partname[64];
            snprintf(partname, sizeof(partname), "vendor_boot%s", slots[i]);
            
            char dev_path[MAX_PATH] = {0};
            if (get_or_create_block_device(partname, dev_path, sizeof(dev_path))) {
                found_any = 1;
                char dst[MAX_PATH];
                snprintf(dst, sizeof(dst), "%s/vendor_boot%s.img", VENDOR_BOOT_SNAP, slots[i]);
                log_print(0, "[SNAPSHOT] Копирование %s (%s) -> %s\n", partname, dev_path, dst);
                if (copy_file(dev_path, dst) == 0) {
                    log_print(0, "[SNAPSHOT] УСПЕХ: vendor_boot%s.img скопирован\n", slots[i]);
                } else {
                    log_print(1, "[SNAPSHOT] ОШИБКА копирования %s\n", dev_path);
                }
            } else {
                log_print(0, "[SNAPSHOT] Раздел %s не найден в sysfs.\n", partname);
            }
        }
        if (!found_any) {
            log_print(1, "[SNAPSHOT] ОШИБКА: Ни один из слотов vendor_boot не был найден!\n");
        }
    }
}

int main(int argc, char *argv[]) {
    init_logging();
    dump_debug_info();

    const char *manifest_path = (argc > 1) ? argv[1] : DEFAULT_MANIFEST;
    const char *snap_dir      = (argc > 2) ? argv[2] : DEFAULT_SNAP_DIR;

    FILE *f = fopen(manifest_path, "r");
    if (!f) {
        log_print(1, "[SNAPSHOT] Cannot open manifest: %s: %s\n",
                manifest_path, strerror(errno));
        if (log_fp) fclose(log_fp);
        return 1;
    }

    log_print(0, "[SNAPSHOT] Creating ramdisk snapshot in %s\n", snap_dir);
    mkdirs(snap_dir, 0755);

    char line[MAX_PATH];
    int ok_count = 0, fail_count = 0;

    while (fgets(line, sizeof(line), f)) {
        char *nl = strchr(line, '\n');
        if (nl) *nl = '\0';
        char *cr = strchr(line, '\r');
        if (cr) *cr = '\0';

        if (line[0] == '#' || line[0] == '\0') continue;

        char type;
        unsigned int perms, uid, gid;
        char path[1024];
        char dst[MAX_PATH];

        if (line[0] == 'l') {
            char *arrow = strstr(line, " -> ");
            if (!arrow) { fail_count++; continue; }
            *arrow = '\0';
            const char *link_target = arrow + 4;

            if (sscanf(line, "%c %o %u %u %1023s",
                        &type, &perms, &uid, &gid, path) != 5) {
                fail_count++;
                continue;
            }

            snprintf(dst, sizeof(dst), "%s%s", snap_dir, path);
            ensure_parent(dst);
            unlink(dst);

            if (symlink(link_target, dst) == 0) {
                apply_metadata(path, dst, type, (mode_t)perms, (uid_t)uid, (gid_t)gid);
                ok_count++;
            } else {
                log_print(1, "[SNAPSHOT] symlink %s -> %s: %s\n",
                        dst, link_target, strerror(errno));
                fail_count++;
            }
            continue;
        }

        if (sscanf(line, "%c %o %u %u %1023s",
                    &type, &perms, &uid, &gid, path) != 5) {
            fail_count++;
            continue;
        }

        snprintf(dst, sizeof(dst), "%s%s", snap_dir, path);

        if (type == 'd') {
            mkdirs(dst, (mode_t)perms);
            apply_metadata(path, dst, type, (mode_t)perms, (uid_t)uid, (gid_t)gid);
            ok_count++;
        } else if (type == 'f') {
            ensure_parent(dst);
            if (copy_file(path, dst) == 0) {
                apply_metadata(path, dst, type, (mode_t)perms, (uid_t)uid, (gid_t)gid);
                ok_count++;
            } else {
                fail_count++;
            }
        }
    }

    fclose(f);
    log_print(0, "[SNAPSHOT] Ramdisk Complete: %d entries copied, %d errors\n",
            ok_count, fail_count);

    snapshot_vendor_boot();

    if (log_fp) {
        fclose(log_fp);
    }

    return (fail_count > 0) ? 1 : 0;
}