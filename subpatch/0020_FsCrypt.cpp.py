from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom DE modifications (FsCrypt.cpp)"
        self.target_file = "system/vold/FsCrypt.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
bool fscrypt_lock_ce_storage(userid_t user_id) {
    LOG(DEBUG) << "fscrypt_lock_ce_storage " << user_id;
    if (!IsFbeEnabled()) return true;
    return evict_user_keys(s_ce_policies, user_id);
}

static bool prepare_subdirs(const std::string& action, const std::string& volume_uuid,
""",
                # Блок 1: Модифицированный код (результат)
                r"""
bool fscrypt_lock_ce_storage(userid_t user_id) {
    LOG(DEBUG) << "fscrypt_lock_ce_storage " << user_id;
    if (!IsFbeEnabled()) return true;
    return evict_user_keys(s_ce_policies, user_id);
}

bool fscrypt_reset_key_caches() {
    LOG(INFO) << "fscrypt_reset_key_caches: clearing stale key caches after /data unmount";
    s_ce_policies.clear();
    s_de_policies.clear();
    fscrypt_init_user0_done = false;
    return true;
}

static bool prepare_subdirs(const std::string& action, const std::string& volume_uuid,
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
        if (!prepare_dir_with_policy(media_ce_path, 02770, AID_MEDIA_RW, AID_MEDIA_RW, ce_policy))
            return false;
        // On devices without sdcardfs (kernel 5.4+), the path permissions aren't fixed
        // up automatically; therefore, use a default ACL, to ensure apps with MEDIA_RW
        // can keep reading external storage; in particular, this allows app cloning
        // scenarios to work correctly on such devices.
""",
                # Блок 1: Модифицированный код (результат)
                r"""
        if (!prepare_dir_with_policy(media_ce_path, 02770, AID_MEDIA_RW, AID_MEDIA_RW, ce_policy)) {
            // On hardware-wrapped-key devices (e.g., Pixel 8 with UFS ISE), /data/media uses
            // hardware-only inline encryption without per-file FBE policy applied on top.
            // This is non-fatal in recovery: the hardware CE key is already active and the
            // directory contents are accessible.
            PLOG(WARNING) << "fscrypt_prepare_user_storage: could not set media CE policy for "
                          << media_ce_path
                          << " (hardware inline encryption may be active), continuing";
        }
        // On devices without sdcardfs (kernel 5.4+), the path permissions aren't fixed
        // up automatically; therefore, use a default ACL, to ensure apps with MEDIA_RW
        // can keep reading external storage; in particular, this allows app cloning
        // scenarios to work correctly on such devices.
"""
            ),
            (
                # Блок 3: BindMount в prepare_special_dirs() — сделать non-fatal
                # BindMount нужен только для zygote app isolation, recovery этого не требует.
                # На реальных устройствах BindMount может провалиться (уже смонтировано,
                # или не поддерживается в контексте recovery).
                r"""
    if (!prepare_dir(data_user_0_dir, 0700, AID_SYSTEM, AID_SYSTEM)) return false;
    if (android::vold::BindMount(data_data_dir, data_user_0_dir) != 0) return false;

    // If /data/media/obb doesn't exist, create it and encrypt it with the
""",
                r"""
    if (!prepare_dir(data_user_0_dir, 0700, AID_SYSTEM, AID_SYSTEM)) return false;
    if (android::vold::BindMount(data_data_dir, data_user_0_dir) != 0) {
        PLOG(WARNING) << "BindMount " << data_data_dir << " -> " << data_user_0_dir
                      << " failed (non-fatal in recovery)";
    }

    // If /data/media/obb doesn't exist, create it and encrypt it with the
"""
            ),
            (
                # Блок 4: fscrypt_init_user0() — сделать prepare_special_dirs и
                # fscrypt_prepare_user_storage non-fatal.
                # DE-ключи УЖЕ загружены к моменту вызова этих функций (load_all_de_keys
                # отработала). Подготовка каталогов нужна фреймворку, не recovery.
                r"""
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
""",
                r"""
    // Now that user 0's CE key has been created, we can prepare /data/data.
    if (!prepare_special_dirs()) {
        // Non-fatal in recovery: directory structure and bind mounts are only
        // needed by the Android framework (zygote, package manager).
        // The DE keys are already loaded by load_all_de_keys() above.
        LOG(WARNING) << "prepare_special_dirs() failed (non-fatal in recovery)";
    }

    // With the exception of what is done by prepare_special_dirs() above, we
    // only prepare DE storage here, since user 0's CE key won't be installed
    // yet unless it was just created.  The framework will prepare the user's CE
    // storage later, once their CE key is installed.
    if (!fscrypt_prepare_user_storage("", 0, android::os::IVold::STORAGE_FLAG_DE)) {
        // Non-fatal in recovery: vold_prepare_subdirs and DE directory policy
        // setup may fail because the Android framework is not running.
        LOG(WARNING) << "Failed to prepare user 0 DE storage (non-fatal in recovery)";
    }
"""
            ),
            (
                # Блок 5: prepare_subdirs в fscrypt_prepare_user_storage() — non-fatal.
                # vold_prepare_subdirs может отсутствовать в recovery или провалиться
                # из-за отсутствия Android-фреймворка. Ключи уже установлены.
                r"""
    if (!prepare_subdirs("prepare", volume_uuid, user_id, flags)) return false;

    return true;
}
""",
                r"""
    if (!prepare_subdirs("prepare", volume_uuid, user_id, flags)) {
        LOG(WARNING) << "prepare_subdirs failed for user " << user_id
                     << " (non-fatal in recovery)";
    }

    return true;
}
"""
            ),
            (
                # Блок 6: system_ce и vendor_ce в fscrypt_prepare_user_storage() — non-fatal.
                # В recovery каталоги /data/system_ce/0 и /data/vendor_ce/0 уже имеют
                # fscrypt-политику от нормальной загрузки ROM. CE-ключ установлен,
                # но повторное назначение политики возвращает EEXIST.
                r"""
        if (volume_uuid.empty()) {
            if (!prepare_dir_with_policy(system_ce_path, 0770, AID_SYSTEM, AID_SYSTEM, ce_policy))
                return false;
            if (!prepare_dir_with_policy(vendor_ce_path, 0771, AID_ROOT, AID_ROOT, ce_policy))
                return false;
        }
