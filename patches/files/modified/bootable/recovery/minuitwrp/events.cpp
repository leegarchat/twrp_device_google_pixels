/*
 * Copyright (C) 2007 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <dirent.h>
#include <sys/poll.h>
#include <limits.h>
#include <linux/input.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <unistd.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <fstream>
#include <atomic>
#include <cstdint>
#include <mutex>
#include <cutils/properties.h>
#include <thread>

#ifdef USE_QTI_HAPTICS
#include <android/hardware/vibrator/1.2/IVibrator.h>
#endif

#ifdef USE_QTI_AIDL_HAPTICS
#include <aidl/android/hardware/vibrator/IVibrator.h>
#include <android/binder_manager.h>
using ::aidl::android::hardware::vibrator::IVibrator;
#ifdef USE_QTI_AIDL_HAPTICS_FQNAME
static const std::string kVibratorInstance = std::string("android.hardware.vibrator.") + USE_QTI_AIDL_HAPTICS_FQNAME;
#else
static const std::string kVibratorInstance = std::string(IVibrator::descriptor) + "/default";
#endif
#ifdef USE_QTI_AIDL_HAPTICS_FIX_OFF
static std::atomic_int vib_on_count = 0;
#endif
#endif

#include "common.h"
#include "twcommon.h"

#include "minuitwrp/minui.h"

//#define _EVENT_LOGGING

#define MAX_DEVICES         32

#define VIBRATOR_TIMEOUT_FILE	"/sys/class/timed_output/vibrator/enable"
#define VIBRATOR_TIME_MS    50

#define LEDS_HAPTICS_DURATION_FILE	"/sys/class/leds/vibrator/duration"
#define LEDS_HAPTICS_ACTIVATE_FILE	"/sys/class/leds/vibrator/activate"
#define LEDS_HAPTICS_BRIGHTNESS_FILE	"/sys/class/leds/vibrator/brightness"

#ifndef SYN_REPORT
#define SYN_REPORT          0x00
#endif
#ifndef SYN_CONFIG
#define SYN_CONFIG          0x01
#endif
#ifndef SYN_MT_REPORT
#define SYN_MT_REPORT       0x02
#endif

#define ABS_MT_POSITION     0x2a /* Group a set of X and Y */
#define ABS_MT_AMPLITUDE    0x2b /* Group a set of Z and W */
#define ABS_MT_SLOT         0x2f
#define ABS_MT_TOUCH_MAJOR  0x30
#define ABS_MT_TOUCH_MINOR  0x31
#define ABS_MT_WIDTH_MAJOR  0x32
#define ABS_MT_WIDTH_MINOR  0x33
#define ABS_MT_ORIENTATION  0x34
#define ABS_MT_POSITION_X   0x35
#define ABS_MT_POSITION_Y   0x36
#define ABS_MT_TOOL_TYPE    0x37
#define ABS_MT_BLOB_ID      0x38
#define ABS_MT_TRACKING_ID  0x39
#define ABS_MT_PRESSURE     0x3a
#define ABS_MT_DISTANCE     0x3b

enum {
    DOWN_NOT,
    DOWN_SENT,
    DOWN_RELEASED,
};

struct virtualkey {
    int scancode;
    int centerx, centery;
    int width, height;
};

struct position {
    int x, y;
    int synced;
    struct input_absinfo xi, yi;
};

struct ev {
    struct pollfd *fd;

    struct virtualkey *vks;
    int vk_count;

    char deviceName[64];

    int ignored;

    struct position p, mt_p;
    int down;
};

static struct pollfd ev_fds[MAX_DEVICES];
static struct ev evs[MAX_DEVICES];
static unsigned ev_count = 0;
static struct timeval lastInputStat;
// Full-resolution mtime: second-granular time_t misses input nodes created
// in the same second as ev_init (late-probing touch on 6.12 never rescanned).
static struct timespec lastInputMTime;
static int has_mouse = 0;

static inline int ABS(int x) {
    return x<0?-x:x;
}

int write_to_file(const std::string& fn, const std::string& line) {
    FILE *file = fopen(fn.c_str(), "w");
    if (file == NULL) {
        LOGE("Failed to open %s for writing: %s\n", fn.c_str(), strerror(errno));
        return -1;
    }

    errno = 0;
    const size_t written = fwrite(line.c_str(), 1, line.size(), file);
    const int write_errno = errno;
    errno = 0;
    const int close_status = fclose(file);
    const int close_errno = errno;
    if (written != line.size() || close_status != 0) {
        int error = write_errno ? write_errno : close_errno;
        if (error == 0)
            error = EIO;
        LOGE("Failed to write %s: %s\n", fn.c_str(), strerror(error));
        return -1;
    }
    return 0;
}

