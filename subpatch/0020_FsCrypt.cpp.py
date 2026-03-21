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
""",
                # Блок 1: Модифицированный код (результат)
                r"""
        if (volume_uuid.empty()) {
            if (!prepare_dir_with_policy(system_ce_path, 0770, AID_SYSTEM, AID_SYSTEM, ce_policy))
                return false;
            if (!prepare_dir_with_policy(vendor_ce_path, 0771, AID_ROOT, AID_ROOT, ce_policy))
                return false;
        }
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
            )
        ]

