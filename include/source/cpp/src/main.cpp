// fox-hsinstall main: full-OTA fast path with liblp-native rebuild.

#include <fcntl.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cstdio>
#include <cstring>
#include <map>
#include <set>
#include <string>
#include <thread>
#include <vector>

#include <android-base/logging.h>
#include <android-base/properties.h>
#include <android-base/strings.h>

#include "backup.h"
#include "extract.h"
#include "hs.h"
#include "lp.h"
#include "ota.h"
#include "plan.h"

namespace {

struct Args {
    std::string zip;
    std::string tmpdir = "/tmp/hs_backup";
    char slot = 0;  // 0 = auto (opposite of booted)
    unsigned threads = 0;  // 0 = auto: min(ncpu,8), RAM-capped
    bool readback = false;
};

bool GetFlag(int argc, char** argv, const char* name, std::string* out) {
    std::string glued = std::string(name) + "=";
    for (int i = 1; i < argc; i++) {
        std::string a = argv[i];
        if (a == name && i + 1 < argc) {
            *out = argv[i + 1];
            return true;
        }
        if (a.compare(0, glued.size(), glued) == 0) {
            *out = a.substr(glued.size());
            return true;
        }
    }
    return false;
}

void Usage() {
    fprintf(stderr,
            "usage: fox-hsinstall --zip PATH [--tmpdir DIR] [--slot a|b]\n"
            "                     [--threads N] [--readback]\n");
}

// Worker budget (peridot parity): min(ncpu,8), additionally capped by
// free RAM: 512MiB reserve + 96MiB per worker. Never below 1.
static unsigned AutoThreads() {
    long ncpu = sysconf(_SC_NPROCESSORS_ONLN);
    unsigned w = 8;
    if (ncpu > 0)
        w = std::min<unsigned>(8, static_cast<unsigned>(ncpu));
    FILE* f = fopen("/proc/meminfo", "r");
    if (f) {
        char line[128];
        while (fgets(line, sizeof(line), f)) {
            long kb = 0;
            if (sscanf(line, "MemAvailable: %ld kB", &kb) == 1 && kb > 0) {
                long cap = (kb / 1024 - 512) / 96;
                if (cap >= 1)
                    w = std::min<unsigned>(w, static_cast<unsigned>(cap));
                else
                    w = 1;
                break;
            }
        }
        fclose(f);
    }
    return w < 1 ? 1 : w;
}

int Fail(const std::string& msg, int code) {
    hs::UiPrint("hs-install: " + msg);
    return code;
}

int WeightOf(const std::string& base, bool in_group) {
    if (in_group)
        return 1;
    static const char* boot[] = {"boot",     "init_boot", "vendor_boot",
                                 "vendor_kernel_boot", "dtbo", nullptr};
    static const char* modem[] = {"modem", "pvmfw", "gcf", "ldfw", nullptr};
    for (int i = 0; boot[i]; i++)
        if (base == boot[i])
            return 2;
    for (int i = 0; modem[i]; i++)
        if (base == modem[i])
            return 3;
    return 4;
}

bool IsCritical(int w) {
    return w == 2 || w == 4;
}

}  // namespace

