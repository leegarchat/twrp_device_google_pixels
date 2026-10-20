// OTA probing: libziparchive directory walk + CrAU header + protobuf.

#include "ota.h"

#include "hs.h"

#include <fcntl.h>
#include <unistd.h>

#include <algorithm>
#include <cstring>

#include <android-base/logging.h>
#include <android-base/strings.h>
#include <ziparchive/zip_archive.h>

#include <update_engine/update_metadata.pb.h>

namespace hs {

uint64_t OtaPart::OutSize(uint64_t bs) const {
    uint64_t end = 0;
    for (const auto& o : ops) {
        for (const auto& e : o.dst) {
            uint64_t x = (e.first + e.second) * bs;
            if (x > end)
                end = x;
        }
    }
    return end;
}

uint64_t OtaPart::DlSize() const {
    uint64_t n = 0;
    for (const auto& o : ops) {
        if (o.type != 6 && o.type != 7)
            n += o.data_len;
    }
    return n;
}

// Read exactly len bytes at off from fd.
static bool PreadAll(int fd, uint64_t off, uint8_t* buf, size_t len) {
    size_t got = 0;
    while (got < len) {
        ssize_t r = TEMP_FAILURE_RETRY(pread(fd, buf + got, len - got, off + got));
        if (r <= 0)
            return false;
        got += r;
    }
    return true;
}

std::string OtaPreDevice(const std::string& zip_path) {
    fprintf(stderr, "DBG predevice: open\n");
    ZipArchiveHandle h = nullptr;
    if (OpenArchive(zip_path.c_str(), &h) != 0 || !h)
        return "";
    std::string out;
    fprintf(stderr, "DBG predevice: find\n");
    ZipEntry entry;
    const std::string meta_name("META-INF/com/android/metadata");
    if (FindEntry(h, meta_name, &entry) == 0) {
        std::string buf(entry.uncompressed_length, '\0');
        if (ExtractToMemory(h, &entry, reinterpret_cast<uint8_t*>(&buf[0]),
                            buf.size()) == 0) {
            for (const auto& line :
                 android::base::Split(buf, "\n")) {  // NOLINT: simple parse
                if (line.compare(0, 11, "pre-device=") == 0)
                    out = line.substr(11);
            }
            // trim trailing \r
            while (!out.empty() && (out.back() == '\r' || out.back() == ' '))
                out.pop_back();
        }
    }
    CloseArchive(h);
    return out;
}

std::string ProbePayload(const std::string& zip_path,
                         std::vector<OtaPart>* parts,
                         std::vector<DynGroup>* groups, uint64_t* block_size,
                         uint64_t* data_start, bool* full_ota,
                         std::string* not_full_reason) {
    ZipArchiveHandle h = nullptr;
    if (OpenArchive(zip_path.c_str(), &h) != 0 || !h)
        return "cannot open zip";
    auto close_h = [&]() { CloseArchive(h); };

    // payload.bin data offset: prefer the archive offset (no decompression).
    fprintf(stderr, "DBG probe: open+find\n");
    ZipEntry payload;
    const std::string payload_name("payload.bin");
    if (FindEntry(h, payload_name, &payload) != 0) {
        close_h();
        return "no payload.bin";
    }
    const uint64_t p_off = static_cast<uint64_t>(payload.offset);

    int fd = open(zip_path.c_str(), O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        close_h();
        return "cannot open zip file";
    }
    uint8_t hdr[24];
    if (!PreadAll(fd, p_off, hdr, sizeof(hdr))) {
        close(fd);
        close_h();
        return "short payload header";
    }
    if (memcmp(hdr, "CrAU", 4) != 0) {
        close(fd);
        close_h();
        return "payload magic mismatch";
    }
    uint64_t msize = 0, ssize = 0;
    for (int i = 0; i < 8; i++)
        msize = (msize << 8) | hdr[12 + i];
    for (int i = 0; i < 4; i++)
        ssize = (ssize << 8) | hdr[20 + i];
    fprintf(stderr, "DBG probe: manifest %llu bytes\n", (unsigned long long)msize);
    std::string mbuf(msize, '\0');
    if (!PreadAll(fd, p_off + 24, reinterpret_cast<uint8_t*>(&mbuf[0]), msize)) {
        close(fd);
        close_h();
        return "short manifest";
    }
    close(fd);

    chromeos_update_engine::DeltaArchiveManifest manifest;
    fprintf(stderr, "DBG probe: parsing\n");
    if (!manifest.ParseFromString(mbuf)) {
        close_h();
        return "manifest protobuf parse failed";
    }
    fprintf(stderr, "DBG probe: parsed %d parts\n", manifest.partitions_size());
    *block_size = manifest.has_block_size() ? manifest.block_size() : 4096;
    *data_start = p_off + 24 + msize + ssize;

    if (manifest.has_dynamic_partition_metadata()) {
        const auto& dpm = manifest.dynamic_partition_metadata();
        for (int i = 0; i < dpm.groups_size(); i++) {
            DynGroup g;
            g.name = dpm.groups(i).name();
            g.size = dpm.groups(i).size();
            for (int j = 0; j < dpm.groups(i).partition_names_size(); j++)
                g.parts.push_back(dpm.groups(i).partition_names(j));
            groups->push_back(g);
        }
    }

    for (int i = 0; i < manifest.partitions_size(); i++) {
        const auto& pp = manifest.partitions(i);
        OtaPart p;
        p.name = pp.partition_name();
        p.has_old_info = pp.has_old_partition_info();
        if (pp.has_new_partition_info()) {
            p.new_size = pp.new_partition_info().size();
            p.new_hash = pp.new_partition_info().hash();
        }
        for (int j = 0; j < pp.operations_size(); j++) {
            const auto& op = pp.operations(j);
            OtaOp o;
            o.type = op.type();
            o.data_off = op.has_data_offset() ? op.data_offset() : 0;
            o.data_len = op.has_data_length() ? op.data_length() : 0;
            if (op.has_data_sha256_hash())
                o.data_hash = op.data_sha256_hash();
            for (int k = 0; k < op.dst_extents_size(); k++) {
                o.dst.emplace_back(op.dst_extents(k).start_block(),
                                   op.dst_extents(k).num_blocks());
            }
            p.ops.push_back(o);
        }
        parts->push_back(p);
    }
    close_h();
    if (parts->empty())
        return "no partitions in manifest";

    // Delta signal #2 (peridot parity): minor_version != 0 means the
    // payload is not a full image even if every op looks plain.
    if (manifest.has_minor_version() && manifest.minor_version() != 0) {
        *not_full_reason = "manifest minor_version=" +
                            std::to_string(manifest.minor_version()) +
                            " (delta payload)";
        *full_ota = false;
        return "";
    }

    // Tiling check (peridot validate() parity): dst extents of every
    // partition must tile [0, new_size) exactly — no gaps, no overlaps.
    // A stock update_engine would refuse these too, so this is a hard
    // error, never a stock fallback.
    for (const auto& p : *parts) {
        if (p.new_size == 0)
            continue;
        uint64_t total = 0;
        for (const auto& o : p.ops)
            for (const auto& e : o.dst)
                total += e.second;
        if (total * *block_size != p.new_size)
            return "tiling: " + p.name + ": extents cover " +
                   std::to_string(total * *block_size) + " bytes, new_size=" +
                   std::to_string(p.new_size);
        std::vector<std::pair<uint64_t, uint64_t>> segs;
        for (const auto& o : p.ops)
            for (const auto& e : o.dst)
                segs.emplace_back(e.first, e.first + e.second);
        std::sort(segs.begin(), segs.end());
        uint64_t expect = 0;
        for (const auto& s : segs) {
            if (s.first != expect)
                return "tiling: " + p.name + ": gap/overlap at block " +
                       std::to_string(expect);
            expect = s.second;
        }
    }

    *full_ota = true;
    for (const auto& p : *parts) {
        if (p.has_old_info) {
            *not_full_reason = p.name + ": old_partition_info present (delta)";
            *full_ota = false;
            return "";
        }
        for (const auto& o : p.ops) {
            if (!IsFullOtaOp(o.type)) {
                *not_full_reason = p.name + ": op " + std::to_string(o.type) +
                                   " (" + OpName(o.type) + ") not in full-OTA set";
                *full_ota = false;
                return "";
            }
        }
    }
    return "";
}

}  // namespace hs
