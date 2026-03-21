from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom keystore modifications (globals.rs)"
        self.target_file = "system/security/keystore2/src/globals.rs" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
fn connect_keymint(
    security_level: &SecurityLevel,
) -> Result<(Strong<dyn IKeyMintDevice>, KeyMintHardwareInfo)> {
    // Show the keymint interface that is registered in the binder
    // service and use the security level to get the service name.
    let service_name = keymint_service_name(security_level)
        .context(ks_err!("Get service name from binder service"))?;

    let (keymint, hal_version) = if let Some(service_name) = service_name {
        let km: Strong<dyn IKeyMintDevice> =
            map_binder_status_code(binder::get_interface(&service_name))
                .context(ks_err!("Trying to connect to genuine KeyMint service."))?;
        // Map the HAL version code for KeyMint to be <AIDL version> * 100, so
        // - V1 is 100
        // - V2 is 200
        // - V3 is 300
        // etc.
        let km_version = km.getInterfaceVersion()?;
        (km, Some(km_version * 100))
    } else {
        // This is a no-op if it was called before.
        keystore2_km_compat::add_keymint_device_service();

        let keystore_compat_service: Strong<dyn IKeystoreCompatService> =
            map_binder_status_code(binder::get_interface("android.security.compat"))
                .context(ks_err!("Trying to connect to compat service."))?;
        (
            map_binder_status(keystore_compat_service.getKeyMintDevice(*security_level))
                .map_err(|e| match e {
                    Error::BinderTransaction(StatusCode::NAME_NOT_FOUND) => {
                        Error::Km(ErrorCode::HARDWARE_TYPE_UNAVAILABLE)
                    }
                    e => e,
                })
                .context(ks_err!("Trying to get Legacy wrapper."))?,
            None,
        )
    };

    // If the KeyMint device is back-level, use a wrapper that intercepts and
    // emulates things that are not supported by the hardware.
    let keymint = match hal_version {
        Some(300) => {
            // Current KeyMint version: use as-is as v3 Keymint is current version
            log::info!(
                "KeyMint device is current version ({:?}) for security level: {:?}",
                hal_version,
                security_level
            );
            keymint
        }
""",
                # Блок 1: Модифицированный код (результат)
                r"""
fn connect_keymint(
    security_level: &SecurityLevel,
) -> Result<(Strong<dyn IKeyMintDevice>, KeyMintHardwareInfo)> {
    // Show the keymint interface that is registered in the binder
    // service and use the security level to get the service name.
    let service_name = keymint_service_name(security_level)
        .context(ks_err!("Get service name from binder service"))?;

    let (keymint, hal_version) = if let Some(service_name) = service_name {
        match map_binder_status_code(binder::get_interface::<dyn IKeyMintDevice>(&service_name)) {
            Ok(km) => {
                // Map the HAL version code for KeyMint to be <AIDL version> * 100, so
                // - V1 is 100
                // - V2 is 200
                // - V3 is 300
                // etc.
                let km_version = km.getInterfaceVersion()?;
                (km, Some(km_version * 100))
            }
            Err(e) => {
                log::warn!(
                    "Direct KeyMint connect to '{}' failed in recovery/runtime path: {:?}. Falling back to km_compat.",
                    service_name,
                    e
                );

                // This is a no-op if it was called before.
                keystore2_km_compat::add_keymint_device_service();

                let keystore_compat_service: Strong<dyn IKeystoreCompatService> =
                    map_binder_status_code(binder::get_interface("android.security.compat"))
                        .context(ks_err!("Trying to connect to compat service."))?;
                (
                    map_binder_status(keystore_compat_service.getKeyMintDevice(*security_level))
                        .map_err(|e| match e {
                            Error::BinderTransaction(StatusCode::NAME_NOT_FOUND) => {
                                Error::Km(ErrorCode::HARDWARE_TYPE_UNAVAILABLE)
                            }
                            e => e,
                        })
                        .context(ks_err!("Trying to get Legacy wrapper."))?,
                    None,
                )
            }
        }
    } else {
        // This is a no-op if it was called before.
        keystore2_km_compat::add_keymint_device_service();

        let keystore_compat_service: Strong<dyn IKeystoreCompatService> =
            map_binder_status_code(binder::get_interface("android.security.compat"))
                .context(ks_err!("Trying to connect to compat service."))?;
        (
            map_binder_status(keystore_compat_service.getKeyMintDevice(*security_level))
                .map_err(|e| match e {
                    Error::BinderTransaction(StatusCode::NAME_NOT_FOUND) => {
                        Error::Km(ErrorCode::HARDWARE_TYPE_UNAVAILABLE)
                    }
                    e => e,
                })
                .context(ks_err!("Trying to get Legacy wrapper."))?,
            None,
        )
    };

    // If the KeyMint device is back-level, use a wrapper that intercepts and
    // emulates things that are not supported by the hardware.
    let keymint = match hal_version {
        Some(v) if v >= 300 => {
            // Current-or-newer KeyMint version: use as-is.
            log::info!(
                "KeyMint device is current version ({:?}) for security level: {:?}",
                hal_version,
                security_level
            );
            keymint
        }
"""
            )
        ]