#ifndef TW_NO_HAPTICS
#ifndef TW_HAPTICS_TSPDRV

static void ff_wake_device(const char *sysfs_path)
{
    /* Walk up from /sys/.../input/inputN to the parent I2C device and
       disable runtime PM autosuspend so the haptic chip stays awake. */
    char link[256], resolved[256], pm_ctrl[256];
    snprintf(link, sizeof(link), "%s/device", sysfs_path);
    ssize_t len = readlink(link, resolved, sizeof(resolved) - 1);
    if (len <= 0) {
        return;
    }
    resolved[len] = '\0';
    /* resolved is relative like "../../../0-0043", build absolute path */
    snprintf(pm_ctrl, sizeof(pm_ctrl), "%s/%s/power/control", sysfs_path, resolved);
    FILE *f = fopen(pm_ctrl, "w");
    if (f) { 
        fputs("on", f); 
        fclose(f); 
    } else {
        LOGE("[TRACE] ff_wake_device: failed to open %s\n", pm_ctrl);
    }
}

// Fox: per-device haptic strength. ro.recovery.vibro_scale is a percent
// of stock FF magnitude (100 = stock, stamped per device from
// devices/*/pixel.json props by recovery-pixel-boot; e.g. 50 halves the
// CS40L26 kick on malibu where stock is painfully strong). Absent or
// out of range = 100 (no change for other families). Cached: props are
// process-stable here.
static int fox_vibro_scale(void) {
    static int cached = -1;
    if (cached < 0) {
        char v[PROPERTY_VALUE_MAX];
        int n = 0;
        if (property_get("ro.recovery.vibro_scale", v, "") > 0)
            n = atoi(v);
        cached = (n > 0 && n <= 100) ? n : 100;
        if (cached != 100)
            printf("fox_vibro_scale: strength %d%% of stock\n", cached);
    }
    return cached;
}

// Fox: gs101 runtime detector for the dedicated haptics backend (same
// contract as is_gs101_family() in recovery-pixel-boot/otg.rs:
// soc_family prop primary, codename fallback). Cached, props are
// process-stable here.
static int fox_is_gs101(void) {
    static int cached = -1;
    if (cached < 0) {
        char f[PROPERTY_VALUE_MAX];
        char h[PROPERTY_VALUE_MAX];
        property_get("ro.recovery.soc_family", f, "");
        property_get("ro.hardware", h, "");
        cached = (strcmp(f, "gs101") == 0 ||
                  strcmp(h, "oriole") == 0 || strcmp(h, "raven") == 0 ||
                  strcmp(h, "bluejay") == 0) ? 1 : 0;
    }
    return cached;
}

static int vibrate_ff(int timeout_ms)
{
    static int ff_fd = -1;
    static int effect_id = -1;
    if (ff_fd < 0) {
        char path[64], syspath[128];
        for (int i = 0; i < 10; i++) {
            snprintf(path, sizeof(path), "/dev/input/event%d", i);
            int fd = open(path, O_RDWR);
            if (fd < 0) continue;

            unsigned long ff_bits[4] = {};
            if (ioctl(fd, EVIOCGBIT(EV_FF, sizeof(ff_bits)), ff_bits) >= 0 &&
                (ff_bits[FF_PERIODIC / (8 * sizeof(unsigned long))] &
                 (1UL << (FF_PERIODIC % (8 * sizeof(unsigned long)))))) {
                ff_fd = fd;
                snprintf(syspath, sizeof(syspath), "/sys/class/input/input%d", i);
                ff_wake_device(syspath);
                break;
            }
            close(fd);
        }
        if (ff_fd < 0) {
            LOGE("[TRACE] vibrate_ff: FF device NOT FOUND! Returning -1\n");
            return -1;
        }
    }

    /* Stop previous effect if still playing */
    if (effect_id >= 0) {
        struct input_event stop = {};
        stop.type = EV_FF;
        stop.code = effect_id;
        stop.value = 0;
        write(ff_fd, &stop, sizeof(stop));
    }

    struct ff_effect effect = {};
    effect.type = FF_PERIODIC;
    effect.id = effect_id;  /* reuse slot or -1 for first upload */
    effect.replay.length = timeout_ms;
    effect.replay.delay = 0;
    effect.u.periodic.waveform = FF_SINE;
    effect.u.periodic.period = 10;
    effect.u.periodic.magnitude = (0x7fff * fox_vibro_scale()) / 100;

    if (ioctl(ff_fd, EVIOCSFF, &effect) < 0) {
        LOGE("[TRACE] vibrate_ff: ioctl EVIOCSFF failed!\n");
        return -1;
    }

    effect_id = effect.id;

    struct input_event play = {};
    play.type = EV_FF;
    play.code = effect_id;
    play.value = 1;
    ssize_t ret = write(ff_fd, &play, sizeof(play));

    /* Do NOT remove the effect — let it play for replay.length ms.
       CS40L26 uses I2C work queue; immediate EVIOCRMFF would cancel
       the vibration before the chip processes the play command. */

    return (ret < 0) ? -1 : 0;
}

