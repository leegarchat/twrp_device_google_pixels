// fox-hsinstall: high-speed OTA installer engine (recovery, static).
//
// Exit contract (read by the twinstall caller):
//   0 = success (slot is switched by the caller, never by the engine)
//   1 = install FAILED (rollback attempted, slot NOT switched)
//   2 = not a full OTA -> caller takes the stock update_engine path
//   3 = IMPOSSIBLE fit -> hard error, nothing was written
//   4 = DEVICE MISMATCH (pre-device vs ro.product.device) -> hard error
//   5 = live super unreadable/unusable -> caller takes the stock path
//   6 = broken invocation (usage) -> hard error, never a fallback
//
// Wire protocol (stdout, one line per event, flushed immediately):
//   progress <frac> <secs> / set_progress <frac> / ui_print <text> / log <text>
// Anything else on stdout is console text. Mirrors the reflash_twrp.sh
// pattern: the caller relays every line to the GUI live.

#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace hs {

constexpr int kExitOk = 0;
constexpr int kExitFail = 1;
constexpr int kExitNotFull = 2;
constexpr int kExitImpossible = 3;
constexpr int kExitDeviceMismatch = 4;
constexpr int kExitNoHelpers = 5;
constexpr int kExitUsage = 6;

// Full-OTA op allow-list (mirrors the peridot probe).
inline bool IsFullOtaOp(int type) {
    return type == 0 || type == 1 || type == 6 || type == 7 || type == 8 ||
           type == 14;
}
const char* OpName(int type);

// ---- wire output (stdout, flushed per line) ----
void UiPrint(const std::string& s);
void SetProgress(double frac);
void LogLine(const std::string& s);

// ---- tiny helpers ----
std::string GetProp(const std::string& name);
bool Mkdirs(const std::string& path);

}  // namespace hs
