// Live LP state via liblp + the fit ladder (ported from the proven
// Rust planner, same math).

#include "plan.h"

#include <algorithm>
#include <cstdio>
#include <cstring>

#include <android-base/logging.h>
#include <liblp/liblp.h>

namespace hs {

static std::string BaseName(const std::string& full) {
    if (full.size() > 2) {
        std::string s = full.substr(full.size() - 2);
        if (s == "_a" || s == "_b")
            return full.substr(0, full.size() - 2);
    }
    return full;
}

static std::string CStr(const char* buf, size_t max) {
    size_t n = strnlen(buf, max);
    return std::string(buf, n);
}

bool QueryLive(const std::string& super, char booted, LiveState* out,
               std::string* err) {
    LiveState st;
    // Slots are strictly 0|1 (lptools_new convention: anything else is
    // rejected). Suffixes resolve separately from ro.boot.slot_suffix.
    for (uint32_t slot : {0u, 1u}) {
        fprintf(stderr, "DBG live: slot %u\n", slot);
        std::unique_ptr<android::fs_mgr::LpMetadata> md =
            android::fs_mgr::ReadMetadata(super, slot);
        fprintf(stderr, "DBG live: slot %u read %s\n", slot, md ? "ok" : "null");
        if (!md)
            continue;
        if (md->header.magic != LP_METADATA_HEADER_MAGIC) {
            if (err)
                *err = "bad LP magic";
            return false;
        }
        if (st.super_size == 0) {
            for (const auto& dev : md->block_devices)
                st.super_size += dev.size;
        }
        for (const auto& p : md->partitions) {
            std::string full = CStr(p.name, sizeof(p.name));
            std::string group =
                p.group_index < md->groups.size()
                    ? CStr(md->groups[p.group_index].name, 36)
                    : "";
            uint64_t sz = 0;
            for (uint32_t i = 0; i < p.num_extents; i++)
                sz += static_cast<uint64_t>(
                          md->extents[p.first_extent_index + i].num_sectors) *
                      LP_SECTOR_SIZE;
            LivePart lp{full, sz, group};
            st.all.push_back(lp);
            if (full.size() > 2 &&
                (full.compare(full.size() - 2, 2, "_a") == 0 ||
                 full.compare(full.size() - 2, 2, "_b") == 0)) {
                if (full.back() == booted)
                    st.current[BaseName(full)] = sz;
                else
                    st.target[BaseName(full)] = sz;
                if (full.back() != booted && st.target_group.empty() &&
                    !group.empty())
                    st.target_group = group;
            }
        }
    }
    if (st.super_size == 0) {
        if (err)
            *err = "no LP metadata found";
        return false;
    }
    fprintf(stderr, "DBG live: merging done\n");
    *out = st;
    return true;
}

const char* FitCode(FitPlan p) {
    switch (p) {
        case FitPlan::kAsIs:
            return "FITS_AS_IS";
        case FitPlan::kKill:
            return "KILL";
        case FitPlan::kWipeAll:
            return "WIPE_ALL";
        case FitPlan::kImpossible:
            return "IMPOSSIBLE";
    }
    return "?";
}

static constexpr uint64_t kLpReserve = 1 << 20;

FitResult PlanGroup(
    const std::vector<std::pair<std::string, uint64_t>>& required,
    const std::string& manifest_group, uint64_t budget,
    const LiveState& live) {
    (void)manifest_group;
    FitResult r;
    uint64_t need = 0;
    for (const auto& kv : required)
        need += kv.second;
    if (need + kLpReserve > live.super_size) {
        r.plan = FitPlan::kImpossible;
        return r;
    }
    if (need > budget) {
        r.plan = FitPlan::kImpossible;
        return r;
    }
    uint64_t used = 0;
    for (const auto& kv : live.current)
        used += kv.second;
    for (const auto& kv : live.target)
        used += kv.second;
    uint64_t free = live.super_size > used ? live.super_size - used : 0;

    // Rebuilding the target group reclaims its current residents.
    uint64_t reclaim = 0;
    for (const auto& rq : required) {
        auto it = live.target.find(rq.first);
        if (it != live.target.end())
            reclaim += it->second;
    }
    uint64_t avail = free + reclaim;
    if (need <= avail) {
        r.plan = FitPlan::kAsIs;
        return r;
    }
    // Greedy kill in the current slot, largest first.
    std::vector<std::pair<std::string, uint64_t>> cur(live.current.begin(),
                                                      live.current.end());
    std::sort(cur.begin(), cur.end(),
              [](const auto& a, const auto& b) { return a.second > b.second; });
    uint64_t deficit = need - avail;
    for (const auto& kv : cur) {
        if (deficit == 0)
            break;
        r.kill.push_back(kv.first);
        deficit = deficit > kv.second ? deficit - kv.second : 0;
    }
    if (deficit > 0) {
        r.plan = FitPlan::kWipeAll;
        r.kill.clear();
        return r;
    }
    r.plan = FitPlan::kKill;
    return r;
}

}  // namespace hs
