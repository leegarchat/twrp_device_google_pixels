// Parallel op writer: per-op decode (BZ/XZ/ZSTD/raw), pwrite scatter,
// atomic progress. Read-back SHA256 validation lives here too.

#include "extract.h"

#include <fcntl.h>
#include <unistd.h>

#include <atomic>
#include <cerrno>
#include <cstring>
#include <mutex>
#include <thread>
#include <vector>

#include <android-base/logging.h>
#include <bzlib.h>
#include <zstd.h>

#include "Alloc.h"
#include "Xz.h"

#include "sha256.h"

namespace hs {

static bool PreadAll(int fd, uint64_t off, uint8_t* buf, size_t len,
                     std::string* err) {
    size_t got = 0;
    while (got < len) {
        ssize_t r = TEMP_FAILURE_RETRY(pread(fd, buf + got, len - got, off + got));
        if (r <= 0) {
            if (err)
                *err = std::string("pread: ") + strerror(errno);
            return false;
        }
        got += r;
    }
    return true;
}

static bool PwriteAll(int fd, uint64_t off, const uint8_t* buf, size_t len,
                      std::string* err) {
    size_t done = 0;
    while (done < len) {
        ssize_t r =
            TEMP_FAILURE_RETRY(pwrite(fd, buf + done, len - done, off + done));
        if (r <= 0) {
            if (err)
                *err = std::string("pwrite: ") + strerror(errno);
            return false;
        }
        done += r;
    }
    return true;
}

// Decode one op blob into exactly expanded_len bytes.
static bool DecodeOp(int typ, const std::vector<uint8_t>& blob,
                     size_t expanded_len, std::vector<uint8_t>* out,
                     std::string* err) {
    out->resize(expanded_len);
    switch (typ) {
        case 0: {  // REPLACE
            if (blob.size() != expanded_len) {
                if (err)
                    *err = "REPLACE size mismatch";
                return false;
            }
            memcpy(out->data(), blob.data(), expanded_len);
            return true;
        }
        case 1: {  // REPLACE_BZ
            unsigned int dst = expanded_len;
            int rc = BZ2_bzBuffToBuffDecompress(
                reinterpret_cast<char*>(out->data()), &dst,
                const_cast<char*>(reinterpret_cast<const char*>(blob.data())),
                blob.size(), 0, 0);
            if (rc != BZ_OK || dst != expanded_len) {
                if (err)
                    *err = "bz decode failed";
                return false;
            }
            return true;
        }
        case 8: {  // REPLACE_XZ (7-Zip XzUnpacker, one-call full decode;
                   // in-tree xz-embedded cannot decode OTA xz streams)
            CXzUnpacker u;
            XzUnpacker_Construct(&u, &g_Alloc);
            XzUnpacker_Init(&u);
            SizeT destLen = expanded_len, srcLen = blob.size();
            ECoderStatus st = CODER_STATUS_NOT_SPECIFIED;
            SRes res = XzUnpacker_CodeFull(&u, out->data(), &destLen,
                                           blob.data(), &srcLen,
                                           CODER_FINISH_END, &st);
            BoolInt finished = XzUnpacker_IsStreamWasFinished(&u);
            XzUnpacker_Free(&u);
            if (res != SZ_OK || destLen != expanded_len || !finished) {
                if (err) {
                    char eb[160];
                    snprintf(eb, sizeof(eb),
                             "xz decode failed: res=%d status=%d out %zu/%zu",
                             (int)res, (int)st, (size_t)destLen, expanded_len);
                    *err = eb;
                }
                return false;
            }
            return true;
        }
        case 14: {  // REPLACE_ZSTD (window-capped, peridot parity)
            ZSTD_DCtx* dctx = ZSTD_createDCtx();
            if (!dctx) {
                if (err)
                    *err = "zstd ctx failed";
                return false;
            }
            size_t wrc = ZSTD_DCtx_setParameter(dctx, ZSTD_d_windowLogMax, 27);
            size_t rc = ZSTD_isError(wrc)
                            ? wrc
                            : ZSTD_decompressDCtx(dctx, out->data(),
                                                  expanded_len, blob.data(),
                                                  blob.size());
            ZSTD_freeDCtx(dctx);
            if (ZSTD_isError(rc) || rc != expanded_len) {
                if (err)
                    *err = std::string("zstd decode failed: ") +
                           ZSTD_getErrorName(rc);
                return false;
            }
            return true;
        }
        default:
            if (err)
                *err = "unsupported op";
            return false;
    }
}

struct Task {
    const OtaPart* part;
    OtaOp op;
    std::string node;
    uint64_t expanded = 0;
};

bool WriteAll(const std::string& zip, uint64_t data_start, uint64_t bs,
              const std::vector<WriteTarget>& targets, unsigned threads,
              uint64_t total_out,
              const std::function<void(uint64_t, uint64_t)>& progress,
              std::string* err) {
    // Flatten to op tasks (independent extents => any order).
    std::vector<Task> tasks;
    for (const auto& t : targets) {
        for (const auto& o : t.manifest->ops) {
            if (o.type == 7)  // DISCARD
                continue;
            Task tk{t.manifest, o, t.node, 0};
            for (const auto& e : o.dst)
                tk.expanded += e.second * bs;
            tasks.push_back(std::move(tk));
        }
    }

    int zfd = open(zip.c_str(), O_RDONLY | O_CLOEXEC);
    if (zfd < 0) {
        if (err)
            *err = "cannot open zip";
        return false;
    }
    // One output fd per node, shared by workers (pwrite is thread-safe).
    // O_EXCL claim (peridot parity): fail fast if a target is busy
    // (mounted/held open) instead of writing under a live user.
    std::map<std::string, int> outfds;
    for (const auto& t : targets) {
        int fd = open(t.node.c_str(), O_RDWR | O_EXCL | O_CLOEXEC);
        if (fd < 0) {
            if (err) {
                *err = "cannot open target " + t.node + ": " +
                       strerror(errno) +
                       (errno == EBUSY ? " (busy: mounted or held open)" : "");
            }
            printf("log WRITE open %s failed: %s\n", t.node.c_str(),
                   strerror(errno));
            fflush(stdout);
            close(zfd);
            for (auto& kv : outfds)
                close(kv.second);
            return false;
        }
        outfds[t.node] = fd;
    }
    printf("log WRITE %zu targets %u threads\n", targets.size(), threads);
    fflush(stdout);

    std::atomic<size_t> next{0};
    std::atomic<uint64_t> done_expanded{0};
    std::atomic<bool> failed{false};
    std::string first_err;
    std::mutex err_mu, prog_mu;
    if (threads < 1)
        threads = 1;
    std::vector<std::thread> workers;
    for (unsigned w = 0; w < threads; w++) {
        workers.emplace_back([&]() {
            // Separate zip fd per thread (independent file offset).
            int zf = open(zip.c_str(), O_RDONLY | O_CLOEXEC);
            if (zf < 0) {
                std::lock_guard<std::mutex> l(err_mu);
                if (!failed) {
                    failed = true;
                    first_err = "cannot reopen zip";
                }
                return;
            }
            std::vector<uint8_t> blob, plain;
            std::vector<uint8_t> zeros(1 << 20, 0);
            while (!failed) {
                size_t i = next.fetch_add(1);
                if (i >= tasks.size())
                    break;
                const Task& tk = tasks[i];
                std::string e;
                bool ok = false;
                if (tk.op.type == 6) {  // ZERO: explicit zero fill
                    ok = true;
                    for (const auto& ex : tk.op.dst) {
                        uint64_t left = ex.second * bs;
                        uint64_t at = ex.first * bs;
                        while (left > 0 && ok) {
                            size_t n = left > zeros.size() ? zeros.size()
                                                           : (size_t)left;
                            int fd = outfds[tk.node];
                            ok = PwriteAll(fd, at, zeros.data(), n, &e);
                            at += n;
                            left -= n;
                        }
                        if (!ok)
                            break;
                    }
                } else {
                    blob.resize(tk.op.data_len);
                    // Per-op input hash (peridot direct.rs parity): verify the
                    // stored blob before decoding it.
                    if (!tk.op.data_hash.empty()) {
                        if (tk.op.data_hash.size() != 32) {
                            e = "op data hash malformed";
                        } else if (PreadAll(zf, data_start + tk.op.data_off,
                                            blob.data(), blob.size(), &e)) {
                            uint8_t sum[32];
                            Sha256 h2;
                            h2.update(blob.data(), blob.size());
                            h2.final(sum);
                            if (memcmp(sum, tk.op.data_hash.data(), 32) != 0)
                                e = "op data sha256 mismatch";
                        }
                    } else if (!PreadAll(zf, data_start + tk.op.data_off,
                                         blob.data(), blob.size(), &e)) {
                        // e already set by PreadAll
                    }
                    if (e.empty() &&
                        DecodeOp(tk.op.type, blob, tk.expanded, &plain, &e)) {
                        ok = true;
                        size_t pos = 0;
                        int fd = outfds[tk.node];
                        for (const auto& ex : tk.op.dst) {
                            size_t n = ex.second * bs;
                            if (!PwriteAll(fd, ex.first * bs, plain.data() + pos,
                                           n, &e)) {
                                ok = false;
                                break;
                            }
                            pos += n;
                        }
                    }
                }
                if (ok) {
                    uint64_t d =
                        done_expanded.fetch_add(tk.expanded) + tk.expanded;
                    std::lock_guard<std::mutex> l(prog_mu);
                    progress(d, total_out);
                } else {
                    printf("log WRITE op failed: %s type=%d off=%llu len=%llu: %s\n",
                           tk.part->name.c_str(), tk.op.type,
                           (unsigned long long)tk.op.data_off,
                           (unsigned long long)tk.op.data_len, e.c_str());
                    fflush(stdout);
                    std::lock_guard<std::mutex> l(err_mu);
                    if (!failed) {
                        failed = true;
                        first_err = tk.part->name + ": " + e;
                    }
                }
            }
            close(zf);
        });
    }
    for (auto& t : workers)
        t.join();
    for (auto& kv : outfds)
        close(kv.second);
    close(zfd);
    if (failed) {
        if (err)
            *err = first_err;
        return false;
    }
    return true;
}

std::string Sha256File(const std::string& path, uint64_t size) {
    int fd = open(path.c_str(), O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return "";
    Sha256 h;
    std::vector<uint8_t> buf(1 << 20);
    uint64_t left = size;
    while (left > 0) {
        size_t n = left > buf.size() ? buf.size() : (size_t)left;
        ssize_t r = TEMP_FAILURE_RETRY(read(fd, buf.data(), n));
        if (r <= 0) {
            close(fd);
            return "";
        }
        h.update(buf.data(), r);
        left -= r;
    }
    close(fd);
    uint8_t sum[32];
    h.final(sum);
    static const char* hex = "0123456789abcdef";
    std::string out;
    for (int i = 0; i < 32; i++) {
        out += hex[sum[i] >> 4];
        out += hex[sum[i] & 15];
    }
    return out;
}

}  // namespace hs
