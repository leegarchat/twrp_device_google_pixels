// Fit ladder (phase 2): does the payload fit the opposite slot, and what
// must die to make room. Pure logic over live LP numbers (liblp).

#pragma once

#include <cstdint>
#include <map>
#include <string>
#include <vector>

namespace hs {

struct LivePart {
    std::string full;   // e.g. system_a
    uint64_t size = 0;
    std::string group;  // live group name, may carry slot suffix
};

struct LiveState {
    uint64_t super_size = 0;
    // base name -> bytes, split by slot suffix vs booted slot
    std::map<std::string, uint64_t> current;
    std::map<std::string, uint64_t> target;
    std::string target_group;  // live group holding the target slot
    std::vector<LivePart> all;  // every LP resident (for unmap-all)
};

// slot_suffix like "_a"/"_b"; booted is 'a'|'b'.
bool QueryLive(const std::string& super, char booted, LiveState* out,
               std::string* err);

enum class FitPlan { kAsIs, kKill, kWipeAll, kImpossible };

struct FitResult {
    FitPlan plan = FitPlan::kAsIs;
    std::vector<std::string> kill;  // full names in the CURRENT slot
};

const char* FitCode(FitPlan p);
// required: (base name, expanded bytes) for one manifest group.
FitResult PlanGroup(const std::vector<std::pair<std::string, uint64_t>>& required,
                    const std::string& manifest_group, uint64_t budget,
                    const LiveState& live);

}  // namespace hs
