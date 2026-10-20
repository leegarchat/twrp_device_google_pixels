// Critical-partition backup via pread/pwrite (no external tools).

#include "backup.h"

#include "extract.h"

#include <fcntl.h>
#include <sys/ioctl.h>
#include <unistd.h>

#include <linux/fs.h>

#include <cerrno>
#include <cstring>
#include <vector>

namespace hs {

static bool CopyAll(int src, int dst, uint64_t size, std::string* err) {
    std::vector<uint8_t> buf(1 << 20);
    uint64_t left = size;
    while (left > 0) {
        size_t n = left > buf.size() ? buf.size() : (size_t)left;
        ssize_t r = TEMP_FAILURE_RETRY(read(src, buf.data(), n));
        if (r <= 0) {
            if (err)
                *err = std::string("read: ") + strerror(errno);
            return false;
        }
        size_t done = 0;
        while (done < (size_t)r) {
            ssize_t w = TEMP_FAILURE_RETRY(
                write(dst, buf.data() + done, r - done));
            if (w <= 0) {
                if (err)
                    *err = std::string("write: ") + strerror(errno);
                return false;
            }
            done += w;
        }
        left -= r;
    }
    return true;
}

bool DumpNode(const std::string& node, const std::string& file, uint64_t size,
              std::string* err) {
    int src = open(node.c_str(), O_RDONLY | O_CLOEXEC);
    if (src < 0) {
        if (err)
            *err = "open " + node + ": " + strerror(errno);
        return false;
    }
    int dst = open(file.c_str(), O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600);
    if (dst < 0) {
        if (err)
            *err = "create " + file + ": " + strerror(errno);
        close(src);
        return false;
    }
    bool ok = CopyAll(src, dst, size, err);
    if (ok && fsync(dst) != 0) {
        if (err)
            *err = std::string("fsync: ") + strerror(errno);
        ok = false;
    }
    close(src);
    close(dst);
    return ok;
}

bool RestoreNode(const std::string& file, const std::string& node,
                 std::string* err) {
    int src = open(file.c_str(), O_RDONLY | O_CLOEXEC);
    if (src < 0) {
        if (err)
            *err = "open " + file + ": " + strerror(errno);
        return false;
    }
    int dst = open(node.c_str(), O_WRONLY | O_CLOEXEC);
    if (dst < 0) {
        if (err)
            *err = "open " + node + ": " + strerror(errno);
        close(src);
        return false;
    }
    // whole file back
    off_t end = lseek(src, 0, SEEK_END);
    lseek(src, 0, SEEK_SET);
    bool ok = end >= 0 && CopyAll(src, dst, end, err);
    if (ok && fsync(dst) != 0) {
        if (err)
            *err = std::string("fsync: ") + strerror(errno);
        ok = false;
    }
    close(src);
    close(dst);
    return ok;
}

bool FsyncNode(const std::string& node, std::string* err) {
    int fd = open(node.c_str(), O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        if (err)
            *err = "open " + node + ": " + strerror(errno);
        return false;
    }
    bool ok = fsync(fd) == 0;
    if (!ok && err)
        *err = std::string("fsync ") + node + ": " + strerror(errno);
    close(fd);
    return ok;
}

std::string ReadbackCheck(const std::string& node, uint64_t size,
                          std::string* err) {
    int fd = open(node.c_str(), O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        if (err)
            *err = "open " + node + ": " + strerror(errno);
        return "";
    }
    // Drop page cache so the hash below reads the storage, not RAM.
    if (ioctl(fd, BLKFLSBUF, 0) != 0) {
        if (err)
            *err = std::string("BLKFLSBUF ") + node + ": " + strerror(errno);
        close(fd);
        return "";
    }
    close(fd);
    return Sha256File(node, size);
}

}  // namespace hs
