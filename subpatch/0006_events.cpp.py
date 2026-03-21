from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom vibrate logic (events.cpp)"
        self.target_file = "bootable/recovery/minuitwrp/events.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
int write_to_file(const std::string& fn, const std::string& line) {
	FILE *file;
	file = fopen(fn.c_str(), "w");
	if (file != NULL) {
		fwrite(line.c_str(), line.size(), 1, file);
		fclose(file);
		return 0;
	}
	LOGI("Cannot find file %s\n", fn.c_str());
	return -1;
}

#ifndef TW_NO_HAPTICS
#ifndef TW_HAPTICS_TSPDRV
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
""",
                # Блок 1: Модифицированный код (результат)
                r"""
int write_to_file(const std::string& fn, const std::string& line) {
    FILE *file;
    file = fopen(fn.c_str(), "w");
    if (file != NULL) {
        fwrite(line.c_str(), line.size(), 1, file);
        fclose(file);
        return 0;
    }
    LOGI("Cannot find file %s\n", fn.c_str());
    return -1;
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
    effect.u.periodic.magnitude = 0x7fff;

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
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
#elif defined(USE_SAMSUNG_HAPTICS)
    /* Newer Samsung devices have duration file only
     0 in VIBRATOR_TIMEOUT_FILE means no vibration
     Anything else is the vibration running for X milliseconds */
    if (std::ifstream(VIBRATOR_TIMEOUT_FILE).good()) {
        write_to_file(VIBRATOR_TIMEOUT_FILE, tout);
    }
#else
    if (std::ifstream(LEDS_HAPTICS_ACTIVATE_FILE).good()) {
        write_to_file(LEDS_HAPTICS_DURATION_FILE, tout);
        write_to_file(LEDS_HAPTICS_ACTIVATE_FILE, "1");
    } else
        write_to_file(VIBRATOR_TIMEOUT_FILE, tout);
#endif
    return 0;
}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
#elif defined(USE_SAMSUNG_HAPTICS)
    /* Newer Samsung devices have duration file only
     0 in VIBRATOR_TIMEOUT_FILE means no vibration
     Anything else is the vibration running for X milliseconds */
    if (std::ifstream(VIBRATOR_TIMEOUT_FILE).good()) {
        write_to_file(VIBRATOR_TIMEOUT_FILE, tout);
    }
#else
    if (std::ifstream(LEDS_HAPTICS_ACTIVATE_FILE).good()) {
        write_to_file(LEDS_HAPTICS_DURATION_FILE, tout);
        write_to_file(LEDS_HAPTICS_ACTIVATE_FILE, "1");
    } else if (std::ifstream(VIBRATOR_TIMEOUT_FILE).good()) {
        write_to_file(VIBRATOR_TIMEOUT_FILE, tout);
    } else {
        vibrate_ff(timeout_ms);
    }
#endif
    return 0;
}
"""
            )
        ]

