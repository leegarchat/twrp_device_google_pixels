from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Libkeymaster_messages available recovery (Android.bp)"
        self.target_file = "system/keymaster/Android.bp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
cc_library_shared {
    name: "libkeymaster_messages",
    srcs: [
        "android_keymaster/android_keymaster_messages.cpp",
""",
                # Блок 1: Модифицированный код (результат)
                r"""
cc_library_shared {
    name: "libkeymaster_messages",
    recovery_available: true,
    srcs: [
        "android_keymaster/android_keymaster_messages.cpp",
"""
            )
        ]

