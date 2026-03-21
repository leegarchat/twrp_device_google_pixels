from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Weaver available recovery (Android.bp)"
        self.target_file = "hardware/interfaces/weaver/aidl/Android.bp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
name: "android.hardware.weaver",
    vendor_available: true,
    srcs: ["android/hardware/weaver/*.aidl"],
    stability: "vintf",
""",
                # Блок 1: Модифицированный код (результат)
                r"""
    name: "android.hardware.weaver",
    vendor_available: true,
    recovery_available: true,
    srcs: ["android/hardware/weaver/*.aidl"],
    stability: "vintf",
"""
            )
        ]

