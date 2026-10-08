/*
 * Copyright (C) 2016 The Android Open Source Project
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

#include "MetadataCrypt.h"
#include "KeyBuffer.h"

#include <fstream>
#include <string>

#include <fcntl.h>
#include <sys/param.h>
#include <sys/stat.h>
#include <sys/types.h>

#include <android-base/logging.h>
#include <android-base/properties.h>
#include <android-base/strings.h>
#include <android-base/unique_fd.h>
#include <cutils/fs.h>
#include <libdm/dm.h>
#include <libgsi/libgsi.h>

#include "Checkpoint.h"
#include "CryptoType.h"
#include "EncryptInplace.h"
#include "FsCrypt.h"
#include "KeyStorage.h"
#include "KeyUtil.h"
#include "Keystore.h"
#include "Utils.h"
#include "VoldUtil.h"
#include "fs/Ext4.h"
#include "fs/F2fs.h"

namespace android {
namespace vold {

using android::base::Basename;
using android::fs_mgr::FstabEntry;
using android::fs_mgr::GetEntryForMountPoint;
using android::fscrypt::GetFirstApiLevel;
using android::vold::KeyBuffer;
using namespace android::dm;
using namespace std::chrono_literals;

// Parsed from metadata options
struct CryptoOptions {
    struct CryptoType cipher = invalid_crypto_type;
    bool use_legacy_options_format = false;
    bool set_dun = true;  // Non-legacy driver always sets DUN
    bool use_hw_wrapped_key = false;
};

static const std::string kDmNameUserdata = "userdata";

// The first entry in this table is the default crypto type.
constexpr CryptoType supported_crypto_types[] = {aes_256_xts, adiantum};

static_assert(validateSupportedCryptoTypes(64, supported_crypto_types,
                                           array_length(supported_crypto_types)),
              "We have a CryptoType which was incompletely constructed.");

constexpr CryptoType legacy_aes_256_xts =
        CryptoType().set_config_name("aes-256-xts").set_kernel_name("AES-256-XTS").set_keysize(64);

static_assert(isValidCryptoType(64, legacy_aes_256_xts),
              "We have a CryptoType which was incompletely constructed.");

// Returns KeyGeneration suitable for key as described in CryptoOptions
const KeyGeneration makeGen(const CryptoOptions& options) {
    return KeyGeneration{options.cipher.get_keysize(), true, options.use_hw_wrapped_key};
}

void defaultkey_precreate_dm_device() {
    auto& dm = DeviceMapper::Instance();
    if (dm.GetState(kDmNameUserdata) != DmDeviceState::INVALID) {
        LOG(INFO) << "Not pre-creating userdata encryption device; device already exists";
        return;
    }

    if (!dm.CreatePlaceholderDevice(kDmNameUserdata)) {
        LOG(ERROR) << "Failed to pre-create userdata metadata encryption device";
    }
}

static bool mount_via_fs_mgr(const char* mount_point, const char* blk_device, bool needs_encrypt) {
    // fs_mgr_do_mount runs fsck. Use setexeccon to run trusted
    // partitions in the fsck domain.
    if (setexeccon(android::vold::sFsckContext)) {
        PLOG(ERROR) << "Failed to setexeccon";
        return false;
    }
    auto mount_rc = fs_mgr_do_mount(&fstab_default, mount_point, blk_device,
                                    android::vold::cp_needsCheckpoint(), needs_encrypt);
    if (setexeccon(nullptr)) {
        PLOG(ERROR) << "Failed to clear setexeccon";
        return false;
    }
    if (mount_rc != 0) {
        LOG(ERROR) << "fs_mgr_do_mount failed with rc " << mount_rc;
        return false;
    }
    LOG(INFO) << "Mounted " << mount_point;
    return true;
}

static bool read_key(const std::string& metadata_key_dir, const KeyGeneration& gen, bool first_key,
                     KeyBuffer* key) {
    if (metadata_key_dir.empty()) {
        LOG(ERROR) << "Failed to get metadata_key_dir";
        return false;
    }
    std::string sKey;
    auto dir = metadata_key_dir + "/key";
    LOG(INFO) << "metadata_key_dir/key: " << dir;
    if (!MkdirsSync(dir, 0700)) return false;
    auto in_dsu = android::base::GetBoolProperty("ro.gsid.image_running", false);
    // !pathExists(dir) does not imply there's a factory reset when in DSU mode.
    if (!pathExists(dir) && !in_dsu && first_key) {
        auto delete_all = android::base::GetBoolProperty(
                "ro.crypto.metadata_init_delete_all_keys.enabled", false);
        if (delete_all) {
            LOG(INFO) << "Metadata key does not exist, calling deleteAllKeys";
            Keystore::deleteAllKeys();
        } else {
            LOG(INFO) << "Metadata key does not exist but "
                          "ro.crypto.metadata_init_delete_all_keys.enabled is false";
        }
    }
    auto temp = metadata_key_dir + "/tmp";
    return retrieveOrGenerateKey(dir, temp, kEmptyAuthentication, gen, key);
}

static bool get_number_of_sectors(const std::string& real_blkdev, uint64_t* nr_sec) {
    if (android::vold::GetBlockDev512Sectors(real_blkdev, nr_sec) != android::OK) {
        PLOG(ERROR) << "Unable to measure size of " << real_blkdev;
        return false;
    }
    return true;
}

static bool create_crypto_blk_dev(const std::string& dm_name, const std::string& blk_device,
                                  const KeyBuffer& key, const CryptoOptions& options,
                                  std::string* crypto_blkdev, uint64_t* nr_sec, bool is_userdata) {
    if (!get_number_of_sectors(blk_device, nr_sec)) return false;
    // TODO(paulcrowley): don't hardcode that DmTargetDefaultKey uses 4096-byte
    // sectors
    *nr_sec &= ~7;

    KeyBuffer module_key;
    if (options.use_hw_wrapped_key) {
        if (!exportWrappedStorageKey(key, &module_key)) {
            LOG(ERROR) << "Failed to get ephemeral wrapped key";
            return false;
        }
    } else {
        module_key = key;
    }

    KeyBuffer hex_key_buffer;
    if (android::vold::StrToHex(module_key, hex_key_buffer) != android::OK) {
        LOG(ERROR) << "Failed to turn key to hex";
        return false;
    }
    std::string hex_key(hex_key_buffer.data(), hex_key_buffer.size());

    auto target = std::make_unique<DmTargetDefaultKey>(0, *nr_sec, options.cipher.get_kernel_name(),
                                                       hex_key, blk_device, 0);
    if (options.use_legacy_options_format) target->SetUseLegacyOptionsFormat();
    if (options.set_dun) target->SetSetDun();
    if (options.use_hw_wrapped_key) target->SetWrappedKeyV0();

    DmTable table;
    table.AddTarget(std::move(target));

    auto& dm = DeviceMapper::Instance();
    if (dm_name == kDmNameUserdata && dm.GetState(dm_name) == DmDeviceState::SUSPENDED) {
        // The device was created in advance, populate it now.
        if (!dm.LoadTableAndActivate(dm_name, table)) {
            LOG(ERROR) << "Failed to populate default-key device " << dm_name;
            return false;
        }
        if (!dm.WaitForDevice(dm_name, 20s, crypto_blkdev)) {
            LOG(ERROR) << "Failed to wait for default-key device " << dm_name;
            return false;
        }
    } else if (!dm.CreateDevice(dm_name, table, crypto_blkdev, 5s)) {
        LOG(ERROR) << "Could not create default-key device " << dm_name;
        return false;
    }

    // If there are multiple partitions used for a single mount, F2FS stores
    // their partition paths in superblock. If the paths are dm targets, we
    // cannot guarantee them across device boots. Let's use the logical paths.
    if (is_userdata) {
        *crypto_blkdev = "/dev/block/mapper/" + dm_name;
    }
    return true;
}

static const CryptoType& lookup_cipher(const std::string& cipher_name) {
    if (cipher_name.empty()) return supported_crypto_types[0];
    for (size_t i = 0; i < array_length(supported_crypto_types); i++) {
        if (cipher_name == supported_crypto_types[i].get_config_name()) {
            return supported_crypto_types[i];
        }
    }
    return invalid_crypto_type;
}

static bool parse_options(const std::string& options_string, CryptoOptions* options) {
    auto parts = android::base::Split(options_string, ":");
    if (parts.size() < 1 || parts.size() > 2) {
        LOG(ERROR) << "Invalid metadata encryption option: " << options_string;
        return false;
    }
    std::string cipher_name = parts[0];
    options->cipher = lookup_cipher(cipher_name);
    if (options->cipher.get_kernel_name() == nullptr) {
        LOG(ERROR) << "No metadata cipher named " << cipher_name << " found";
        return false;
    }

    if (parts.size() == 2) {
        if (parts[1] == "wrappedkey_v0") {
            options->use_hw_wrapped_key = true;
        } else {
            LOG(ERROR) << "Invalid metadata encryption flag: " << parts[1];
            return false;
        }
    }
    return true;
}

class EncryptionInProgress {
  private:
    std::string file_path_;
    bool need_cleanup_ = false;

  public:
    EncryptionInProgress(const FstabEntry& entry) {
        file_path_ = fs_mgr_metadata_encryption_in_progress_file_name(entry);
    }

    [[nodiscard]] bool Mark() {
        {
            std::ofstream touch(file_path_);
            if (!touch.is_open()) {
                PLOG(ERROR) << "Failed to mark metadata encryption in progress " << file_path_;
                return false;
            }
            need_cleanup_ = true;
        }
        if (!android::vold::FsyncParentDirectory(file_path_)) return false;

        LOG(INFO) << "Marked metadata encryption in progress (" << file_path_ << ")";
        return true;
    }

    [[nodiscard]] bool Remove() {
        need_cleanup_ = false;
        if (unlink(file_path_.c_str()) != 0) {
            PLOG(ERROR) << "Failed to clear metadata encryption in progress (" << file_path_ << ")";
            return false;
        }
        if (!android::vold::FsyncParentDirectory(file_path_)) return false;

        LOG(INFO) << "Cleared metadata encryption in progress (" << file_path_ << ")";
        return true;
    }

    ~EncryptionInProgress() {
        if (need_cleanup_) (void)Remove();
    }
};

bool fscrypt_mount_metadata_encrypted(const std::string& blk_device, const std::string& mount_point,
                                      bool needs_encrypt, bool should_format,
                                      const std::string& fs_type, bool is_zoned,
                                      const std::vector<std::string>& user_devices,
                                      const std::vector<bool>& device_aliased, int64_t length, std::string fstab_path) {
    LOG(INFO) << "fscrypt_mount_metadata_encrypted: " << mount_point
               << " encrypt: " << needs_encrypt << " format: " << should_format << " with "
               << fs_type << " block device: " << blk_device << " with zoned " << is_zoned
               << " length: " << length;

    for (auto& device : user_devices) {
        LOG(DEBUG) << " - user devices: " << device;
    }

    auto encrypted_state = android::base::GetProperty("ro.crypto.state", "");
    if (encrypted_state != "" && encrypted_state != "encrypted") {
        LOG(INFO) << "fscrypt_mount_metadata_encrypted got unexpected starting state: "
                   << encrypted_state;
        return false;
    }
    if (!fstab_path.empty()) {
	printf("Using additional fstab for decryption %s \n", fstab_path.c_str());
        if (!ReadFstabFromFile(fstab_path, &fstab_default)) {
            PLOG(ERROR) << "Failed to open " << fstab_path << " Fstab ";
            return false;
        }
    } else {
        if (fstab_default.empty()) {
            if (!ReadDefaultFstab(&fstab_default)) {
                PLOG(ERROR) << "Failed to open default fstab";
                return false;
            }
        }
    }

    auto data_rec = GetEntryForMountPoint(&fstab_default, mount_point);
    if (!data_rec) {
        LOG(ERROR) << "Failed to get data_rec for " << mount_point;
        return false;
    }

    unsigned int options_format_version = 1;
    {
        EncryptionOptions options;
        if (!ParseOptions(data_rec->encryption_options, &options)) {
            LOG(ERROR) << "Unable to parse encryption options for " << DATA_MNT_POINT ": "
                       << data_rec->encryption_options;
        }
        options_format_version = options.version;
    }

    CryptoOptions options;
    if (options_format_version == 1) {
        if (!data_rec->metadata_encryption_options.empty()) {
            LOG(ERROR) << "metadata_encryption options cannot be set in legacy mode";
            return false;
        }
        options.cipher = legacy_aes_256_xts;
        options.use_legacy_options_format = true;
        if (is_metadata_wrapped_key_supported())
            options.use_hw_wrapped_key = true;
        options.set_dun = android::base::GetBoolProperty("ro.crypto.set_dun", false);
        if (!options.set_dun && data_rec->fs_mgr_flags.checkpoint_blk) {
            LOG(ERROR)
                    << "Block checkpoints and metadata encryption require ro.crypto.set_dun option";
            return false;
        }
    } else if (options_format_version == 2) {
        if (!parse_options(data_rec->metadata_encryption_options, &options)) return false;
    } else {
        LOG(ERROR) << "Unknown options_format_version: " << options_format_version;
        return false;
    }

    auto default_metadata_key_dir = data_rec->metadata_key_dir;
    if (!user_devices.empty()) {
        default_metadata_key_dir = default_metadata_key_dir + "/default";
    }
    auto gen = needs_encrypt ? makeGen(options) : neverGen();
    KeyBuffer key;
    if (!read_key(default_metadata_key_dir, gen, true, &key)) {
        LOG(ERROR) << "read_key failed in mountFstab";
        return false;
    }

    std::string crypto_blkdev;
    uint64_t nr_sec;
    if (!create_crypto_blk_dev(kDmNameUserdata, blk_device, key, options, &crypto_blkdev, &nr_sec,
                               true)) {
        LOG(ERROR) << "create_crypto_blk_dev failed in mountFstab";
        return false;
    }

    // create dm-default-key for user devices
    std::vector<std::string> crypto_user_blkdev;
    for (auto& device : user_devices) {
        std::string name = Basename(device);
        auto metadata_key_dir = data_rec->metadata_key_dir + "/" + name;

        if (!read_key(metadata_key_dir, gen, false, &key)) {
            LOG(ERROR) << "read_key failed with zoned device: " << device;
            return false;
        }
        std::string crypto_blkdev_arg;
        if (!create_crypto_blk_dev(name, device, key, options, &crypto_blkdev_arg, &nr_sec, true)) {
            LOG(ERROR) << "fscrypt_mount_metadata_encrypted: failed with device: " << device;
            return false;
        }
        crypto_user_blkdev.push_back(crypto_blkdev_arg.c_str());
    }

    if (needs_encrypt) {
        EncryptionInProgress marker(*data_rec);
        if (!marker.Mark()) return false;
        if (should_format) {
            status_t error;

            if (fs_type == "ext4") {
                error = ext4::Format(crypto_blkdev, 0, mount_point);
            } else if (fs_type == "f2fs") {
                error = f2fs::Format(crypto_blkdev, is_zoned, crypto_user_blkdev, device_aliased,
                                     length);
            } else {
                LOG(ERROR) << "Unknown filesystem type: " << fs_type;
                return false;
            }
            if (error != 0) {
                LOG(ERROR) << "Format of " << crypto_blkdev << " for " << mount_point
                           << " failed (err=" << error << ").";
                return false;
            }
            LOG(INFO) << "Format of " << crypto_blkdev << " for " << mount_point << " succeeded.";
        } else {
            if (!user_devices.empty()) {
                LOG(ERROR) << "encrypt_inplace cannot support zoned or userdata_exp device; should "
                              "format it.";
                return false;
            }
            if (!encrypt_inplace(crypto_blkdev, blk_device, nr_sec)) {
                LOG(ERROR) << "encrypt_inplace failed in mountFstab";
                return false;
            }
        }
        if (!marker.Remove()) return false;
    }

    LOG(INFO) << "Mounting metadata-encrypted filesystem:" << mount_point;
    mount_via_fs_mgr(mount_point.c_str(), crypto_blkdev.c_str(), needs_encrypt);
    android::base::SetProperty("ro.crypto.fs_crypto_blkdev", crypto_blkdev);

    // Record that there's at least one fstab entry with metadata encryption
    if (!android::base::SetProperty("ro.crypto.metadata.enabled", "true")) {
        LOG(WARNING) << "failed to set ro.crypto.metadata.enabled";  // This isn't fatal
    }
    return true;
}

static bool get_volume_options(CryptoOptions* options) {
    return parse_options(android::base::GetProperty("ro.crypto.volume.metadata.encryption", ""),
                         options);
}

bool defaultkey_volume_keygen(KeyGeneration* gen) {
    CryptoOptions options;
    if (!get_volume_options(&options)) return false;
    *gen = makeGen(options);
    return true;
}

bool defaultkey_setup_ext_volume(const std::string& label, const std::string& blk_device,
                                 const KeyBuffer& key, std::string* out_crypto_blkdev) {
    LOG(INFO) << "defaultkey_setup_ext_volume: " << label << " " << blk_device;

    CryptoOptions options;
    if (!get_volume_options(&options)) return false;
    uint64_t nr_sec;
    return create_crypto_blk_dev(label, blk_device, key, options, out_crypto_blkdev, &nr_sec,
                                 false);
}

bool destroy_dsu_metadata_key(const std::string& dsu_slot) {
    LOG(INFO) << "destroy_dsu_metadata_key: " << dsu_slot;

    const auto dsu_metadata_key_dir = android::gsi::GetDsuMetadataKeyDir(dsu_slot);
    if (!pathExists(dsu_metadata_key_dir)) {
        LOG(INFO) << "DSU metadata_key_dir doesn't exist, nothing to remove: "
                   << dsu_metadata_key_dir;
        return true;
    }

    // Ensure that the DSU key directory is different from the host OS'.
    // Under normal circumstances, this should never happen, but handle it just in case.
    if (fstab_default.empty()) {
        if (!ReadDefaultFstab(&fstab_default)) {
            PLOG(ERROR) << "Failed to open default fstab";
            return false;
        }
    }
    if (auto data_rec = GetEntryForMountPoint(&fstab_default, "/data")) {
        if (dsu_metadata_key_dir == data_rec->metadata_key_dir) {
            LOG(ERROR) << "DSU metadata_key_dir is same as host OS: " << dsu_metadata_key_dir;
            return false;
        }
    }

    bool ok = true;
    for (auto suffix : {"/key", "/tmp"}) {
        const auto key_path = dsu_metadata_key_dir + suffix;
        if (pathExists(key_path)) {
            LOG(INFO) << "Destroy key: " << key_path;
            if (!android::vold::destroyKey(key_path)) {
                LOG(ERROR) << "Failed to destroyKey(): " << key_path;
                ok = false;
            }
        }
    }
    if (!ok) {
        return false;
    }

    LOG(INFO) << "Remove DSU metadata_key_dir: " << dsu_metadata_key_dir;
    // DeleteDirContentsAndDir() already logged any error, so don't log repeatedly.
    return android::vold::DeleteDirContentsAndDir(dsu_metadata_key_dir) == android::OK;
}

}  // namespace vold
}  // namespace android