// The gs101 transient trigger's activate attribute is readable but not
// writable on affected recovery builds. Control the LED through its writable
// brightness attribute instead, then force it off after the requested pulse.
// A generation counter prevents an older timer from cutting off a newer tap.
static std::mutex gs101_haptics_mutex;
static std::uint64_t gs101_haptics_generation = 0;

static int vibrate_gs101_brightness(int timeout_ms)
{
    if (timeout_ms <= 0)
        return 0;

    std::uint64_t generation;
    {
        std::lock_guard<std::mutex> lock(gs101_haptics_mutex);
        if (write_to_file(LEDS_HAPTICS_BRIGHTNESS_FILE, "255") != 0)
            return -1;
        generation = ++gs101_haptics_generation;
    }

    std::thread([generation, timeout_ms] {
        usleep(timeout_ms * 1000);
        std::lock_guard<std::mutex> lock(gs101_haptics_mutex);
        if (generation == gs101_haptics_generation)
            write_to_file(LEDS_HAPTICS_BRIGHTNESS_FILE, "0");
    }).detach();
    return 0;
}

int vibrate(int timeout_ms)
{
    if (timeout_ms > 10000) timeout_ms = 1000;
    char tout[6];
    sprintf(tout, "%i", timeout_ms);

#ifdef USE_QTI_HAPTICS
    android::sp<android::hardware::vibrator::V1_2::IVibrator> vib = android::hardware::vibrator::V1_2::IVibrator::getService();
    if (vib != nullptr) {
        vib->on((uint32_t)timeout_ms);
    }
#elif defined(USE_QTI_AIDL_HAPTICS)
    std::shared_ptr<IVibrator> vib = IVibrator::fromBinder(ndk::SpAIBinder(AServiceManager_getService(kVibratorInstance.c_str())));
    if (vib != nullptr) {
#ifdef USE_QTI_AIDL_HAPTICS_FIX_OFF
        std::thread([vib, timeout_ms] {
            if (vib->on((uint32_t)timeout_ms, nullptr).isOk()) {
                vib_on_count++;
                usleep(timeout_ms * 1000);
                vib_on_count--;
                if (!vib_on_count) vib->off();
            }
        }).detach();
#else
        vib->on((uint32_t)timeout_ms, nullptr);
#endif
    }
#elif defined(USE_SAMSUNG_HAPTICS)
    /* Newer Samsung devices have duration file only
     0 in VIBRATOR_TIMEOUT_FILE means no vibration
     Anything else is the vibration running for X milliseconds */
    if (std::ifstream(VIBRATOR_TIMEOUT_FILE).good()) {
        write_to_file(VIBRATOR_TIMEOUT_FILE, tout);
    }
#else
    // Backend is picked once (modules are up before the first tap) and
    // traced so flog shows which path a unit took:
    // 0=transient LEDS 1=timed_output 2=force feedback 3=gs101 brightness.
    static int vib_backend = -1;
    if (vib_backend < 0) {
        if (fox_is_gs101())
            vib_backend = 3;
        else if (std::ifstream(LEDS_HAPTICS_ACTIVATE_FILE).good() &&
            std::ifstream(LEDS_HAPTICS_DURATION_FILE).good())
            vib_backend = 0;
        else if (std::ifstream(VIBRATOR_TIMEOUT_FILE).good())
            vib_backend = 1;
        else
            vib_backend = 2;
        LOGE("[TRACE] vibrate: backend %d (0=leds 1=timed_output 2=ff 3=gs101-brightness)\n", vib_backend);
    }
    if (vib_backend == 3) {
        vibrate_gs101_brightness(timeout_ms);
    } else if (vib_backend == 0) {
        write_to_file(LEDS_HAPTICS_DURATION_FILE, tout);
        write_to_file(LEDS_HAPTICS_ACTIVATE_FILE, "1");
    } else if (vib_backend == 1) {
        write_to_file(VIBRATOR_TIMEOUT_FILE, tout);
    } else {
        vibrate_ff(timeout_ms);
    }
#endif
    return 0;
}
#endif
#endif

