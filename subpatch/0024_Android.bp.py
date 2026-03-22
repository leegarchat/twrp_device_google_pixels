from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Keymaster available recovery (Android.bp)"
        self.target_file = "system/core/trusty/keymaster/Android.bp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
vintf_fragments: [
        "keymint/android.hardware.security.keymint-service.trusty.xml",
    ],
    vendor: true,
    cflags: [
        "-Wall",
        "-Wextra",
    ],
""",
                # Блок 1: Модифицированный код (результат)
                r"""
    vintf_fragments: [
        "keymint/android.hardware.security.keymint-service.trusty.xml",
    ],
    vendor: true,
    recovery_available: true,
    cflags: [
        "-Wall",
        "-Wextra",
    ],
"""
            )
        ]

