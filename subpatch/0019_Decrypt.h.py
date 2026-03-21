from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom DE modifications (Decrypt.h)"
        self.target_file = "system/vold/Decrypt.h" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
namespace keystore {
    void copySqliteDb();
    int Get_Password_Type(const userid_t user_id, std::string& filename);
    bool Decrypt_DE();
    bool Decrypt_User(const userid_t user_id, const std::string& Password);
}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
namespace keystore {
    void copySqliteDb();
    int Get_Password_Type(const userid_t user_id, std::string& filename);
    bool Decrypt_DE();
    bool Decrypt_User(const userid_t user_id, const std::string& Password);
    bool Reset_FBE_Caches();
}
"""
            )
        ]