/* Returns empty tokens */
static char *vk_strtok_r(char *str, const char *delim, char **save_str)
{
    if(!str)
    {
        if(!*save_str)
            return NULL;

        str = (*save_str) + 1;
    }
    *save_str = strpbrk(str, delim);

    if (*save_str)
        **save_str = '\0';

    return str;
}

static int vk_init(struct ev *e)
{
    char vk_path[PATH_MAX] = "/sys/board_properties/virtualkeys.";
    char vks[2048], *ts = NULL;
    ssize_t len;
    int vk_fd;
    int i;

    e->vk_count = 0;

    len = strlen(vk_path);
    len = ioctl(e->fd->fd, EVIOCGNAME(sizeof(e->deviceName)), e->deviceName);
    if (len <= 0)
    {
        LOGE("Unable to query event object.\n");
        return -1;
    }
#ifdef _EVENT_LOGGING
    printf("Event object: %s\n", e->deviceName);
#endif

#ifdef WHITELIST_INPUT
    if (strcmp(e->deviceName, EXPAND(WHITELIST_INPUT)) != 0)
    {
        e->ignored = 1;
    }
#else
#ifndef TW_INPUT_BLACKLIST
    // Blacklist these "input" devices, use TW_INPUT_BLACKLIST := "accelerometer\x0atest1\x0atest2" using the \x0a as a separator between input devices
    if (strcmp(e->deviceName, "bma250") == 0 || strcmp(e->deviceName, "bma150") == 0)
    {
        LOGI("Blacklisting input device: %s\n", e->deviceName);
        e->ignored = 1;
    }
#else
    char* bl = strdup(EXPAND(TW_INPUT_BLACKLIST));
    char* blacklist = strtok(bl, "\n");

    while (blacklist != NULL) {
        if (strcmp(e->deviceName, blacklist) == 0) {
            LOGI("Blacklisting input device: %s\n", blacklist);
            e->ignored = 1;
        }
        blacklist = strtok(NULL, "\n");
    }
    free(bl);
#endif
#endif

    strcat(vk_path, e->deviceName);

    // Some devices split the keys from the touchscreen
    e->vk_count = 0;
    vk_fd = open(vk_path, O_RDONLY);
    if (vk_fd >= 0)
    {
        len = read(vk_fd, vks, sizeof(vks)-1);
        close(vk_fd);
        if (len <= 0)
            return -1;

        vks[len] = '\0';

        /* Parse a line like:
            keytype:keycode:centerx:centery:width:height:keytype2:keycode2:centerx2:...
        */
        for (ts = vks, e->vk_count = 1; *ts; ++ts) {
            if (*ts == ':')
                ++e->vk_count;
        }

        if (e->vk_count % 6) {
            LOGI("minui: %s is %d %% 6\n", vk_path, e->vk_count % 6);
        }
        e->vk_count /= 6;
        if (e->vk_count <= 0)
            return -1;

        e->down = DOWN_NOT;
    }

    ioctl(e->fd->fd, EVIOCGABS(ABS_X), &e->p.xi);
    ioctl(e->fd->fd, EVIOCGABS(ABS_Y), &e->p.yi);
    e->p.synced = 0;
#ifdef _EVENT_LOGGING
    printf("EV: ST minX: %d  maxX: %d  minY: %d  maxY: %d\n", e->p.xi.minimum, e->p.xi.maximum, e->p.yi.minimum, e->p.yi.maximum);
#endif

    ioctl(e->fd->fd, EVIOCGABS(ABS_MT_POSITION_X), &e->mt_p.xi);
    ioctl(e->fd->fd, EVIOCGABS(ABS_MT_POSITION_Y), &e->mt_p.yi);
    e->mt_p.synced = 0;
#ifdef _EVENT_LOGGING
    printf("EV: MT minX: %d  maxX: %d  minY: %d  maxY: %d\n", e->mt_p.xi.minimum, e->mt_p.xi.maximum, e->mt_p.yi.minimum, e->mt_p.yi.maximum);
#endif

    e->vks = (virtualkey *)malloc(sizeof(*e->vks) * e->vk_count);

    for (i = 0; i < e->vk_count; ++i) {
        char *token[6];
        int j;

        for (j = 0; j < 6; ++j) {
            token[j] = vk_strtok_r((i||j)?NULL:vks, ":", &ts);
        }

        if (strcmp(token[0], "0x01") != 0) {
            /* Java does string compare, so we do too. */
            LOGI("minui: %s: ignoring unknown virtual key type %s\n", vk_path, token[0]);
            continue;
        }

        e->vks[i].scancode = strtol(token[1], NULL, 0);
        e->vks[i].centerx = strtol(token[2], NULL, 0);
        e->vks[i].centery = strtol(token[3], NULL, 0);
        e->vks[i].width = strtol(token[4], NULL, 0);
        e->vks[i].height = strtol(token[5], NULL, 0);
    }

    return 0;
}