""",
                r"""
        if (volume_uuid.empty()) {
            if (!prepare_dir_with_policy(system_ce_path, 0770, AID_SYSTEM, AID_SYSTEM, ce_policy)) {
                PLOG(WARNING) << "fscrypt_prepare_user_storage: could not set CE policy for "
                              << system_ce_path << " (non-fatal in recovery)";
            }
            if (!prepare_dir_with_policy(vendor_ce_path, 0771, AID_ROOT, AID_ROOT, ce_policy)) {
                PLOG(WARNING) << "fscrypt_prepare_user_storage: could not set CE policy for "
                              << vendor_ce_path << " (non-fatal in recovery)";
            }
        }
"""
            ),
            (
                # Блок 7: SetDefaultAcl, misc_ce, user_ce — non-fatal.
                # Аналогично: каталоги уже существуют и зашифрованы ROM-ом.
                # SetDefaultAcl может провалиться из-за отсутствия xattr support
                # в recovery-контексте.
                r"""
        int ret = SetDefaultAcl(media_ce_path, 02770, AID_MEDIA_RW, AID_MEDIA_RW, {AID_MEDIA_RW});
        if (ret != android::OK) {
            return false;
        }
        if (!prepare_dir_with_policy(misc_ce_path, 01771, AID_SYSTEM, AID_MISC, ce_policy))
            return false;
        if (!prepare_dir_with_policy(user_ce_path, 0771, AID_SYSTEM, AID_SYSTEM, ce_policy))
            return false;
""",
                r"""
        int ret = SetDefaultAcl(media_ce_path, 02770, AID_MEDIA_RW, AID_MEDIA_RW, {AID_MEDIA_RW});
        if (ret != android::OK) {
            LOG(WARNING) << "SetDefaultAcl for " << media_ce_path
                         << " failed (non-fatal in recovery)";
        }
        if (!prepare_dir_with_policy(misc_ce_path, 01771, AID_SYSTEM, AID_MISC, ce_policy)) {
            PLOG(WARNING) << "fscrypt_prepare_user_storage: could not set CE policy for "
                          << misc_ce_path << " (non-fatal in recovery)";
        }
        if (!prepare_dir_with_policy(user_ce_path, 0771, AID_SYSTEM, AID_SYSTEM, ce_policy)) {
            PLOG(WARNING) << "fscrypt_prepare_user_storage: could not set CE policy for "
                          << user_ce_path << " (non-fatal in recovery)";
        }
"""
            )
        ]

