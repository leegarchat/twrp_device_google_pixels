/*
 * Copyright (C) 2015 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

#include "FsCrypt.h"

#include "Checkpoint.h"
#include "KeyStorage.h"
#include "KeyUtil.h"
#include "Utils.h"
#include "VoldUtil.h"

#include <algorithm>
#include <map>
#include <optional>
#include <set>
#include <sstream>
#include <string>
#include <vector>

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include <private/android_filesystem_config.h>
#include <private/android_projectid_config.h>

#include "android/os/IVold.h"

#include <cutils/fs.h>
#include <cutils/properties.h>

#include <fscrypt/fscrypt.h>
#include <libdm/dm.h>

#include <android-base/file.h>
#include <android-base/logging.h>
#include <android-base/properties.h>
#include <android-base/stringprintf.h>
#include <android-base/strings.h>
#include <android-base/unique_fd.h>

using android::base::Basename;
using android::base::Realpath;
using android::base::StartsWith;
using android::base::StringPrintf;
using android::fs_mgr::GetEntryForMountPoint;
using android::vold::BuildDataPath;
using android::vold::IsDotOrDotDot;
using android::vold::IsFilesystemSupported;
using android::vold::kEmptyAuthentication;
using android::vold::KeyBuffer;
using android::vold::KeyGeneration;
using android::vold::retrieveKey;
using android::vold::retrieveOrGenerateKey;
using android::vold::SetDefaultAcl;
using android::vold::SetQuotaInherit;
using android::vold::SetQuotaProjectId;
using namespace android::fscrypt;
using namespace android::dm;

namespace {

const std::string device_key_dir = std::string() + DATA_MNT_POINT + fscrypt_unencrypted_folder;
const std::string device_key_path = device_key_dir + "/key";
const std::string device_key_temp = device_key_dir + "/temp";

const std::string user_key_dir = std::string() + DATA_MNT_POINT + "/misc/vold/user_keys";
const std::string user_key_temp = user_key_dir + "/temp";
const std::string prepare_subdirs_path = "/system/bin/vold_prepare_subdirs";

const std::string systemwide_volume_key_dir =
    std::string() + DATA_MNT_POINT + "/misc/vold/volume_keys";

const std::string data_data_dir = std::string() + DATA_MNT_POINT + "/data";
const std::string data_user_0_dir = std::string() + DATA_MNT_POINT + "/user/0";
const std::string media_obb_dir = std::string() + DATA_MNT_POINT + "/media/obb";

// The file encryption options to use on the /data filesystem
EncryptionOptions s_data_options;

// Some users are ephemeral; don't try to store or wipe their keys on disk.
std::set<userid_t> s_ephemeral_users;

// New CE keys that haven't been committed to disk yet
std::map<userid_t, KeyBuffer> s_new_ce_keys;

// CE key fixation operations that have been deferred to checkpoint commit
std::map<std::string, std::string> s_deferred_fixations;

// The system DE encryption policy
EncryptionPolicy s_device_policy;

}  // namespace

// The currently installed CE and DE keys for each user.  Protected by VolumeManager::mCryptLock.
std::map<userid_t, UserPolicies> s_ce_policies;
std::map<userid_t, UserPolicies> s_de_policies;

std::string de_key_raw_ref;
bool retry = true;

// Returns KeyGeneration suitable for key as described in EncryptionOptions
static KeyGeneration makeGen(const EncryptionOptions& options) {
    if (options.version == 0) {
        LOG(ERROR) << "EncryptionOptions not initialized";
        return android::vold::neverGen();
    }
    return KeyGeneration{FSCRYPT_MAX_KEY_SIZE, true, options.use_hw_wrapped_key};
}

static const char* escape_empty(const std::string& value) {
    return value.empty() ? "null" : value.c_str();
}

static std::string get_de_key_path(userid_t user_id) {
    return StringPrintf("%s/de/%d", user_key_dir.c_str(), user_id);
}

static std::string get_ce_key_directory_path(userid_t user_id) {
    return StringPrintf("%s/ce/%d", user_key_dir.c_str(), user_id);
}

// Returns the keys newest first
static std::vector<std::string> get_ce_key_paths(const std::string& directory_path) {
    auto dirp = std::unique_ptr<DIR, int (*)(DIR*)>(opendir(directory_path.c_str()), closedir);
    if (!dirp) {
        PLOG(ERROR) << "Unable to open ce key directory: " + directory_path;
        return std::vector<std::string>();
    }
    std::vector<std::string> result;
    for (;;) {
        errno = 0;
        auto const entry = readdir(dirp.get());
        if (!entry) {
            if (errno) {
                PLOG(ERROR) << "Unable to read ce key directory: " + directory_path;
                return std::vector<std::string>();
            }
            break;
        }
        if (IsDotOrDotDot(*entry)) continue;
        if (entry->d_type != DT_DIR || entry->d_name[0] != 'c') {
            LOG(DEBUG) << "Skipping non-key " << entry->d_name;
            continue;
        }
        result.emplace_back(directory_path + "/" + entry->d_name);
    }
    std::sort(result.begin(), result.end());
    std::reverse(result.begin(), result.end());
    return result;
}

static std::string get_ce_key_current_path(const std::string& directory_path) {
    return directory_path + "/current";
}

static bool get_ce_key_new_path(const std::string& directory_path,
                                const std::vector<std::string>& paths, std::string* ce_key_path) {
    if (paths.empty()) {
        *ce_key_path = get_ce_key_current_path(directory_path);
        return true;
    }
    for (unsigned int i = 0; i < UINT_MAX; i++) {
        auto const candidate = StringPrintf("%s/cx%010u", directory_path.c_str(), i);
        if (paths[0] < candidate) {
            *ce_key_path = candidate;
            return true;
        }
    }
    return false;
}

// Discard all keys but the named one; rename it to canonical name.
static bool fixate_user_ce_key(const std::string& directory_path, const std::string& to_fix,
                               const std::vector<std::string>& paths) {
    bool need_sync = false;
    for (auto const other_path : paths) {
        if (other_path != to_fix) {
            android::vold::destroyKey(other_path);
            need_sync = true;
        }
    }
    auto const current_path = get_ce_key_current_path(directory_path);
    if (to_fix != current_path) {
        LOG(DEBUG) << "Renaming " << to_fix << " to " << current_path;
        if (!android::vold::RenameKeyDir(to_fix, current_path)) return false;
        need_sync = true;
    }
    if (need_sync && !android::vold::FsyncDirectory(directory_path)) return false;
    return true;
}

static bool read_and_fixate_user_ce_key(userid_t user_id,
                                        const android::vold::KeyAuthentication& auth,
                                        KeyBuffer* ce_key) {
    auto const directory_path = get_ce_key_directory_path(user_id);
    auto const paths = get_ce_key_paths(directory_path);
    for (auto const ce_key_path : paths) {
        LOG(INFO) << "Trying user CE key " << ce_key_path;
        if (retrieveKey(ce_key_path, auth, ce_key)) {
            LOG(INFO) << "Successfully retrieved key";
            s_deferred_fixations.erase(directory_path);
            fixate_user_ce_key(directory_path, ce_key_path, paths);
            return true;
        }
    }
    LOG(ERROR) << "Failed to find working ce key for user " << user_id;
    return false;
}

static bool MightBeEmmcStorage(const std::string& blk_device) {
    // Handle symlinks.
    std::string real_path;
    if (!Realpath(blk_device, &real_path)) {
        real_path = blk_device;
    }

    // Handle logical volumes.
    auto& dm = DeviceMapper::Instance();
    for (;;) {
        auto parent = dm.GetParentBlockDeviceByPath(real_path);
        if (!parent.has_value()) break;
        real_path = *parent;
    }

    // Now we should have the "real" block device.
    LOG(DEBUG) << "MightBeEmmcStorage(): blk_device = " << blk_device
               << ", real_path=" << real_path;
    std::string name = Basename(real_path);
    return StartsWith(name, "mmcblk") ||
           // virtio devices may provide inline encryption support that is
           // backed by eMMC inline encryption on the host, thus inheriting the
           // DUN size limitation.  So virtio devices must be allowed here too.
           // TODO(b/207390665): check the maximum DUN size directly instead.
           StartsWith(name, "vd");
}

// Sets s_data_options to the file encryption options for the /data filesystem.
static bool init_data_file_encryption_options() {
    if (fstab_default.empty()) {
        if (!ReadDefaultFstab(&fstab_default)) {
            PLOG(ERROR) << "Failed to open default fstab";
            return false;
        }
    }
    auto entry = GetEntryForMountPoint(&fstab_default, DATA_MNT_POINT);
    if (entry == nullptr) {
        LOG(ERROR) << "No mount point entry for " << DATA_MNT_POINT;
        return false;
    }
    if (!ParseOptions(entry->encryption_options, &s_data_options)) {
        LOG(ERROR) << "Unable to parse encryption options for " << DATA_MNT_POINT ": "
                   << entry->encryption_options;
        return false;
    }
    if ((s_data_options.flags & FSCRYPT_POLICY_FLAG_IV_INO_LBLK_32) &&
        !MightBeEmmcStorage(entry->blk_device)) {
        LOG(ERROR) << "The emmc_optimized encryption flag is only allowed on eMMC storage.  Remove "
                      "this flag from the device's fstab";
        return false;
    }
    if (s_data_options.version == 1 || !retry) {
        s_data_options.use_hw_wrapped_key =
            GetEntryForMountPoint(&fstab_default, DATA_MNT_POINT)->fs_mgr_flags.wrapped_key;
    }
    return true;
}

static bool install_storage_key(const std::string& mountpoint, const EncryptionOptions& options,
                                const KeyBuffer& key, EncryptionPolicy* policy) {
    if (options.version == 0) {
        LOG(ERROR) << "EncryptionOptions not initialized";
        return false;
    }
    KeyBuffer ephemeral_wrapped_key;
    if (options.use_hw_wrapped_key) {
        if (!exportWrappedStorageKey(key, &ephemeral_wrapped_key)) {
            LOG(ERROR) << "Failed to get ephemeral wrapped key";
            return false;
        }
    }
    return installKey(mountpoint, options, options.use_hw_wrapped_key ? ephemeral_wrapped_key : key,
                      policy);
}

// Retrieve the options to use for encryption policies on adoptable storage.
static bool get_volume_file_encryption_options(EncryptionOptions* options) {
    // If we give the empty string, libfscrypt will use the default (currently XTS)
    auto contents_mode = android::base::GetProperty("ro.crypto.volume.contents_mode", "");
    // HEH as default was always a mistake. Use the libfscrypt default (CTS)
    // for devices launching on versions above Android 10.
    auto first_api_level = GetFirstApiLevel();
    auto filenames_mode =
            android::base::GetProperty("ro.crypto.volume.filenames_mode",
                                       first_api_level > __ANDROID_API_Q__ ? "" : "aes-256-heh");
    auto options_string = android::base::GetProperty("ro.crypto.volume.options",
                                                     contents_mode + ":" + filenames_mode);
    if (!ParseOptionsForApiLevel(first_api_level, options_string, options)) {
        LOG(ERROR) << "Unable to parse volume encryption options: " << options_string;
        return false;
    }
    if (options->flags & FSCRYPT_POLICY_FLAG_IV_INO_LBLK_32) {
        LOG(ERROR) << "The emmc_optimized encryption flag is only allowed on eMMC storage.  Remove "
                      "this flag from ro.crypto.volume.options";
        return false;
    }
    return true;
}

bool is_metadata_wrapped_key_supported() {
    return GetEntryForMountPoint(&fstab_default, METADATA_MNT_POINT)->fs_mgr_flags.wrapped_key;
}

// Prepare a directory without assigning it an encryption policy.  The directory
// will inherit the encryption policy of its parent directory, or will be
// unencrypted if the parent directory is unencrypted.
static bool prepare_dir(const std::string& dir, mode_t mode, uid_t uid, gid_t gid) {
    LOG(DEBUG) << "Preparing: " << dir;
    if (android::vold::PrepareDir(dir, mode, uid, gid, 0) != 0) {
        PLOG(ERROR) << "Failed to prepare " << dir;
        return false;
    }
    return true;
}

// Prepare a directory and assign it the given encryption policy.
static bool prepare_dir_with_policy(const std::string& dir, mode_t mode, uid_t uid, gid_t gid,
                                    const EncryptionPolicy& policy) {
    if (android::vold::pathExists(dir)) {
        if (!prepare_dir(dir, mode, uid, gid)) return false;
        if (IsFbeEnabled() && !EnsurePolicy(policy, dir)) return false;
    } else {
        // If the directory does not yet exist, then create it under a temporary name, and only move
        // it to the final name after it is fully prepared with an encryption policy and the desired
        // file permissions.  This prevents the directory from being accessed before it is ready.
        //
        // Note: this relies on the SELinux file_contexts assigning the same type to the file path
        // with the ".new" suffix as to the file path without the ".new" suffix.

        const std::string tmp_dir = dir + ".new";
        if (android::vold::pathExists(tmp_dir)) {
            android::vold::DeleteDirContentsAndDir(tmp_dir);
        }
        if (!prepare_dir(tmp_dir, mode, uid, gid)) return false;
        if (IsFbeEnabled() && !EnsurePolicy(policy, tmp_dir)) return false;

        // On some buggy kernels, renaming a directory that is both encrypted and case-insensitive
        // fails in some specific circumstances.  Unfortunately, these circumstances happen here
        // when processing the "media" directory.  This was already fixed by kernel commit
        // https://git.kernel.org/linus/b5639bb4313b9d45 ('f2fs: don't use casefolded comparison for
        // "." and ".."').  But to support kernels that lack that fix, we use the below workaround.
        // It bypasses the bug by making the encryption key of tmp_dir be loaded before the rename.
        android::vold::pathExists(tmp_dir + "/subdir");

        if (rename(tmp_dir.c_str(), dir.c_str()) != 0) {
            PLOG(ERROR) << "Failed to rename " << tmp_dir << " to " << dir;
            return false;
        }
    }
    return true;
}

static bool destroy_dir(const std::string& dir) {
    LOG(DEBUG) << "Destroying: " << dir;
    if (rmdir(dir.c_str()) != 0 && errno != ENOENT) {
        PLOG(ERROR) << "Failed to destroy " << dir;
        return false;
    }
    return true;
}

// Checks whether the DE key directory exists for the given user.
static bool de_key_exists(userid_t user_id) {
    return android::vold::pathExists(get_de_key_path(user_id));
}

// Checks whether at least one CE key subdirectory exists for the given user.
static bool ce_key_exists(userid_t user_id) {
    auto directory_path = get_ce_key_directory_path(user_id);
    // The common case is that "$dir/current" exists, so check for that first.
    if (android::vold::pathExists(get_ce_key_current_path(directory_path))) return true;

    // Else, there could still be another subdirectory of $dir (if a crash
    // occurred during fixate_user_ce_key()), so check for one.
    return android::vold::pathExists(directory_path) && !get_ce_key_paths(directory_path).empty();
}

static bool create_de_key(userid_t user_id, bool ephemeral) {
    KeyBuffer de_key;
    if (!generateStorageKey(makeGen(s_data_options), &de_key)) return false;
    if (!ephemeral && !android::vold::storeKeyAtomically(get_de_key_path(user_id), user_key_temp,
                                                         kEmptyAuthentication, de_key))
        return false;
    EncryptionPolicy de_policy;
    if (!install_storage_key(DATA_MNT_POINT, s_data_options, de_key, &de_policy)) return false;
    s_de_policies[user_id].internal = de_policy;
    LOG(INFO) << "Created DE key for user " << user_id;
    return true;
}

static bool create_ce_key(userid_t user_id, bool ephemeral) {
    KeyBuffer ce_key;
    if (!generateStorageKey(makeGen(s_data_options), &ce_key)) return false;
    if (!ephemeral) {
        if (!prepare_dir(get_ce_key_directory_path(user_id), 0700, AID_ROOT, AID_ROOT))
            return false;
        // We don't store the CE key on disk here, since here we don't have the
        // secret needed to do so securely.  Instead, we cache it in memory for
        // now, and we store it later in fscrypt_set_ce_key_protection().
        s_new_ce_keys.insert({user_id, ce_key});
    }
    EncryptionPolicy ce_policy;
    if (!install_storage_key(DATA_MNT_POINT, s_data_options, ce_key, &ce_policy)) return false;
    s_ce_policies[user_id].internal = ce_policy;
    LOG(INFO) << "Created CE key for user " << user_id;
    return true;
}

static bool is_numeric(const char* name) {
    for (const char* p = name; *p != '\0'; p++) {
        if (!isdigit(*p)) return false;
    }
    return true;
}

static bool load_all_de_keys() {
    auto de_dir = user_key_dir + "/de";
    auto dirp = std::unique_ptr<DIR, int (*)(DIR*)>(opendir(de_dir.c_str()), closedir);
    if (!dirp) {
        PLOG(ERROR) << "Unable to read de key directory";
        return false;
    }
    for (;;) {
        errno = 0;
        auto entry = readdir(dirp.get());
        if (!entry) {
            if (errno) {
                PLOG(ERROR) << "Unable to read de key directory";
                return false;
            }
            break;
        }
        if (IsDotOrDotDot(*entry)) continue;
        if (entry->d_type != DT_DIR || !is_numeric(entry->d_name)) {
            LOG(DEBUG) << "Skipping non-de-key " << entry->d_name;
            continue;
        }
        userid_t user_id = std::stoi(entry->d_name);
        auto key_path = de_dir + "/" + entry->d_name;
        KeyBuffer de_key;
        LOG(INFO) << "fscrypt::load_all_de_keys::retrieveKey";
        if (!retrieveKey(key_path, kEmptyAuthentication, &de_key)) {
            // This is probably a partially removed user, so ignore
            if (user_id != 0) continue;
            return false;
        }
        EncryptionPolicy de_policy;
        if (!install_storage_key(DATA_MNT_POINT, s_data_options, de_key, &de_policy)) return false;
        const auto& [existing, is_new] = s_de_policies.insert({user_id, {de_policy, {}}});
        if (!is_new && existing->second.internal != de_policy) {
            LOG(ERROR) << "DE policy for user" << user_id << " changed";
            return false;
        }
        LOG(INFO) << "Installed de key for user " << user_id;
        std::string user_prop = "twrp.user." + std::to_string(user_id) + ".decrypt";
        property_set(user_prop.c_str(), "0");
    }
    // fscrypt:TODO: go through all DE directories, ensure that all user dirs have the
    // correct policy set on them, and that no rogue ones exist.
    return true;
}

bool fscrypt_initialize_systemwide_keys() {
    LOG(INFO) << "fscrypt_initialize_systemwide_keys";

    if (!init_data_file_encryption_options()) return false;

    EncryptionOptions options;
    if (!get_volume_file_encryption_options(&options)) return false;

    KeyBuffer device_key;
install:
    if (!retrieveOrGenerateKey(device_key_path, device_key_temp, kEmptyAuthentication,
                               makeGen(s_data_options), &device_key))
        return false;

    // This initializes s_device_policy, which is a global variable so that
    // fscrypt_init_user0() can access it later.
    if (!install_storage_key(DATA_MNT_POINT, s_data_options, device_key, &s_device_policy)) {
        if (retry) {
            printf("Trying %s wrappedkey\n", options.use_hw_wrapped_key ? "without" : "with");
            GetEntryForMountPoint(&fstab_default, DATA_MNT_POINT)->fs_mgr_flags.wrapped_key =
                options.use_hw_wrapped_key = !options.use_hw_wrapped_key;
            retry = false;
            goto install;
        }
        return false;
    }

    std::string options_string;
    if (!OptionsToString(s_device_policy.options, &options_string)) {
        LOG(ERROR) << "Unable to serialize options";
        return false;
    }
    std::string options_filename = std::string(DATA_MNT_POINT) + fscrypt_key_mode;
    if (!android::vold::writeStringToFile(options_string, options_filename)) return false;

    std::string ref_filename = std::string(DATA_MNT_POINT) + fscrypt_key_ref;
    de_key_raw_ref = s_device_policy.key_raw_ref;
    if (!android::vold::writeStringToFile(s_device_policy.key_raw_ref, ref_filename)) return false;
    LOG(INFO) << "Wrote system DE key reference to:" << ref_filename;

    KeyBuffer per_boot_key;
    if (!generateStorageKey(makeGen(s_data_options), &per_boot_key)) return false;
    EncryptionPolicy per_boot_policy;
    if (!install_storage_key(DATA_MNT_POINT, s_data_options, per_boot_key, &per_boot_policy))
        return false;
    std::string per_boot_ref_filename = std::string("/data") + fscrypt_key_per_boot_ref;
    if (!android::vold::writeStringToFile(per_boot_policy.key_raw_ref, per_boot_ref_filename))
        return false;
    LOG(INFO) << "Wrote per boot key reference to:" << per_boot_ref_filename;

    return true;
}

static bool prepare_special_dirs() {
    // Ensure that /data/data and its "alias" /data/user/0 exist, and create the
    // bind mount of /data/data onto /data/user/0.  This *should* happen in
    // fscrypt_prepare_user_storage().  However, it actually must be done early,
    // before the rest of user 0's CE storage is prepared.  This is because
    // zygote may need to set up app data isolation before then, which requires
    // mounting a tmpfs over /data/data to ensure it remains hidden.  This issue
    // arises due to /data/data being in the top-level directory.

    // /data/user/0 used to be a symlink to /data/data, so we must first delete
    // the old symlink if present.
    if (android::vold::IsSymlink(data_user_0_dir) && android::vold::Unlink(data_user_0_dir) != 0)
        return false;
    // On first boot, we'll be creating /data/data for the first time, and user
    // 0's CE key will be installed already since it was just created.  Take the
    // opportunity to also set the encryption policy of /data/data right away.
    if (s_ce_policies.count(0) != 0) {
        const EncryptionPolicy& ce_policy = s_ce_policies[0].internal;
        if (!prepare_dir_with_policy(data_data_dir, 0771, AID_SYSTEM, AID_SYSTEM, ce_policy)) {
            // Preparing /data/data failed, yet we had just generated a new CE
            // key because one wasn't stored.  Before erroring out, try deleting
            // the directory and retrying, as it's possible that the directory
            // exists with different CE policy from an interrupted first boot.
            if (rmdir(data_data_dir.c_str()) != 0) {
                PLOG(ERROR) << "rmdir " << data_data_dir << " failed";
            }
            if (!prepare_dir_with_policy(data_data_dir, 0771, AID_SYSTEM, AID_SYSTEM, ce_policy))
                return false;
        }
    } else {
        if (!prepare_dir(data_data_dir, 0771, AID_SYSTEM, AID_SYSTEM)) return false;
        // EnsurePolicy() will have to happen later, in fscrypt_prepare_user_storage().
    }
    if (!prepare_dir(data_user_0_dir, 0700, AID_SYSTEM, AID_SYSTEM)) return false;
    if (android::vold::BindMount(data_data_dir, data_user_0_dir) != 0) return false;

    // If /data/media/obb doesn't exist, create it and encrypt it with the
    // device policy.  Normally, device-policy-encrypted directories are created
    // and encrypted by init; /data/media/obb is special because it is located
    // in /data/media.  Since /data/media also contains per-user encrypted
    // directories, by design only vold can write to it.  As a side effect of
    // that, vold must create /data/media/obb.
    //
    // We must tolerate /data/media/obb being unencrypted if it already exists
    // on-disk, since it used to be unencrypted (b/64566063).
    if (android::vold::pathExists(media_obb_dir)) {
        if (!prepare_dir(media_obb_dir, 0770, AID_MEDIA_RW, AID_MEDIA_RW)) return false;
    } else {
        if (!prepare_dir_with_policy(media_obb_dir, 0770, AID_MEDIA_RW, AID_MEDIA_RW,
                                     s_device_policy))
            return false;
    }
    return true;
}

bool fscrypt_init_user0_done;

bool fscrypt_init_user0() {
    LOG(INFO) << "fscrypt_init_user0";

    if (IsFbeEnabled()) {
        if (!prepare_dir(user_key_dir, 0700, AID_ROOT, AID_ROOT)) return false;
        if (!prepare_dir(user_key_dir + "/ce", 0700, AID_ROOT, AID_ROOT)) return false;
        if (!prepare_dir(user_key_dir + "/de", 0700, AID_ROOT, AID_ROOT)) return false;

        // Create user 0's DE and CE keys if they don't already exist.  Check
        // each key independently, since if the first boot was interrupted it is
        // possible that the DE key exists but the CE key does not.
        if (!de_key_exists(0) && !create_de_key(0, false)) return false;
        if (!ce_key_exists(0) && !create_ce_key(0, false)) return false;

        // TODO: switch to loading only DE_0 here once framework makes
        // explicit calls to install DE keys for secondary users
        if (!load_all_de_keys()) return false;
    }

    // Now that user 0's CE key has been created, we can prepare /data/data.
    if (!prepare_special_dirs()) return false;

    // With the exception of what is done by prepare_special_dirs() above, we
    // only prepare DE storage here, since user 0's CE key won't be installed
    // yet unless it was just created.  The framework will prepare the user's CE
    // storage later, once their CE key is installed.
    if (!fscrypt_prepare_user_storage("", 0, android::os::IVold::STORAGE_FLAG_DE)) {
        LOG(ERROR) << "Failed to prepare user 0 storage";
        return false;
    }

    fscrypt_init_user0_done = true;
    return true;
}

// Creates the CE and DE keys for a new user.
bool fscrypt_create_user_keys(userid_t user_id, bool ephemeral) {
    LOG(DEBUG) << "fscrypt_create_user_keys for " << user_id;
    if (!IsFbeEnabled()) {
        return true;
    }
    // FIXME test for existence of key that is not loaded yet
    if (s_ce_policies.count(user_id) != 0) {
        LOG(ERROR) << "Already exists, can't create keys for " << user_id;
        // FIXME should we fail the command?
        return true;
    }
    if (!create_de_key(user_id, ephemeral)) return false;
    if (!create_ce_key(user_id, ephemeral)) return false;
    if (ephemeral) s_ephemeral_users.insert(user_id);
    return true;
}

// Evicts all the user's keys of one type from all volumes (internal and adoptable).
// This evicts either CE keys or DE keys, depending on which map is passed.
static bool evict_user_keys(std::map<userid_t, UserPolicies>& policy_map, userid_t user_id) {
    bool success = true;
    auto it = policy_map.find(user_id);
    if (it != policy_map.end()) {
        const UserPolicies& policies = it->second;
        success &= android::vold::evictKey(BuildDataPath(""), policies.internal);
        for (const auto& [volume_uuid, policy] : policies.adoptable) {
            success &= android::vold::evictKey(BuildDataPath(volume_uuid), policy);
        }
        policy_map.erase(it);
    }
    return success;
}

// Evicts and destroys all CE and DE keys for a user.  This is called when the user is removed.
bool fscrypt_destroy_user_keys(userid_t user_id) {
    LOG(DEBUG) << "fscrypt_destroy_user_keys(" << user_id << ")";
    if (!IsFbeEnabled()) {
        return true;
    }
    bool success = true;

    success &= evict_user_keys(s_ce_policies, user_id);
    success &= evict_user_keys(s_de_policies, user_id);

    if (!s_ephemeral_users.erase(user_id)) {
        auto ce_path = get_ce_key_directory_path(user_id);
        if (!s_new_ce_keys.erase(user_id)) {
            for (auto const path : get_ce_key_paths(ce_path)) {
                success &= android::vold::destroyKey(path);
            }
        }
        s_deferred_fixations.erase(ce_path);
        success &= destroy_dir(ce_path);

        auto de_key_path = get_de_key_path(user_id);
        if (android::vold::pathExists(de_key_path)) {
            success &= android::vold::destroyKey(de_key_path);
        } else {
            LOG(INFO) << "Not present so not erasing: " << de_key_path;
        }
    }
    return success;
}

static bool parse_hex(const std::string& hex, std::string* result) {
    if (hex == "!") {
        *result = "";
        return true;
    }
    if (android::vold::HexToStr(hex, *result) != 0) {
        LOG(ERROR) << "Invalid FBE hex string";  // Don't log the string for security reasons
        return false;
    }
    return true;
}

static std::optional<android::vold::KeyAuthentication> authentication_from_hex(
        const std::string& secret_hex) {
    std::string secret;
    if (!parse_hex(secret_hex, &secret)) return std::optional<android::vold::KeyAuthentication>();
    if (secret.empty()) {
        return kEmptyAuthentication;
    } else {
        return android::vold::KeyAuthentication(secret);
    }
}

static std::string volkey_path(const std::string& misc_path, const std::string& volume_uuid) {
    return misc_path + "/vold/volume_keys/" + volume_uuid + "/default";
}

static std::string volume_secdiscardable_path(const std::string& volume_uuid) {
    return systemwide_volume_key_dir + "/" + volume_uuid + "/secdiscardable";
}

static bool read_or_create_volkey(const std::string& misc_path, const std::string& volume_uuid,
                                  UserPolicies& user_policies, EncryptionPolicy* policy) {
    auto secdiscardable_path = volume_secdiscardable_path(volume_uuid);
    std::string secdiscardable_hash;
    if (android::vold::pathExists(secdiscardable_path)) {
        if (!android::vold::readSecdiscardable(secdiscardable_path, &secdiscardable_hash))
            return false;
    } else {
        if (!android::vold::MkdirsSync(secdiscardable_path, 0700)) return false;
        if (!android::vold::createSecdiscardable(secdiscardable_path, &secdiscardable_hash))
            return false;
    }
    auto key_path = volkey_path(misc_path, volume_uuid);
    if (!android::vold::MkdirsSync(key_path, 0700)) return false;
    android::vold::KeyAuthentication auth(secdiscardable_hash);

    EncryptionOptions options;
    if (!get_volume_file_encryption_options(&options)) return false;
    KeyBuffer key;
    if (!retrieveOrGenerateKey(key_path, key_path + "_tmp", auth, makeGen(options), &key))
        return false;
    if (!install_storage_key(BuildDataPath(volume_uuid), options, key, policy)) return false;
    user_policies.adoptable[volume_uuid] = *policy;
    return true;
}

static bool destroy_volkey(const std::string& misc_path, const std::string& volume_uuid) {
    auto path = volkey_path(misc_path, volume_uuid);
    if (!android::vold::pathExists(path)) return true;
    return android::vold::destroyKey(path);
}

// (Re-)encrypts the user's CE key with the given secret.  This function handles
// storing the CE key for a new user for the first time.  It also handles
// re-encrypting the CE key upon upgrade from an Android version where the CE
// key was stored with kEmptyAuthentication when the user didn't have an LSKF.
// See the comments below for the different cases handled.
bool fscrypt_set_ce_key_protection(userid_t user_id, const std::string& secret_hex) {
    LOG(DEBUG) << "fscrypt_set_ce_key_protection " << user_id;
    if (!IsFbeEnabled()) return true;
    auto auth = authentication_from_hex(secret_hex);
    if (!auth) return false;
    if (auth->secret.empty()) {
        LOG(ERROR) << "fscrypt_set_ce_key_protection: secret must be nonempty";
        return false;
    }
    // We shouldn't store any keys for ephemeral users.
    if (s_ephemeral_users.count(user_id) != 0) {
        LOG(DEBUG) << "Not storing key because user is ephemeral";
        return true;
    }
    KeyBuffer ce_key;
    auto it = s_new_ce_keys.find(user_id);
    if (it != s_new_ce_keys.end()) {
        // If the key exists in s_new_ce_keys, then the key is a
        // not-yet-committed key for a new user, and we are committing it here.
        // This happens when the user's synthetic password is created.
        ce_key = it->second;
    } else if (ce_key_exists(user_id)) {
        // If the key doesn't exist in s_new_ce_keys but does exist on-disk,
        // then we are setting the protection on an existing key.  This happens
        // at upgrade time, when CE keys that were previously protected by
        // kEmptyAuthentication are encrypted by the user's synthetic password.
        LOG(INFO) << "CE key already exists on-disk; re-protecting it with the given secret";
        if (!read_and_fixate_user_ce_key(user_id, kEmptyAuthentication, &ce_key)) {
            LOG(ERROR) << "Failed to retrieve CE key for user " << user_id << " using empty auth";
            // Before failing, also check whether the key is already protected
            // with the given secret.  This isn't expected, but in theory it
            // could happen if an upgrade is requested for a user more than once
            // due to a power-off or other interruption.
            if (read_and_fixate_user_ce_key(user_id, *auth, &ce_key)) {
                LOG(WARNING) << "CE key is already protected by given secret";
                return true;
            }
            // The key isn't protected by either kEmptyAuthentication or by
            // |auth|.  This should never happen, and there's nothing we can do
            // besides return an error.
            return false;
        }
    } else {
        // If the key doesn't exist in memory or on-disk, then we need to
        // generate it here, then commit it to disk.  This is needed after the
        // unusual case where a non-system user was created during early boot,
        // and then the device was force-rebooted before the boot completed.  In
        // that case, the Android user record was committed but the CE key was
        // not.  So the CE key was lost, and we need to regenerate it.  This
        // should be fine, since the key should not have been used yet.
        LOG(WARNING) << "CE key not found!  Regenerating it";
        if (!create_ce_key(user_id, false)) return false;
        ce_key = s_new_ce_keys.find(user_id)->second;
    }

    auto const directory_path = get_ce_key_directory_path(user_id);
    auto const paths = get_ce_key_paths(directory_path);
    std::string ce_key_path;
    if (!get_ce_key_new_path(directory_path, paths, &ce_key_path)) return false;
    if (!android::vold::storeKeyAtomically(ce_key_path, user_key_temp, *auth, ce_key)) return false;

    // Fixate the key, i.e. delete all other bindings of it.  (In practice this
    // just means the kEmptyAuthentication binding, if there is one.)  However,
    // if a userdata filesystem checkpoint is pending, then we need to delay the
    // fixation until the checkpoint has been committed, since deleting keys
    // from Keystore cannot be rolled back.
    if (android::vold::cp_needsCheckpoint()) {
        LOG(INFO) << "Deferring fixation of " << directory_path << " until checkpoint is committed";
        s_deferred_fixations[directory_path] = ce_key_path;
    } else {
        s_deferred_fixations.erase(directory_path);
        if (!fixate_user_ce_key(directory_path, ce_key_path, paths)) return false;
    }

    if (s_new_ce_keys.erase(user_id)) {
        LOG(INFO) << "Stored CE key for new user " << user_id;
    }
    return true;
}

void fscrypt_deferred_fixate_ce_keys() {
    for (const auto& it : s_deferred_fixations) {
        const auto& directory_path = it.first;
        const auto& to_fix = it.second;
        LOG(INFO) << "Doing deferred fixation of " << directory_path;
        fixate_user_ce_key(directory_path, to_fix, get_ce_key_paths(directory_path));
        // Continue on error.
    }
    s_deferred_fixations.clear();
}

std::vector<int> fscrypt_get_unlocked_users() {
    std::vector<int> user_ids;
    for (const auto& [user_id, user_policies] : s_ce_policies) {
        user_ids.push_back(user_id);
    }
    return user_ids;
}

// Unlocks internal CE storage for the given user.  This only unlocks internal storage, since
// fscrypt_prepare_user_storage() has to be called for each adoptable storage volume anyway (since
// the volume might have been absent when the user was created), and that handles the unlocking.
bool fscrypt_unlock_ce_storage(userid_t user_id, const std::string& secret_hex) {
    LOG(INFO) << "fscrypt_unlock_ce_storage " << user_id;
    if (!IsFbeEnabled()) return true;
    if (s_ce_policies.count(user_id) != 0) {
        LOG(WARNING) << "CE storage for user " << user_id << " is already unlocked";
        return true;
    }
    auto auth = authentication_from_hex(secret_hex);
    if (!auth) return false;
    KeyBuffer ce_key;
    if (!read_and_fixate_user_ce_key(user_id, *auth, &ce_key)) return false;
    EncryptionPolicy ce_policy;
    if (!install_storage_key(DATA_MNT_POINT, s_data_options, ce_key, &ce_policy)) return false;
    s_ce_policies[user_id].internal = ce_policy;
    LOG(INFO) << "Installed CE key for user " << user_id;
    return true;
}

// Locks CE storage for the given user.  This locks both internal and adoptable storage.
bool fscrypt_lock_ce_storage(userid_t user_id) {
    LOG(DEBUG) << "fscrypt_lock_ce_storage " << user_id;
    if (!IsFbeEnabled()) return true;
    return evict_user_keys(s_ce_policies, user_id);
}

static bool prepare_subdirs(const std::string& action, const std::string& volume_uuid,
                            userid_t user_id, int flags) {
    if (0 != android::vold::ForkExecvp(
                 std::vector<std::string>{prepare_subdirs_path, action, volume_uuid,
                                          std::to_string(user_id), std::to_string(flags)})) {
        LOG(ERROR) << "vold_prepare_subdirs failed";
        return false;
    }
    return true;
}

bool fscrypt_prepare_user_storage(const std::string& volume_uuid, userid_t user_id, int flags) {
    LOG(INFO) << "fscrypt_prepare_user_storage for volume " << escape_empty(volume_uuid)
               << ", user " << user_id << ", flags " << flags;

    // Internal storage must be prepared before adoptable storage, since the
    // user's volume keys are stored in their internal storage.
    if (!volume_uuid.empty()) {
        if ((flags & android::os::IVold::STORAGE_FLAG_DE) &&
            !android::vold::pathExists(android::vold::BuildDataMiscDePath("", user_id))) {
            LOG(ERROR) << "Cannot prepare DE storage for user " << user_id << " on volume "
                       << volume_uuid << " before internal storage";
            return false;
        }
        if ((flags & android::os::IVold::STORAGE_FLAG_CE) &&
            !android::vold::pathExists(android::vold::BuildDataMiscCePath("", user_id))) {
            LOG(ERROR) << "Cannot prepare CE storage for user " << user_id << " on volume "
                       << volume_uuid << " before internal storage";
            return false;
        }
    }

    if (flags & android::os::IVold::STORAGE_FLAG_DE) {
        // DE_sys key
        auto system_legacy_path = android::vold::BuildDataSystemLegacyPath(user_id);
        auto profiles_de_path = android::vold::BuildDataProfilesDePath(user_id);

        // DE_n key
        EncryptionPolicy de_policy;
        auto system_de_path = android::vold::BuildDataSystemDePath(user_id);
        auto misc_de_path = android::vold::BuildDataMiscDePath(volume_uuid, user_id);
        auto vendor_de_path = android::vold::BuildDataVendorDePath(user_id);
        auto user_de_path = android::vold::BuildDataUserDePath(volume_uuid, user_id);

        if (IsFbeEnabled()) {
            auto it = s_de_policies.find(user_id);
            if (it == s_de_policies.end()) {
                LOG(ERROR) << "Cannot find DE policy for user " << user_id;
                return false;
            }
            UserPolicies& user_de_policies = it->second;
            if (volume_uuid.empty()) {
                de_policy = user_de_policies.internal;
            } else {
                auto misc_de_empty_volume_path = android::vold::BuildDataMiscDePath("", user_id);
                if (!read_or_create_volkey(misc_de_empty_volume_path, volume_uuid, user_de_policies,
                                           &de_policy)) {
                    return false;
                }
            }
        }

        if (volume_uuid.empty()) {
            if (!prepare_dir(system_legacy_path, 0700, AID_SYSTEM, AID_SYSTEM)) return false;
            if (!prepare_dir(profiles_de_path, 0771, AID_SYSTEM, AID_SYSTEM)) return false;

            if (!prepare_dir_with_policy(system_de_path, 0770, AID_SYSTEM, AID_SYSTEM, de_policy))
                return false;
            if (!prepare_dir_with_policy(vendor_de_path, 0771, AID_ROOT, AID_ROOT, de_policy))
                return false;
        }

        if (!prepare_dir_with_policy(misc_de_path, 01771, AID_SYSTEM, AID_MISC, de_policy))
            return false;
        if (!prepare_dir_with_policy(user_de_path, 0771, AID_SYSTEM, AID_SYSTEM, de_policy))
            return false;
    }
    if (flags & android::os::IVold::STORAGE_FLAG_CE) {
        // CE_n key
        EncryptionPolicy ce_policy;
        auto system_ce_path = android::vold::BuildDataSystemCePath(user_id);
        auto misc_ce_path = android::vold::BuildDataMiscCePath(volume_uuid, user_id);
        auto vendor_ce_path = android::vold::BuildDataVendorCePath(user_id);
        auto media_ce_path = android::vold::BuildDataMediaCePath(volume_uuid, user_id);
        auto user_ce_path = android::vold::BuildDataUserCePath(volume_uuid, user_id);
        LOG(INFO) << "fscrypt_prepare_user_storage";

        if (IsFbeEnabled()) {
            auto it = s_ce_policies.find(user_id);
            if (it == s_ce_policies.end()) {
                LOG(ERROR) << "Cannot find CE policy for user " << user_id;
                return false;
            }
            UserPolicies& user_ce_policies = it->second;
            if (volume_uuid.empty()) {
                ce_policy = user_ce_policies.internal;
            } else {
                auto misc_ce_empty_volume_path = android::vold::BuildDataMiscCePath("", user_id);
                if (!read_or_create_volkey(misc_ce_empty_volume_path, volume_uuid, user_ce_policies,
                                           &ce_policy)) {
                    return false;
                }
            }
        }

        if (volume_uuid.empty()) {
            if (!prepare_dir_with_policy(system_ce_path, 0770, AID_SYSTEM, AID_SYSTEM, ce_policy))
                return false;
            if (!prepare_dir_with_policy(vendor_ce_path, 0771, AID_ROOT, AID_ROOT, ce_policy))
                return false;
        }
        if (!prepare_dir_with_policy(media_ce_path, 02770, AID_MEDIA_RW, AID_MEDIA_RW, ce_policy))
            return false;
        // On devices without sdcardfs (kernel 5.4+), the path permissions aren't fixed
        // up automatically; therefore, use a default ACL, to ensure apps with MEDIA_RW
        // can keep reading external storage; in particular, this allows app cloning
        // scenarios to work correctly on such devices.
        int ret = SetDefaultAcl(media_ce_path, 02770, AID_MEDIA_RW, AID_MEDIA_RW, {AID_MEDIA_RW});
        if (ret != android::OK) {
            return false;
        }
        if (!prepare_dir_with_policy(misc_ce_path, 01771, AID_SYSTEM, AID_MISC, ce_policy))
            return false;
        if (!prepare_dir_with_policy(user_ce_path, 0771, AID_SYSTEM, AID_SYSTEM, ce_policy))
            return false;

        if (volume_uuid.empty()) {
            // Now that credentials have been installed, we can run restorecon
            // over these paths
            // NOTE: these paths need to be kept in sync with libselinux
            android::vold::RestoreconRecursive(system_ce_path);
            android::vold::RestoreconRecursive(vendor_ce_path);
            android::vold::RestoreconRecursive(misc_ce_path);
        }
    }
    if (!prepare_subdirs("prepare", volume_uuid, user_id, flags)) return false;

    return true;
}

bool fscrypt_destroy_user_storage(const std::string& volume_uuid, userid_t user_id, int flags) {
    LOG(DEBUG) << "fscrypt_destroy_user_storage for volume " << escape_empty(volume_uuid)
               << ", user " << user_id << ", flags " << flags;
    bool res = true;

    res &= prepare_subdirs("destroy", volume_uuid, user_id, flags);

    if (flags & android::os::IVold::STORAGE_FLAG_CE) {
        // CE_n key
        auto system_ce_path = android::vold::BuildDataSystemCePath(user_id);
        auto misc_ce_path = android::vold::BuildDataMiscCePath(volume_uuid, user_id);
        auto vendor_ce_path = android::vold::BuildDataVendorCePath(user_id);
        auto media_ce_path = android::vold::BuildDataMediaCePath(volume_uuid, user_id);
        auto user_ce_path = android::vold::BuildDataUserCePath(volume_uuid, user_id);

        res &= destroy_dir(media_ce_path);
        res &= destroy_dir(misc_ce_path);
        res &= destroy_dir(user_ce_path);
        if (volume_uuid.empty()) {
            res &= destroy_dir(system_ce_path);
            res &= destroy_dir(vendor_ce_path);
        } else {
            if (IsFbeEnabled()) {
                auto misc_ce_empty_volume_path = android::vold::BuildDataMiscCePath("", user_id);
                res &= destroy_volkey(misc_ce_empty_volume_path, volume_uuid);
            }
        }
    }

    if (flags & android::os::IVold::STORAGE_FLAG_DE) {
        // DE_sys key
        auto system_legacy_path = android::vold::BuildDataSystemLegacyPath(user_id);
        auto profiles_de_path = android::vold::BuildDataProfilesDePath(user_id);

        // DE_n key
        auto system_de_path = android::vold::BuildDataSystemDePath(user_id);
        auto misc_de_path = android::vold::BuildDataMiscDePath(volume_uuid, user_id);
        auto vendor_de_path = android::vold::BuildDataVendorDePath(user_id);
        auto user_de_path = android::vold::BuildDataUserDePath(volume_uuid, user_id);

        res &= destroy_dir(user_de_path);
        res &= destroy_dir(misc_de_path);
        if (volume_uuid.empty()) {
            res &= destroy_dir(system_legacy_path);
            res &= destroy_dir(profiles_de_path);
            res &= destroy_dir(system_de_path);
            res &= destroy_dir(vendor_de_path);
        } else {
            if (IsFbeEnabled()) {
                auto misc_de_empty_volume_path = android::vold::BuildDataMiscDePath("", user_id);
                res &= destroy_volkey(misc_de_empty_volume_path, volume_uuid);
            }
        }
    }

    return res;
}

static bool destroy_volume_keys(const std::string& directory_path, const std::string& volume_uuid) {
    auto dirp = std::unique_ptr<DIR, int (*)(DIR*)>(opendir(directory_path.c_str()), closedir);
    if (!dirp) {
        PLOG(ERROR) << "Unable to open directory: " + directory_path;
        return false;
    }
    bool res = true;
    for (;;) {
        errno = 0;
        auto const entry = readdir(dirp.get());
        if (!entry) {
            if (errno) {
                PLOG(ERROR) << "Unable to read directory: " + directory_path;
                return false;
            }
            break;
        }
        if (IsDotOrDotDot(*entry)) continue;
        if (entry->d_type != DT_DIR || entry->d_name[0] == '.') {
            LOG(DEBUG) << "Skipping non-user " << entry->d_name;
            continue;
        }
        res &= destroy_volkey(directory_path + "/" + entry->d_name, volume_uuid);
    }
    return res;
}

static void erase_volume_policies(std::map<userid_t, UserPolicies>& policy_map,
                                  const std::string& volume_uuid) {
    for (auto& [user_id, user_policies] : policy_map) {
        user_policies.adoptable.erase(volume_uuid);
    }
}

// Destroys all CE and DE keys for an adoptable storage volume that is permanently going away.
// Requires VolumeManager::mCryptLock.
bool fscrypt_destroy_volume_keys(const std::string& volume_uuid) {
    if (!IsFbeEnabled()) return true;
    bool res = true;
    LOG(DEBUG) << "fscrypt_destroy_volume_keys for volume " << escape_empty(volume_uuid);
    auto secdiscardable_path = volume_secdiscardable_path(volume_uuid);
    res &= android::vold::runSecdiscardSingle(secdiscardable_path);
    res &= destroy_volume_keys("/data/misc_ce", volume_uuid);
    res &= destroy_volume_keys("/data/misc_de", volume_uuid);
    // Drop the CE and DE policies stored in memory, as they are not needed anymore.  Note that it's
    // not necessary to also evict the corresponding keys from the kernel, as that happens
    // automatically as a result of the volume being unmounted.
    erase_volume_policies(s_ce_policies, volume_uuid);
    erase_volume_policies(s_de_policies, volume_uuid);
    return res;
}

bool lookup_key_ref(const std::map<userid_t, UserPolicies>& policy_map, userid_t user_id,
                           std::string* raw_ref) {
    auto refi = policy_map.find(user_id);
    if (refi == policy_map.end()) {
        LOG(ERROR) << "Cannot find key for " << user_id;
        return false;
    }
    *raw_ref = refi->second.internal.key_raw_ref;
    return true;
}