#define BITS_PER_LONG (sizeof(long) * 8)
#define NBITS(x) ((((x)-1)/BITS_PER_LONG)+1)
#define OFF(x)  ((x)%BITS_PER_LONG)
#define LONG(x) ((x)/BITS_PER_LONG)
#define test_bit(bit, array)	((array[LONG(bit)] >> OFF(bit)) & 1)

// Check for EV_REL (REL_X and REL_Y) and, because touchscreens can have those too,
// check also for EV_KEY (BTN_LEFT and BTN_RIGHT)
static void check_mouse(int fd, const char* deviceName)
{
	if(has_mouse)
		return;

	unsigned long bit[EV_MAX][NBITS(KEY_MAX)];
	memset(bit, 0, sizeof(bit));
	ioctl(fd, EVIOCGBIT(0, EV_MAX), bit[0]);

	if(!test_bit(EV_REL, bit[0]) || !test_bit(EV_KEY, bit[0]))
		return;

	ioctl(fd, EVIOCGBIT(EV_REL, KEY_MAX), bit[EV_REL]);
	if(!test_bit(REL_X, bit[EV_REL]) || !test_bit(REL_Y, bit[EV_REL]))
		return;

	ioctl(fd, EVIOCGBIT(EV_KEY, KEY_MAX), bit[EV_KEY]);
	if(!test_bit(BTN_LEFT, bit[EV_KEY]) || !test_bit(BTN_RIGHT, bit[EV_KEY]))
		return;

	LOGI("Found mouse '%s'\n", deviceName);
	has_mouse = 1;
}

int ev_has_mouse(void)
{
	return has_mouse;
}

int ev_init(void)
{
    DIR *dir;
    struct dirent *de;
    int fd;

    has_mouse = 0;

	dir = opendir("/dev/input");
    if(dir != 0) {
        while((de = readdir(dir))) {
#ifdef _EVENT_LOGGING
            fprintf(stderr,"/dev/input/%s\n", de->d_name);
#endif
            if(strncmp(de->d_name,"event",5)) continue;
            fd = openat(dirfd(dir), de->d_name, O_RDONLY);
            if(fd < 0) continue;

			ev_fds[ev_count].fd = fd;
            ev_fds[ev_count].events = POLLIN;
            evs[ev_count].fd = &ev_fds[ev_count];

            /* Load virtualkeys if there are any */
            vk_init(&evs[ev_count]);

            if (!evs[ev_count].ignored)
                check_mouse(fd, evs[ev_count].deviceName);

            ev_count++;
            if(ev_count == MAX_DEVICES) break;
        }
        closedir(dir);
    }

    struct stat st;
    if(stat("/dev/input", &st) >= 0)
        lastInputMTime = st.st_mtim;
    gettimeofday(&lastInputStat, NULL);

    return 0;
}

void ev_exit(void)
{
	while (ev_count-- > 0) {
		if (evs[ev_count].vk_count) {
			free(evs[ev_count].vks);
			evs[ev_count].vk_count = 0;
		}
		close(ev_fds[ev_count].fd);
	}
	ev_count = 0;
}

/*static int vk_inside_display(__s32 value, struct input_absinfo *info, int screen_size)
{
    int screen_pos;

    if (info->minimum == info->maximum)
        return 0;

    screen_pos = (value - info->minimum) * (screen_size - 1) / (info->maximum - info->minimum);
    return (screen_pos >= 0 && screen_pos < screen_size);
}*/

