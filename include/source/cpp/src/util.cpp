// Shared bits: progress/report output, props, misc.

#include "hs.h"

#include <cstdarg>
#include <cerrno>
#include <cstdio>
#include <string>
#include <sys/stat.h>
#include <sys/types.h>

#include <android-base/properties.h>

namespace hs {

const char* OpName(int type) {
    switch (type) {
        case 0:
            return "REPLACE";
        case 1:
            return "REPLACE_BZ";
        case 4:
            return "SOURCE_COPY";
        case 5:
            return "SOURCE_BSDIFF";
        case 6:
            return "ZERO";
        case 7:
            return "DISCARD";
        case 8:
            return "REPLACE_XZ";
        case 9:
            return "PUFFDIFF";
        case 10:
            return "BROTLI_BSDIFF";
        case 11:
            return "ZUCCHINI";
        case 12:
            return "LZ4DIFF_BSDIFF";
        case 13:
            return "LZ4DIFF_PUFFDIFF";
        case 14:
            return "REPLACE_ZSTD";
        default:
            return "UNKNOWN";
    }
}

static void Emit(const char* fmt, ...) {
    char buf[1024];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    fputs(buf, stdout);
    fflush(stdout);
}

void UiPrint(const std::string& s) {
    Emit("ui_print hs-install: %s\n", s.c_str());
}

void SetProgress(double frac) {
    if (frac < 0) frac = 0;
    if (frac > 1) frac = 1;
    Emit("set_progress %.4f\n", frac);
}

void LogLine(const std::string& s) {
    Emit("log hs-install: %s\n", s.c_str());
}

std::string GetProp(const std::string& name) {
    return android::base::GetProperty(name, "");
}

bool Mkdirs(const std::string& path) {
    if (mkdir(path.c_str(), 0755) == 0 || errno == EEXIST)
        return true;
    // best effort parents
    size_t pos = path.find('/', 1);
    while (pos != std::string::npos) {
        mkdir(path.substr(0, pos).c_str(), 0755);
        pos = path.find('/', pos + 1);
    }
    return mkdir(path.c_str(), 0755) == 0 || errno == EEXIST;
}

}  // namespace hs
