from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom graphics logic (graphics.cpp)"
        self.target_file = "bootable/recovery/minuitwrp/graphics.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
void gr_color(unsigned char r, unsigned char g, unsigned char b, unsigned char a)
{
    GGLContext *gl = gr_context;
    GGLint color[4];
#if defined(RECOVERY_ARGB) || defined(RECOVERY_BGRA) || defined(RECOVERY_ABGR)
    color[0] = ((b << 8) | r) + 1;
    color[1] = ((g << 8) | g) + 1;
    color[2] = ((r << 8) | b) + 1;
    color[3] = ((a << 8) | a) + 1;
#else
""",
                # Блок 1: Модифицированный код (результат)
                r"""
void gr_color(unsigned char r, unsigned char g, unsigned char b, unsigned char a)
{
    GGLContext *gl = gr_context;
    GGLint color[4];
#if defined(RECOVERY_ARGB) || defined(RECOVERY_BGRA)
    color[0] = ((b << 8) | r) + 1;
    color[1] = ((g << 8) | g) + 1;
    color[2] = ((r << 8) | b) + 1;
    color[3] = ((a << 8) | a) + 1;
#else
"""
            )
        ]