static int vk_tp_to_screen(struct position *p, int *x, int *y)
{
    if (p->xi.minimum == p->xi.maximum || p->yi.minimum == p->yi.maximum)
    {
        // In this case, we assume the screen dimensions are the same.
        *x = p->x;
        *y = p->y;
        return 0;
    }

#ifdef _EVENT_LOGGING
    printf("EV: p->x=%d  x-range=%d,%d  fb-width=%d\n", p->x, p->xi.minimum, p->xi.maximum, gr_fb_width());
#endif

#ifndef RECOVERY_TOUCHSCREEN_SWAP_XY
    int fb_width = gr_fb_width();
    int fb_height = gr_fb_height();
#else
    // We need to swap the scaling sizes, too
    int fb_width = gr_fb_height();
    int fb_height = gr_fb_width();
#endif

    *x = (p->x - p->xi.minimum) * (fb_width - 1) / (p->xi.maximum - p->xi.minimum);
    *y = (p->y - p->yi.minimum) * (fb_height - 1) / (p->yi.maximum - p->yi.minimum);

    if (*x >= 0 && *x < fb_width &&
        *y >= 0 && *y < fb_height)
    {
        return 0;
    }

    return 1;
}

/* Translate a virtual key in to a real key event, if needed */
/* Returns non-zero when the event should be consumed */
static int vk_modify(struct ev *e, struct input_event *ev)
{
    static int downX = -1, downY = -1;
    static int discard = 0;
    static int last_virt_key = 0;
    static int lastWasSynReport = 0;
    static int touchReleaseOnNextSynReport = 0;
	static int use_tracking_id_negative_as_touch_release = 0; // On some devices, type: 3  code: 39  value: -1, aka EV_ABS ABS_MT_TRACKING_ID -1 indicates a true touch release
    int i;
    int x, y;

    // This is used to ditch useless event handlers, like an accelerometer
    if (e->ignored)     return 1;

    if (ev->type == EV_REL && ev->code == REL_Z)
    {
        // This appears to be an accelerometer or another strange input device. It's not the touchscreen.
#ifdef _EVENT_LOGGING
        printf("EV: Device disabled due to non-touchscreen messages.\n");
#endif
        e->ignored = 1;
        return 1;
    }

#ifdef _EVENT_LOGGING
    printf("EV: %s => type: %x  code: %x  value: %d\n", e->deviceName, ev->type, ev->code, ev->value);
#endif

	// Handle keyboard events, value of 1 indicates key down, 0 indicates key up
	if (ev->type == EV_KEY) {
		return 0;
	}

    if (ev->type == EV_ABS) {
        switch (ev->code) {

        case ABS_X: //00
            e->p.synced |= 0x01;
            e->p.x = ev->value;
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS  ABS_X  %d\n", e->deviceName, ev->value);
#endif
            break;

        case ABS_Y: //01
            e->p.synced |= 0x02;
            e->p.y = ev->value;
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS  ABS_Y  %d\n", e->deviceName, ev->value);
#endif
            break;

        case ABS_MT_POSITION: //2a
            e->mt_p.synced = 0x03;
            if (ev->value == (1 << 31))
            {
#ifndef TW_IGNORE_MT_POSITION_0
                e->mt_p.x = 0;
                e->mt_p.y = 0;
                lastWasSynReport = 1;
#endif
#ifdef _EVENT_LOGGING
#ifndef TW_IGNORE_MT_POSITION_0
                printf("EV: %s => EV_ABS  ABS_MT_POSITION  %d, set x and y to 0 and lastWasSynReport to 1\n", e->deviceName, ev->value);
#else
                printf("Ignoring ABS_MT_POSITION 0\n", e->deviceName, ev->value);
#endif
#endif
            }
            else
            {
                lastWasSynReport = 0;
                e->mt_p.x = (ev->value & 0x7FFF0000) >> 16;
                e->mt_p.y = (ev->value & 0xFFFF);
#ifdef _EVENT_LOGGING
                printf("EV: %s => EV_ABS  ABS_MT_POSITION  %d, set x: %d and y: %d and lastWasSynReport to 0\n", e->deviceName, ev->value, (ev->value & 0x7FFF0000) >> 16, (ev->value & 0xFFFF));
#endif
            }
            break;

        case ABS_MT_TOUCH_MAJOR: //30
            if (ev->value == 0)
            {
#ifndef TW_IGNORE_MAJOR_AXIS_0
                // We're in a touch release, although some devices will still send positions as well
                e->mt_p.x = 0;
                e->mt_p.y = 0;
                touchReleaseOnNextSynReport = 1;
#endif
            }
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS  ABS_MT_TOUCH_MAJOR  %d\n", e->deviceName, ev->value);
#endif
            break;

		case ABS_MT_PRESSURE: //3a
                    if (ev->value == 0)
            {
                // We're in a touch release, although some devices will still send positions as well
                e->mt_p.x = 0;
                e->mt_p.y = 0;
                touchReleaseOnNextSynReport = 1;
            }
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS  ABS_MT_PRESSURE  %d\n", e->deviceName, ev->value);
#endif
            break;

		case ABS_MT_POSITION_X: //35
            e->mt_p.synced |= 0x01;
            e->mt_p.x = ev->value;
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS  ABS_MT_POSITION_X  %d\n", e->deviceName, ev->value);
#endif
            break;

        case ABS_MT_POSITION_Y: //36
            e->mt_p.synced |= 0x02;
            e->mt_p.y = ev->value;
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS  ABS_MT_POSITION_Y  %d\n", e->deviceName, ev->value);
#endif
            break;

        case ABS_MT_TOUCH_MINOR: //31
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS ABS_MT_TOUCH_MINOR %d\n", e->deviceName, ev->value);
#endif
            break;

        case ABS_MT_WIDTH_MAJOR: //32
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS ABS_MT_WIDTH_MAJOR %d\n", e->deviceName, ev->value);
#endif
            break;

        case ABS_MT_WIDTH_MINOR: //33
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS ABS_MT_WIDTH_MINOR %d\n", e->deviceName, ev->value);
#endif
            break;

        case ABS_MT_TRACKING_ID: //39
#ifdef TW_IGNORE_ABS_MT_TRACKING_ID
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS ABS_MT_TRACKING_ID %d ignored\n", e->deviceName, ev->value);
#endif
            return 1;
#endif
            if (ev->value < 0) {
                e->mt_p.x = 0;
                e->mt_p.y = 0;
                touchReleaseOnNextSynReport = 2;
                use_tracking_id_negative_as_touch_release = 1;
#ifdef _EVENT_LOGGING
                if (use_tracking_id_negative_as_touch_release)
                    printf("using ABS_MT_TRACKING_ID value -1 to indicate touch releases\n");
#endif
            }
#ifdef _EVENT_LOGGING
            printf("EV: %s => EV_ABS ABS_MT_TRACKING_ID %d\n", e->deviceName, ev->value);
#endif
            break;

#ifdef _EVENT_LOGGING
        // These are for touch logging purposes only
        case ABS_MT_ORIENTATION: //34
            printf("EV: %s => EV_ABS ABS_MT_ORIENTATION %d\n", e->deviceName, ev->value);
			return 1;
            break;

		case ABS_MT_TOOL_TYPE: //37
            LOGI("EV: %s => EV_ABS ABS_MT_TOOL_TYPE %d\n", e->deviceName, ev->value);
			return 1;
            break;

        case ABS_MT_BLOB_ID: //38
            printf("EV: %s => EV_ABS ABS_MT_BLOB_ID %d\n", e->deviceName, ev->value);
			return 1;
            break;

		case ABS_MT_DISTANCE: //3b
            printf("EV: %s => EV_ABS ABS_MT_DISTANCE %d\n", e->deviceName, ev->value);
			return 1;
            break;
        case ABS_MT_SLOT:
            printf("EV: %s => ABS_MT_SLOT %d\n", e->deviceName, ev->value);
			return 1;
			break;
#endif

        default:
            // This is an unhandled message, just skip it
            return 1;
        }

        if (ev->code != ABS_MT_POSITION)
        {
            lastWasSynReport = 0;
            return 1;
        }
    }

    // Check if we should ignore the message
    if (ev->code != ABS_MT_POSITION && (ev->type != EV_SYN || (ev->code != SYN_REPORT && ev->code != SYN_MT_REPORT)))
    {
        lastWasSynReport = 0;
        return 0;
    }

#ifdef _EVENT_LOGGING
    if (ev->type == EV_SYN && ev->code == SYN_REPORT)
        printf("EV: %s => EV_SYN  SYN_REPORT\n", e->deviceName);
    if (ev->type == EV_SYN && ev->code == SYN_MT_REPORT)
        printf("EV: %s => EV_SYN  SYN_MT_REPORT\n", e->deviceName);
#endif

    // Discard the MT versions
    if (ev->code == SYN_MT_REPORT)      return 0;

    if (((lastWasSynReport == 1 || touchReleaseOnNextSynReport == 1) && !use_tracking_id_negative_as_touch_release) || (use_tracking_id_negative_as_touch_release && touchReleaseOnNextSynReport == 2))
    {
        // Reset the value
        touchReleaseOnNextSynReport = 0;

        // We are a finger-up state
        if (!discard)
        {
            // Report the key up
            ev->type = EV_ABS;
            ev->code = 0;
            ev->value = (downX << 16) | downY;
        }
        downX = -1;
        downY = -1;
        if (discard)
        {
            discard = 0;

            // Send the keyUp event
            ev->type = EV_KEY;
            ev->code = last_virt_key;
            ev->value = 0;
        }
        return 0;
    }
    lastWasSynReport = 1;

    // Retrieve where the x,y position is
    if (e->p.synced & 0x03)
    {
        vk_tp_to_screen(&e->p, &x, &y);
    }
    else if (e->mt_p.synced & 0x03)
    {
        vk_tp_to_screen(&e->mt_p, &x, &y);
    }
    else
    {
        // We don't have useful information to convey
        return 1;
    }

#ifdef RECOVERY_TOUCHSCREEN_SWAP_XY
    x ^= y;
    y ^= x;
    x ^= y;
#endif
#ifdef RECOVERY_TOUCHSCREEN_FLIP_X
    x = gr_fb_width() - x;
#endif
#ifdef RECOVERY_TOUCHSCREEN_FLIP_Y
    y = gr_fb_height() - y;
#endif

#ifdef _EVENT_LOGGING
    printf("EV: x: %d  y: %d\n", x, y);
#endif

    // Clear the current sync states
    e->p.synced = e->mt_p.synced = 0;

    // If we have nothing useful to report, skip it
    if (x == -1 || y == -1)     return 1;

    // Special case, we'll ignore touches on 0,0 because it usually means
    // that we received extra data after our last sync and x and y were
    // reset to 0. We should not be using 0,0 anyway.
    if (x == 0 && y == 0)
        return 1;

    // On first touch, see if we're at a virtual key
    if (downX == -1)
    {
        // Attempt mapping to virtual key
        for (i = 0; i < e->vk_count; ++i)
        {
            int xd = ABS(e->vks[i].centerx - x);
            int yd = ABS(e->vks[i].centery - y);

            if (xd < e->vks[i].width/2 && yd < e->vks[i].height/2)
            {
                ev->type = EV_KEY;
                ev->code = e->vks[i].scancode;
                ev->value = 1;

                last_virt_key = e->vks[i].scancode;

#ifndef TW_NO_HAPTICS
                vibrate(VIBRATOR_TIME_MS);
#endif

                // Mark that all further movement until lift is discard,
                // and make sure we don't come back into this area
                discard = 1;
                downX = 0;
                return 0;
            }
        }
    }

    // If we were originally a button press, discard this event
    if (discard)
    {
        return 1;
    }

    // Record where we started the touch for deciding if this is a key or a scroll
    downX = x;
    downY = y;

    ev->type = EV_ABS;
    ev->code = 1;
    ev->value = (x << 16) | y;
    return 0;
}