int main(int argc, char** argv) {
    Args args;
    std::string tmp;
    if (!GetFlag(argc, argv, "--zip", &args.zip)) {
        Usage();
        return hs::kExitUsage;
    }
    // Accepted for compatibility with older callers; ignored.
    GetFlag(argc, argv, "--status-fd", &tmp);
    GetFlag(argc, argv, "--sysroot", &tmp);
    if (GetFlag(argc, argv, "--tmpdir", &tmp))
        args.tmpdir = tmp;
    if (GetFlag(argc, argv, "--slot", &tmp)) {
        if (tmp != "a" && tmp != "b") {
            Usage();
            return hs::kExitUsage;
        }
        args.slot = tmp[0];
    }
    if (GetFlag(argc, argv, "--threads", &tmp)) {
        args.threads = atoi(tmp.c_str());
        if (args.threads < 1)
            args.threads = 1;
        if (args.threads > 16)
            args.threads = 16;
    } else {
        args.threads = AutoThreads();
    }
    // Boolean flag (no value): present anywhere in argv.
    for (int i = 1; i < argc; i++)
        if (std::string(argv[i]) == "--readback")
            args.readback = true;

    printf("progress 1.0 600\n");
    fflush(stdout);

    // --- 0. identity: universal, no hardcoded devices ---
    std::string device = android::base::GetProperty("ro.product.device", "");
    if (device.empty())
        return Fail("no ro.product.device prop", hs::kExitFail);
    std::string booted = android::base::GetProperty("ro.boot.slot_suffix", "");
    char bootc = 0;
    if (booted == "_a")
        bootc = 'a';
    else if (booted == "_b")
        bootc = 'b';
    if (bootc == 0)
        return Fail("unparsable ro.boot.slot_suffix", hs::kExitFail);
    char target = args.slot ? args.slot : (bootc == 'a' ? 'b' : 'a');
    char tstr[2] = {target, 0};
    hs::UiPrint(std::string("hs-install: device=") + device + " guest slot=_" +
                tstr);

    // --- 1. device gate: hard mismatch, never falls back ---
    {
        std::string pre = hs::OtaPreDevice(args.zip);
        if (pre.empty())
            return Fail("OTA metadata has no pre-device", hs::kExitFail);
        bool ok = false;
        for (const auto& d : android::base::Split(pre, "| ,")) {
            if (d == device)
                ok = true;
        }
        if (!ok)
            return Fail("package is for '" + pre + "' but device is '" +
                            device + "': refusing",
                        hs::kExitDeviceMismatch);
    }

    // --- 2. manifest: full OTA only, else stock path ---
    std::vector<hs::OtaPart> parts;
    std::vector<hs::DynGroup> groups;
    uint64_t bs = 4096, data_start = 0;
    bool full = false;
    std::string not_full, perr;
    perr = hs::ProbePayload(args.zip, &parts, &groups, &bs, &data_start,
                            &full, &not_full);
    if (!perr.empty())
        return Fail("probe: " + perr, hs::kExitFail);
    fprintf(stderr, "DBG main: verdict full=%d\n", (int)full);
    if (!full) {
        hs::UiPrint("hs-install: not full OTA (" + not_full + "), stock path");
        return hs::kExitNotFull;
    }

    // --- 3. live super state (liblp, no siw) ---
    fprintf(stderr, "DBG main: querying live super\n");
    hs::LiveState live;
    {
        std::string e;
        if (!hs::QueryLive("/dev/block/by-name/super", bootc, &live, &e))
            return Fail("live super: " + e, hs::kExitNoHelpers);
        fprintf(stderr, "DBG main: live ok\n");
    }
    auto out_of = [&](const std::string& base) -> uint64_t {
        for (const auto& p : parts)
            if (p.name == base)
                return p.OutSize(bs);
        return 0;
    };
    std::map<std::string, bool> in_group;
    for (const auto& g : groups)
        for (const auto& n : g.parts)
            in_group[n] = true;

    // --- 4. fit ladder (phase 2) ---
    struct ReqGrp {
        std::vector<std::pair<std::string, uint64_t>> req;
        std::string manifest_group;
        uint64_t budget = 0;
    };
    std::vector<ReqGrp> rgs;
    for (const auto& g : groups) {
        ReqGrp r;
        r.manifest_group = g.name;
        r.budget = g.size;
        for (const auto& n : g.parts)
            r.req.emplace_back(n, out_of(n));
        rgs.push_back(r);
    }
    bool impossible = false;
    // (base name, slot letter holding it)
    std::vector<std::pair<std::string, char>> kill;
    std::string worst = "FITS_AS_IS";
    for (const auto& r : rgs) {
        hs::FitResult f =
            hs::PlanGroup(r.req, r.manifest_group, r.budget, live);
        hs::UiPrint(std::string("hs-install: fit ") + hs::FitCode(f.plan));
        if (f.plan == hs::FitPlan::kImpossible) {
            impossible = true;
        } else if (f.plan == hs::FitPlan::kWipeAll) {
            worst = "WIPE_ALL";
        } else if (f.plan == hs::FitPlan::kKill && worst != "WIPE_ALL") {
            worst = "KILL";
            for (const auto& n : f.kill)
                kill.emplace_back(n, bootc);
        }
    }
    if (impossible)
        return Fail(
            "payload does not fit even a bare super: abort, nothing written",
            hs::kExitImpossible);
    if (worst == "WIPE_ALL") {
        kill.clear();
        for (const auto& kv : live.current)
            kill.emplace_back(kv.first, bootc);
        for (const auto& kv : live.target)
            kill.emplace_back(kv.first, target);
        hs::UiPrint("hs-install: WIPE_ALL plan");
    } else if (!kill.empty()) {
        std::string k = "hs-install: reclaim:";
        for (const auto& n : kill)
            k += " " + n.first + "_" + n.second;
        hs::UiPrint(k);
    }
    auto full_name = [&](const std::string& base, char slot) {
        return base + std::string("_") + slot;
    };

    // --- 5. order: weights; backup critical (2+4) of the target slot ---
    struct Job {
        const hs::OtaPart* part = nullptr;
        int weight = 4;
    };
    std::vector<Job> jobs;
    for (const auto& p : parts) {
        bool ing = in_group.count(p.name) > 0;
        jobs.push_back({&p, WeightOf(p.name, ing)});
    }
    hs::Mkdirs(args.tmpdir);
    // Scheme phase 0 #5-7 (runs before any write, backup included):
    // every BLOCK (non-group) target must already exist as a block node.
    for (const auto& j : jobs) {
        if (in_group.count(j.part->name) > 0)
            continue;  // logical: created from the group below
        std::string node = std::string("/dev/block/by-name/") +
                           full_name(j.part->name, target);
        struct stat st;
        if (stat(node.c_str(), &st) != 0)
            return Fail("BLOCK target missing: " + node + " (" +
                            strerror(errno) + "), nothing written",
                        hs::kExitFail);
        if (!S_ISBLK(st.st_mode))
            return Fail("BLOCK target not a block device: " + node +
                            ", nothing written",
                        hs::kExitFail);
    }
    struct Backed {
        bool restored;  // true = file backup; false = fresh-created (remove)
        std::string node, file, full;
    };
    std::vector<Backed> backed;
    for (const auto& j : jobs) {
        if (!IsCritical(j.weight))
            continue;
        std::string full = full_name(j.part->name, target);
        auto it = live.target.find(j.part->name);
        if (it == live.target.end()) {
            // created fresh below; rollback removes it
            backed.push_back({false, "", "", full});
            continue;
        }
        std::string node, e;
        auto bslot = hs::ResolveMetadataSlot(full);
        if (!bslot)
            return Fail("backup map " + full +
                            ": LP entry vanished from both slots",
                        hs::kExitFail);
        if (!hs::MapWritable(full, *bslot, &node, &e))
            return Fail("backup map " + full + ": " + e, hs::kExitFail);
        std::string file = args.tmpdir + "/" + j.part->name + ".img";
        hs::UiPrint("hs-install: backup " + j.part->name);
        if (!hs::DumpNode(node, file, it->second, &e))
            return Fail("backup " + j.part->name + ": " + e, hs::kExitFail);
        backed.push_back({true, node, file, full});
    }

    // --- 6. rebuild, single thread: unmount+unmap all, remove, create, map
    hs::UiPrint("hs-install: rebuilding target group (single-threaded)");
    {
        std::vector<std::string> gone;
        std::string e;
        if (!hs::UnmountUnmapAll(live, &gone, &e))
            return Fail(e, hs::kExitFail);
        hs::UiPrint("hs-install: unmapped " + std::to_string(gone.size()) +
                    " dm node(s)");
    }
    // remove: kill list + target group members
    {
        std::set<std::string> victims;
        for (const auto& b : kill)
            victims.insert(full_name(b.first, b.second));
        for (const auto& g : groups)
            for (const auto& n : g.parts)
                victims.insert(full_name(n, target));
        std::string e;
        for (const auto& v : victims) {
            printf("log TRACE victim=[%s]\n", v.c_str());
            fflush(stdout);
            if (!hs::RemovePartitionLp(v, &e))
                return Fail("remove " + v + ": " + e, hs::kExitFail);
        }
    }
    // create target members with manifest sizes in the live group
    std::string live_group = live.target_group;
    if (live_group.empty() && !groups.empty())
        live_group = groups[0].name;
    for (const auto& g : groups) {
        std::string grp = live_group.empty() ? g.name : live_group;
        for (const auto& n : g.parts) {
            std::string full = full_name(n, target);
            printf("log TRACE create=[%s] group=[%s] part=[%s]\n", full.c_str(),
                   grp.c_str(), n.c_str());
            fflush(stdout);
            std::string e;
            if (!hs::CreatePartitionLp(full, grp, out_of(n), &e)) {
                std::string grp2 = g.name + std::string("_") + target;
                if (grp2 == grp ||
                    !hs::CreatePartitionLp(full, grp2, out_of(n), &e))
                    return Fail("create " + full + ": " + e, hs::kExitFail);
            }
        }
    }
    // map every payload partition of the target slot for writing:
    // logical (manifest group members) via dm mapper, physical ones
    // (boot/modem/abl/...) straight at /dev/block/by-name/<full>
    // (mirrors hs-install/src/run.rs node_of).
    std::map<std::string, std::string> node_of;
    for (const auto& j : jobs) {
        std::string full = full_name(j.part->name, target);
        if (in_group.count(j.part->name) == 0) {
            node_of[j.part->name] = std::string("/dev/block/by-name/") + full;
            continue;
        }
        std::string node, e;
        auto mslot = hs::ResolveMetadataSlot(full);
        if (!mslot)
            return Fail("map " + full + ": LP entry missing from both slots",
                        hs::kExitFail);
        if (!hs::MapWritable(full, *mslot, &node, &e))
            return Fail("map " + full + ": " + e, hs::kExitFail);
        node_of[j.part->name] = node;
    }

    // --- 7. batches 1->2->3->4, barrier + read-back validation ---
    uint64_t total_out = 0;
    for (const auto& j : jobs)
        total_out += j.part->OutSize(bs);
    auto report = [&](uint64_t d, uint64_t t) {
        hs::SetProgress(0.05 + 0.9 * (double)d / (double)(t ? t : 1));
    };
    auto do_rollback = [&]() {
        hs::UiPrint("hs-install: rollback: restoring critical partitions");
        for (const auto& b : backed) {
            std::string e;
            if (b.restored) {
                if (!hs::RestoreNode(b.file, b.node, &e))
                    hs::UiPrint("hs-install: rollback FAILED " + b.node +
                                ": " + e);
                else
                    hs::UiPrint("hs-install: rollback ok: " + b.node);
            } else {
                if (!hs::RemovePartitionLp(b.full, &e))
                    hs::UiPrint("hs-install: rollback FAILED " + b.full +
                                ": " + e);
                else
                    hs::UiPrint("hs-install: rollback removed fresh " + b.full);
            }
        }
    };
    for (int w = 1; w <= 4; w++) {
        std::vector<Job> batch;
        for (const auto& j : jobs)
            if (j.weight == w)
                batch.push_back(j);
        if (batch.empty())
            continue;
        std::sort(batch.begin(), batch.end(),
                  [&](const Job& a, const Job& b) {
                      return a.part->OutSize(bs) > b.part->OutSize(bs);
                  });
        {
            char bmsg[128];
            snprintf(bmsg, sizeof(bmsg), "hs-install: batch weight %d (%zu parts)",
                     w, batch.size());
            hs::UiPrint(bmsg);
        }
        std::vector<hs::WriteTarget> tgts;
        for (const auto& j : batch)
            tgts.push_back({j.part->name, node_of[j.part->name], j.part});
        std::string e;
        if (!hs::WriteAll(args.zip, data_start, bs, tgts, args.threads,
                          total_out, report, &e)) {
            hs::UiPrint("hs-install: batch " + std::to_string(w) +
                        " FAILED: " + e);
            do_rollback();
            return Fail(
                "install failed; critical partitions restored, slot NOT switched",
                hs::kExitFail);
        }
        for (const auto& j : batch) {
            std::string e2;
            if (!hs::FsyncNode(node_of[j.part->name], &e2)) {
                do_rollback();
                return Fail("fsync " + j.part->name + ": " + e2, hs::kExitFail);
            }
            uint64_t out_sz = j.part->OutSize(bs);
            std::string sum = hs::Sha256File(node_of[j.part->name], out_sz);
            if (args.readback) {
                // --readback: hash what actually reached storage, not RAM.
                std::string rbe;
                std::string rsum =
                    hs::ReadbackCheck(node_of[j.part->name], out_sz, &rbe);
                if (rsum.empty()) {
                    do_rollback();
                    return Fail(j.part->name + ": readback failed: " + rbe,
                                hs::kExitFail);
                }
                sum = rsum;
            }
            if (!j.part->new_hash.empty() && j.part->new_hash.size() == 32) {
                std::string ref;
                for (char c : j.part->new_hash) {
                    char hb[3];
                    snprintf(hb, sizeof(hb), "%02x", (unsigned char)c);
                    ref += hb;
                }
                if (sum != ref) {
                    do_rollback();
                    return Fail(j.part->name + ": sha256 mismatch",
                                hs::kExitFail);
                }
            }
        }
        {
            char bmsg[64];
            snprintf(bmsg, sizeof(bmsg), "hs-install: batch %d ok", w);
            hs::UiPrint(bmsg);
        }
    }

    hs::UiPrint("hs-install: syncing");
    sync();
    hs::SetProgress(1.0);
    {
        // Scheme phase 6: drop the /tmp backup on success (kept on
        // failure for forensics). Slot switch stays with the caller.
        pid_t rm = fork();
        if (rm == 0) {
            execl("/system/bin/rm", "rm", "-rf", args.tmpdir.c_str(),
                  (char*)nullptr);
            _exit(127);
        } else if (rm > 0) {
            int st = 0;
            waitpid(rm, &st, 0);
        }
        char tstr2[2] = {target, 0};
        hs::UiPrint(std::string("hs-install: DONE slot _") + tstr2);
        printf("hs_done %c\n", target);
        fflush(stdout);
    }
    return hs::kExitOk;
}
