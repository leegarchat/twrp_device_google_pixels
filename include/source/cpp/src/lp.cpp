// Own LP surgery via liblp/libdm. Conventions mirror siw 0.2.4 +
// lptools_new: slots strictly 0|1, multi-slot removal, explicit
// metadata_slot on map, open_count (ACTIVE) guard on unmap.

#include "lp.h"

#include <dirent.h>
#include <mntent.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <cerrno>
#include <set>
#include <string>

#include <android-base/logging.h>
#include <libdm/dm.h>
#include <fs_mgr_dm_linear.h>
#include <liblp/builder.h>
#include <liblp/liblp.h>
#include <liblp/metadata_format.h>

namespace hs {

bool UnmountUnmapAll(const LiveState& live, std::vector<std::string>* gone,
                     std::string* err) {
    // pass 1: unmount everything sourced from /dev/block/mapper/* (or the
    // raw dm-N nodes), deepest mountpoints first.
    FILE* mtab = setmntent("/proc/mounts", "r");
    std::vector<std::string> mounts;
    if (mtab) {
        struct mntent* e;
        while ((e = getmntent(mtab)) != nullptr) {
            std::string src = e->mnt_fsname ? e->mnt_fsname : "";
            std::string mnt = e->mnt_dir ? e->mnt_dir : "";
            if ((src.compare(0, 19, "/dev/block/mapper/") == 0 ||
                 src.compare(0, 8, "/dev/dm-") == 0) &&
                !mnt.empty()) {
                mounts.push_back(mnt);
            }
        }
        endmntent(mtab);
    }
    std::sort(mounts.begin(), mounts.end(),
              [](const std::string& a, const std::string& b) {
                  return a.size() > b.size();
              });
    mounts.erase(std::unique(mounts.begin(), mounts.end()), mounts.end());
    for (const auto& m : mounts) {
        if (umount2(m.c_str(), 0) != 0 && errno != EINVAL) {
            LOG(WARNING) << "hs: umount " << m << " busy, continuing";
        }
    }

    // pass 2: unmap every known LP resident + any stale mapper node.
    std::set<std::string> names;
    for (const auto& p : live.all)
        names.insert(p.full);
    // stale nodes: anything still under /dev/block/mapper
    DIR* d = opendir("/dev/block/mapper");
    if (d) {
        struct dirent* de;
        while ((de = readdir(d)) != nullptr) {
            std::string n = de->d_name;
            if (n != "." && n != ".." && n != "by-uuid")
                names.insert(n);
        }
        closedir(d);
    }
    android::dm::DeviceMapper& dm = android::dm::DeviceMapper::Instance();
    std::vector<std::string> busy;
    for (const auto& n : names) {
        // siw 0.2.4 parity: an idle mapping (even ACTIVE) is safe to
        // drop; only a node the kernel refuses to remove (EBUSY: still
        // mounted or held open after the unmount pass above) is a hard
        // error. The kernel is the arbiter, not GetState (ACTIVE only
        // means "present", open_count may well be 0).
        if (dm.GetState(n) == android::dm::DmDeviceState::INVALID)
            continue;  // absent (or never a dm node): fine
        if (dm.DeleteDeviceIfExists(n)) {
            gone->push_back(n);
            continue;
        }
        // Removal refused: still live or lost a race. Re-check state.
        if (dm.GetState(n) == android::dm::DmDeviceState::INVALID)
            continue;  // vanished concurrently: fine
        busy.push_back(n);
    }
    if (!busy.empty()) {
        if (err) {
            *err = "dm busy, refusing to proceed: ";
            for (size_t i = 0; i < busy.size(); i++) {
                if (i)
                    *err += ",";
                *err += busy[i];
            }
        }
        return false;
    }
    return true;
}

namespace {

// /system/bin/siw is the only writer baseband_guard lets through
// (verified: direct liblp writes from this binary vanish silently,
// su-domain writers get EPERM; siw in the recovery domain persists).
// Keep the exact CLI shapes the proven Rust engine used.
constexpr const char* kSiw = "/system/bin/siw";

// Run siw, capture stderr. Returns exit code (0 = ok).
int RunSiw(const std::vector<std::string>& args, std::string* err_text) {
    int pe[2];
    if (pipe(pe) != 0)
        return 127;
    pid_t pid = fork();
    if (pid == -1) {
        close(pe[0]);
        close(pe[1]);
        return 127;
    }
    if (pid == 0) {
        dup2(pe[1], STDERR_FILENO);
        close(pe[0]);
        close(pe[1]);
        std::vector<char*> av;
        av.push_back(const_cast<char*>(kSiw));
        for (const auto& a : args)
            av.push_back(const_cast<char*>(a.c_str()));
        av.push_back(nullptr);
        execv(kSiw, av.data());
        _exit(127);
    }
    close(pe[1]);
    std::string err;
    char buf[512];
    ssize_t r;
    while ((r = TEMP_FAILURE_RETRY(read(pe[0], buf, sizeof(buf)))) > 0)
        err.append(buf, r);
    close(pe[0]);
    int st = 0;
    while (TEMP_FAILURE_RETRY(waitpid(pid, &st, 0)) < 0 && errno == EINTR) {
    }
    if (err_text)
        *err_text = err;
    if (WIFEXITED(st))
        return WEXITSTATUS(st);
    return 127;
}

}  // namespace

bool RemovePartitionLp(const std::string& full, std::string* err) {
    // Clear EVERY slot holding the entry (mirrored tables must not
    // diverge). Explicit -s per slot: siw refuses ambiguity, we iterate.
    for (uint32_t slot : {0u, 1u}) {
        std::string serr;
        int rc = RunSiw({"remove", kSuperBlock, full, "-s",
                         std::to_string(slot), "-y"},
                        &serr);
        if (rc == 0)
            continue;  // cleared here; keep clearing the other slot
        if (serr.find("not found") != std::string::npos)
            continue;  // absent in this slot: fine
        if (err)
            *err = "siw remove " + full + " (slot " +
                   std::to_string(slot) + ") failed: " + serr;
        return false;
    }
    return true;  // absent everywhere: idempotent success
}

std::optional<uint32_t> ResolveMetadataSlot(const std::string& full) {
    for (uint32_t slot : {0u, 1u}) {
        std::unique_ptr<android::fs_mgr::LpMetadata> md =
            android::fs_mgr::ReadMetadata(std::string(kSuperBlock), slot);
        if (!md)
            continue;
        for (const auto& p : md->partitions) {
            std::string nm(p.name, strnlen(p.name, sizeof(p.name)));
            if (nm == full)
                return slot;
        }
    }
    return std::nullopt;
}

bool CreatePartitionLp(const std::string& full, const std::string& group,
                       uint64_t size, std::string* err) {
    // Factory convention on this device: FULL names stored, readonly,
    // NO slot_suffixed flag (see intact slot 2: attrs=[readonly]).
    // Setting the flag makes liblp/lptools append a second suffix on
    // read (system_b -> system_b_b in memory) and nothing resolves.
    // First slot whose table holds the group wins.
    std::string last_err = "no metadata slot accepted the create";
    for (uint32_t slot : {0u, 1u}) {
        std::string serr;
        int rc = RunSiw({"create", kSuperBlock, "-n", full, "--group", group,
                         "--size", std::to_string(size), "--attrs",
                         "readonly", "-s", std::to_string(slot)},
                        &serr);
        if (rc == 0)
            return true;
        last_err = "siw create " + full + " in " + group + " (slot " +
                   std::to_string(slot) + ") failed: " + serr;
    }
    if (err)
        *err = last_err;
    return false;
}

bool MapWritable(const std::string& full, uint32_t metadata_slot,
                 std::string* out_path, std::string* err) {
    // lptools parity: explicit metadata_slot + timeout, never lib defaults.
    for (int attempt = 0; attempt < 2; attempt++) {
        android::fs_mgr::CreateLogicalPartitionParams params = {};
        params.block_device = kSuperBlock;
        params.partition_name = full;
        params.metadata_slot = metadata_slot;
        params.timeout_ms = std::chrono::milliseconds(10000);
        params.force_writable = true;
        std::string path;
        if (android::fs_mgr::CreateLogicalPartition(params, &path)) {
            *out_path = path;
            return true;
        }
        // Stale idle node from an aborted run: drop it once and retry.
        // A node the kernel refuses to remove (still mounted/held open)
        // is a hard error, never stolen (siw 0.2.4).
        android::dm::DeviceMapper& dm = android::dm::DeviceMapper::Instance();
        if (!dm.DeleteDeviceIfExists(full) &&
            dm.GetState(full) != android::dm::DmDeviceState::INVALID) {
            if (err)
                *err = "CreateLogicalPartition failed for " + full +
                       " (slot " + std::to_string(metadata_slot) +
                       "): live mapping present, refusing to take over";
            return false;
        }
    }
    if (err)
        *err = "CreateLogicalPartition failed for " + full + " (slot " +
               std::to_string(metadata_slot) + ")";
    return false;
}

}  // namespace hs