int ev_get(struct input_event *ev, int timeout_ms)
{
    int r;
    unsigned n;
    struct timeval curr;

    gettimeofday(&curr, NULL);
    if(curr.tv_sec - lastInputStat.tv_sec >= 2)
    {
        struct stat st;
        stat("/dev/input", &st);
        if (st.st_mtim.tv_sec > lastInputMTime.tv_sec ||
            (st.st_mtim.tv_sec == lastInputMTime.tv_sec &&
             st.st_mtim.tv_nsec > lastInputMTime.tv_nsec))
        {
            LOGI("Reloading input devices\n");
            ev_exit();
            ev_init();
            lastInputMTime = st.st_mtim;
        }
        lastInputStat = curr;
    }

    r = poll(ev_fds, ev_count, timeout_ms);

    if(r > 0) {
        for(n = 0; n < ev_count; n++) {
            if(ev_fds[n].revents & POLLIN) {
                r = read(ev_fds[n].fd, ev, sizeof(*ev));
                if(r == sizeof(*ev)) {
                    if (!vk_modify(&evs[n], ev))
                        return 0;
                }
            }
        }
        return -1;
    }

    return -2;
}

int ev_wait(int timeout __unused)
{
    return -1;
}

void ev_dispatch(void)
{
    return;
}

int ev_get_input(int fd __unused, short revents __unused, struct input_event *ev __unused)
{
    return -1;
}
