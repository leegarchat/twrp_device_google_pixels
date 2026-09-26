/*
 * Copyright (C) 2007 The Android Open Source Project
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

 * Copyright (C) 2026 The OrangeFox Recovery Project
 *
 */

#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include <fcntl.h>
#include <stdio.h>

#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/types.h>

#include <linux/fb.h>
#include <linux/kd.h>

#include <time.h>

#include <cutils/properties.h>
#include <stdint.h>
#include <pixelflinger/pixelflinger.h>
#include "gui/placement.h"
#include "minuitwrp/minui.h"
#include "graphics.h"
// For std::min and std::max
#include <algorithm>
#include "minuitwrp/truetype.hpp"

struct GRFont {
    GRSurface* texture;
    int cwidth;
    int cheight;
};

static minui_backend* gr_backend = NULL;

static int overscan_percent = OVERSCAN_PERCENT;
static int overscan_offset_x = 0;
static int overscan_offset_y = 0;

static unsigned char gr_current_r = 255;
static unsigned char gr_current_g = 255;
static unsigned char gr_current_b = 255;

// Fox AIO: runtime R/B channel swap selector. Display/GPU generations
// disagree about byte order (Mali families gs101/gs201/zuma/zumapro render
// correctly with the patched no-swap path, PowerVR malibu/laguna need the
// upstream swap), and AIO ships one binary — so the family decides at
// runtime via ro.recovery.rb_swap (family.json props, stamped by
// recovery-pixel-boot before the UI starts). Absent prop = no swap,
// i.e. today's behavior. Cached: properties are process-stable here.
static int g_fox_rb_swap = -1;
static bool fox_rb_swap(void) {
    if (g_fox_rb_swap < 0) {
        char v[PROPERTY_VALUE_MAX];
        g_fox_rb_swap = (property_get("ro.recovery.rb_swap", v, "") > 0 && v[0] == '1') ? 1 : 0;
        printf("fox_rb_swap: ro.recovery.rb_swap=%s -> %s\n", v[0] ? v : "(absent)",
               g_fox_rb_swap ? "SWAP (PowerVR path)" : "no-swap (Mali path)");
    }
    return g_fox_rb_swap == 1;
}

GRSurface* gr_draw = NULL;

static GGLContext *gr_context = 0;
GGLSurface gr_mem_surface;
static int gr_is_curr_clr_opaque = 0;

unsigned int gr_rotation = 0;

int gr_textEx_scaleW(int x, int y, const char *s, void* pFont, int max_width, int placement, int scale)
{
    GGLContext *gl = gr_context;
    void* vfont = pFont;
    GRFont *font = (GRFont*) pFont;
    int y_scale = 0, measured_width, measured_height, new_height;

    if (!s || strlen(s) == 0 || !font)
        return 0;

    measured_height = twrpTruetype::gr_ttf_getMaxFontHeight(font);

    if (scale) {
        measured_width = twrpTruetype::gr_ttf_measureEx(s, vfont);
        if (measured_width > max_width) {
            // Adjust font size down until the text fits
            void *new_font = twrpTruetype::gr_ttf_scaleFont(vfont, max_width, measured_width);
            if (!new_font) {
                printf("gr_textEx_scaleW new_font is NULL\n");
                return 0;
            }
            measured_width = twrpTruetype::gr_ttf_measureEx(s, new_font);
            // These next 2 lines adjust the y point based on the new font's height
            new_height = twrpTruetype::gr_ttf_getMaxFontHeight(new_font);
            y_scale = (measured_height - new_height) / 2;
            vfont = new_font;
        }
    } else
        measured_width = twrpTruetype::gr_ttf_measureEx(s, vfont);

    int x_adj = measured_width;
    if (measured_width > max_width)
        x_adj = max_width;

    if (placement != TOP_LEFT && placement != BOTTOM_LEFT && placement != TEXT_ONLY_RIGHT) {
        if (placement == CENTER || placement == CENTER_X_ONLY)
            x -= (x_adj / 2);
        else
            x -= x_adj;
    }

    if (placement != TOP_LEFT && placement != TOP_RIGHT) {
        if (placement == CENTER || placement == TEXT_ONLY_RIGHT)
            y -= (measured_height / 2);
        else if (placement == BOTTOM_LEFT || placement == BOTTOM_RIGHT)
            y -= measured_height;
    }
    return twrpTruetype::gr_ttf_textExWH(gl, x, y + y_scale, s, vfont, measured_width + x, -1, gr_draw);
}

/* Fox fps_boost: CPU-side clip mirror + direct 32bpp raster fast paths.
 * Every gr_blit/gr_fill currently pays a full pixelflinger round-trip
 * (bindTexture + texEnvi/texGeni + recti + state toggles per call) plus a
 * generic software rasterizer. With ~150 blits and ~10MB of flat fills per
 * file-list frame at 120Hz (8.3ms budget), that overhead is the 3-4ms we
 * are over. Fast paths engage only when bit-identical to the slow path:
 *  - gr_rotation == 0 (rotation is boot-fixed, never changes at runtime);
 *  - 32bpp, source and dest share the same 8888 layout (positional copy,
 *    no channel lore; alpha is always byte 3 of the word on LE);
 *  - rects clamped to both surfaces (no OOB reads/writes) and intersected
 *    with a CPU mirror of gr_clip/gr_noclip (kept 1:1 below);
 *  - fills: the packed 4 bytes pixelflinger writes for a logical (r,g,b)
 *    are learned once per color by reading back one pixel after a
 *    slow-path fill — exact on every family/format/swap combo by
 *    construction (theme colors are bounded; overflow stays slow).
 * Blend is src-over straight alpha; +/-1 LSB vs GGL rounding on
 * antialiased edges is invisible (recovery UI has no gradients). The dst
 * alpha byte is preserved on partial blends (scanout ignores it).
 * Anything else falls through to pixelflinger unchanged.
 */
static bool fox_clip_on = false;
static int fox_clip_x = 0, fox_clip_y = 0, fox_clip_w = 0, fox_clip_h = 0;
static unsigned long long fox_blt_n = 0, fox_blt_px = 0, fox_blt_us = 0;

struct FoxFillEntry { uint8_t r, g, b; uint32_t px; bool used; };
static FoxFillEntry fox_fill_cache[32];

static bool fox_fill_lookup(uint8_t r, uint8_t g, uint8_t b, uint32_t *px) {
    for (int i = 0; i < 32; i++) {
        if (fox_fill_cache[i].used && fox_fill_cache[i].r == r &&
            fox_fill_cache[i].g == g && fox_fill_cache[i].b == b) {
            *px = fox_fill_cache[i].px;
            return true;
        }
    }
    return false;
}

static void fox_fill_learn(uint8_t r, uint8_t g, uint8_t b, uint32_t px) {
    for (int i = 0; i < 32; i++) {
        if (!fox_fill_cache[i].used) {
            fox_fill_cache[i].r = r;
            fox_fill_cache[i].g = g;
            fox_fill_cache[i].b = b;
            fox_fill_cache[i].px = px;
            fox_fill_cache[i].used = true;
            return;
        }
    }
}

/* Intersect (x,y,w,h) with the draw surface and the active clip mirror
 * (rotation-0 logical coords). False = nothing would be drawn. */
static bool fox_clip_rect(int *x, int *y, int *w, int *h) {
    if (!gr_draw || !gr_draw->data || *w <= 0 || *h <= 0)
        return false;
    if (*x < 0) { *w += *x; *x = 0; }
    if (*y < 0) { *h += *y; *y = 0; }
    if (*x + *w > gr_draw->width) *w = gr_draw->width - *x;
    if (*y + *h > gr_draw->height) *h = gr_draw->height - *y;
    if (fox_clip_on) {
        int x2 = *x + *w, y2 = *y + *h;
        int cx2 = fox_clip_x + fox_clip_w, cy2 = fox_clip_y + fox_clip_h;
        if (*x < fox_clip_x) *x = fox_clip_x;
        if (*y < fox_clip_y) *y = fox_clip_y;
        if (x2 > cx2) x2 = cx2;
        if (y2 > cy2) y2 = cy2;
        *w = x2 - *x; *h = y2 - *y;
    }
    return *w > 0 && *h > 0;
}

/* Blit census accounting shared by the slow and fast paths (temporary,
 * remove before merge). */
static void fox_blit_account(const struct timespec *b0, const struct timespec *b1,
                             unsigned long long px) {
    long long fox_dns = (long long)(b1->tv_sec - b0->tv_sec) * 1000000000LL +
        (long long)(b1->tv_nsec - b0->tv_nsec);
    if (fox_dns < 0) fox_dns = 0;
    fox_blt_n++;
    fox_blt_px += px;
    fox_blt_us += (unsigned long long)fox_dns / 1000ULL;
    if (fox_blt_n >= 20000) {
        printf("foxblit: %llu blits, %llu Mpix, %llu ms total (avg %llu us/blt)\n",
            fox_blt_n, fox_blt_px / 1000000ULL, fox_blt_us / 1000ULL,
            fox_blt_us / (fox_blt_n ? fox_blt_n : 1));
        fox_blt_n = fox_blt_px = fox_blt_us = 0;
    }
}

/* Direct 32bpp blit. True = performed (bytes identical to the pixelflinger
 * path: positional copy within one shared 8888 layout). */
static bool fox_fast_blit(gr_surface src0, int sx, int sy, int w, int h, int dx, int dy) {
    if (gr_rotation != 0 || !gr_draw || !gr_draw->data || gr_draw->pixel_bytes != 4)
        return false;
    if (gr_draw->row_bytes < gr_draw->width * 4)
        return false;
    GGLSurface *surface = (GGLSurface *)src0;
    if (!surface || !surface->data || surface->width <= 0 || surface->height <= 0)
        return false;
    /* Either side may be RGBA or RGBX: bytes 0-2 are R,G,B on both, byte 3
     * is alpha-or-don't-care (an RGBX source onto an RGBA dest still copies
     * positionally exactly like the slow path; the dest X/alpha byte is
     * never scanned out). Anything else (BGRA, 565, ...) stays slow. */
    if ((surface->format != GGL_PIXEL_FORMAT_RGBA_8888 &&
         surface->format != GGL_PIXEL_FORMAT_RGBX_8888) ||
        (gr_draw->format != GGL_PIXEL_FORMAT_RGBA_8888 &&
         gr_draw->format != GGL_PIXEL_FORMAT_RGBX_8888))
        return false;
    if (surface->stride < surface->width)
        return false;
    if (w <= 0 || h <= 0)
        return true;
    if (sx < 0) { w += sx; dx -= sx; sx = 0; }
    if (sy < 0) { h += sy; dy -= sy; sy = 0; }
    if (sx + w > surface->width) w = surface->width - sx;
    if (sy + h > surface->height) h = surface->height - sy;
    if (w <= 0 || h <= 0)
        return true;
    if (dx < 0) { w += dx; sx -= dx; dx = 0; }
    if (dy < 0) { h += dy; sy -= dy; dy = 0; }
    if (dx + w > gr_draw->width) w = gr_draw->width - dx;
    if (dy + h > gr_draw->height) h = gr_draw->height - dy;
    if (w <= 0 || h <= 0)
        return true;
    if (fox_clip_on) {
        int ox = dx, oy = dy;
        int x2 = dx + w, y2 = dy + h;
        int cx2 = fox_clip_x + fox_clip_w, cy2 = fox_clip_y + fox_clip_h;
        if (dx < fox_clip_x) dx = fox_clip_x;
        if (dy < fox_clip_y) dy = fox_clip_y;
        if (x2 > cx2) x2 = cx2;
        if (y2 > cy2) y2 = cy2;
        if (x2 <= dx || y2 <= dy)
            return true;
        sx += dx - ox; sy += dy - oy;
        w = x2 - dx; h = y2 - dy;
    }
    const int src_rb = surface->stride * 4;
    const unsigned char *srow =
        surface->data + (size_t)sy * (size_t)src_rb + (size_t)sx * 4;
    unsigned char *drow =
        gr_draw->data + (size_t)dy * (size_t)gr_draw->row_bytes + (size_t)dx * 4;
    if (surface->format == GGL_PIXEL_FORMAT_RGBX_8888) {
        for (int i = 0; i < h; i++, drow += gr_draw->row_bytes, srow += src_rb)
            memcpy(drow, srow, (size_t)w * 4);
        return true;
    }
    for (int i = 0; i < h; i++, drow += gr_draw->row_bytes, srow += src_rb) {
        const uint32_t *s = (const uint32_t *)srow;
        uint32_t *d = (uint32_t *)drow;
        for (int j = 0; j < w; j++) {
            uint32_t sp = s[j];
            unsigned a = sp >> 24;
            if (a == 255) {
                d[j] = sp;
            } else if (a) {
                uint32_t dp = d[j];
                unsigned inv = 255 - a;
                unsigned r = (unsigned)(sp & 0xff) * a + (unsigned)(dp & 0xff) * inv;
                unsigned g = (unsigned)((sp >> 8) & 0xff) * a + (unsigned)((dp >> 8) & 0xff) * inv;
                unsigned b = (unsigned)((sp >> 16) & 0xff) * a + (unsigned)((dp >> 16) & 0xff) * inv;
                d[j] = (dp & 0xff000000u) |
                       ((((b + 127) / 255) & 0xff) << 16) |
                       ((((g + 127) / 255) & 0xff) << 8) |
                       (((r + 127) / 255) & 0xff);
            }
        }
    }
    return true;
}

void gr_clip(int x, int y, int w, int h)
{
    GGLContext *gl = gr_context;

    switch (gr_rotation) {
        case 90:
            gl->scissor(gl, gr_draw->width - y - h, x, h, w);
            break;
        case 180:
            gl->scissor(gl, gr_draw->width - x - w, gr_draw->height - y - h, w, h);
            break;
        case 270:
            gl->scissor(gl, y, gr_draw->height - x - w, h, w);
            break;
        default:
            gl->scissor(gl, x, y, w, h);
            break;
    }
    gl->enable(gl, GGL_SCISSOR_TEST);
    /* Fox fps_boost: mirror the clip for the direct raster fast paths
     * (rotation-0 logical coords; the mirror is only consulted then). */
    fox_clip_on = true;
    fox_clip_x = x;
    fox_clip_y = y;
    fox_clip_w = w;
    fox_clip_h = h;
}

void gr_noclip()
{
    GGLContext *gl = gr_context;
    gl->scissor(gl, 0, 0,
                gr_draw->width - 2 * overscan_offset_x,
                gr_draw->height - 2 * overscan_offset_y);
    gl->disable(gl, GGL_SCISSOR_TEST);
    /* Fox fps_boost: test disabled = no clipping, mirror follows. */
    fox_clip_on = false;
}

void gr_line(int x0, int y0, int x1, int y1, int width)
{
    GGLContext *gl = gr_context;
    int x0_disp, y0_disp, x1_disp, y1_disp;

    x0_disp = ROTATION_X_DISP(x0, y0, gr_draw->width);
    y0_disp = ROTATION_Y_DISP(x0, y0, gr_draw->height);
    x1_disp = ROTATION_X_DISP(x1, y1, gr_draw->width);
    y1_disp = ROTATION_Y_DISP(x1, y1, gr_draw->height);

    if(gr_is_curr_clr_opaque)
        gl->disable(gl, GGL_BLEND);

    const int coords0[2] = { x0_disp << 4, y0_disp << 4 };
    const int coords1[2] = { x1_disp << 4, y1_disp << 4 };
    gl->linex(gl, coords0, coords1, width << 4);

    if(gr_is_curr_clr_opaque)
        gl->enable(gl, GGL_BLEND);
}

gr_surface gr_render_circle(int radius, unsigned char r, unsigned char g, unsigned char b, unsigned char a)
{
    int rx, ry;
    GGLSurface *surface;
    const int diameter = radius*2 + 1;
    const int radius_check = radius*radius + radius*0.8;
    // Fox AIO: packed pixel must agree with gr_color's runtime swap —
    // the stock packing is ABGR-order (b in bits 16-23); swapped families
    // need RGBA-order instead.
    const uint32_t px = fox_rb_swap() ? (uint32_t)((a << 24) | (r << 16) | (g << 8) | b)
                                      : (uint32_t)((a << 24) | (b << 16) | (g << 8) | r);
    uint32_t *data;

    surface = (GGLSurface *)malloc(sizeof(GGLSurface));
    memset(surface, 0, sizeof(GGLSurface));

    data = (uint32_t *)malloc(diameter * diameter * 4);
    memset(data, 0, diameter * diameter * 4);

    surface->version = sizeof(surface);
    surface->width = diameter;
    surface->height = diameter;
    surface->stride = diameter;
    surface->data = (GGLubyte*)data;
#if defined(RECOVERY_BGRA)
    surface->format = GGL_PIXEL_FORMAT_BGRA_8888;
#else
    surface->format = GGL_PIXEL_FORMAT_RGBA_8888;
#endif

    for(ry = -radius; ry <= radius; ++ry)
        for(rx = -radius; rx <= radius; ++rx)
            if(rx*rx+ry*ry <= radius_check)
                *(data + diameter*(radius + ry) + (radius+rx)) = px;

    return (gr_surface)surface;
}

void gr_color(unsigned char r, unsigned char g, unsigned char b, unsigned char a)
{
    GGLContext *gl = gr_context;
    GGLint color[4];
    // Fox AIO runtime R/B-swap override (ro.recovery.rb_swap, stamped per
    // family from family.json by recovery-pixel-boot before the UI starts:
    // "1" = upstream swap for malibu/laguna, "0" = patched no-swap for
    // gs101/gs201/zuma/zumapro). Handled here and returned early; the
    // compile-time branches below stay untouched as the absent-prop
    // fallback (default = no swap, i.e. current behavior for 6-9).
    if (fox_rb_swap()) {
        color[0] = ((b << 8) | r) + 1;
        color[1] = ((g << 8) | g) + 1;
        color[2] = ((r << 8) | b) + 1;
        color[3] = ((a << 8) | a) + 1;
        gl->color4xv(gl, color);

        gr_is_curr_clr_opaque = (a == 255);
        return;
    }
#if defined(RECOVERY_ARGB) || defined(RECOVERY_BGRA)
    color[0] = ((b << 8) | r) + 1;
    color[1] = ((g << 8) | g) + 1;
    color[2] = ((r << 8) | b) + 1;
    color[3] = ((a << 8) | a) + 1;
#else
    color[0] = ((r << 8) | r) + 1;
    color[1] = ((g << 8) | g) + 1;
    color[2] = ((b << 8) | b) + 1;
    color[3] = ((a << 8) | a) + 1;
#endif
    gl->color4xv(gl, color);

    gr_is_curr_clr_opaque = (a == 255);
}

void gr_clear()
{
    if (gr_draw->pixel_bytes == 2) {
        gr_fill(0, 0, gr_fb_width(), gr_fb_height());
        return;
    }

    // This code only works on 32bpp devices
    if (gr_current_r == gr_current_g && gr_current_r == gr_current_b) {
        memset(gr_draw->data, gr_current_r, gr_draw->height * gr_draw->row_bytes);
    } else {
        unsigned char* px = gr_draw->data;
        for (int y = 0; y < gr_draw->height; ++y) {
            for (int x = 0; x < gr_draw->width; ++x) {
                *px++ = gr_current_r;
                *px++ = gr_current_g;
                *px++ = gr_current_b;
                px++;
            }
            px += gr_draw->row_bytes - (gr_draw->width * gr_draw->pixel_bytes);
        }
    }
}

void gr_fill(int x, int y, int w, int h)
{
    /* Fox fps_boost fast path: opaque flat fills are the bulk of every
     * frame (page + list + separators + header) and pixelflinger recti is
     * a generic rasterizer for them. New colors take the slow path once
     * and are learned by read-back (see FoxFillEntry). */
    bool fox_learn = false;
    int fox_lx = 0, fox_ly = 0;
    if (gr_rotation == 0 && gr_draw && gr_draw->data && gr_draw->pixel_bytes == 4 &&
        gr_is_curr_clr_opaque) {
        int fx = x, fy = y, fw = w, fh = h;
        if (fox_clip_rect(&fx, &fy, &fw, &fh)) {
            uint32_t px;
            if (fox_fill_lookup(gr_current_r, gr_current_g, gr_current_b, &px)) {
                unsigned char *row = gr_draw->data +
                    (size_t)fy * (size_t)gr_draw->row_bytes + (size_t)fx * 4;
                for (int i = 0; i < fh; i++, row += gr_draw->row_bytes) {
                    uint32_t *d = (uint32_t *)row;
                    for (int j = 0; j < fw; j++)
                        d[j] = px;
                }
                return;
            }
            fox_learn = true;
            fox_lx = fx;
            fox_ly = fy;
        } else {
            return;  // empty after surface/clip clamp: slow path draws nothing
        }
    }
    GGLContext *gl = gr_context;
    int x0_disp, y0_disp, x1_disp, y1_disp;
    int l_disp, r_disp, t_disp, b_disp;

    if(gr_is_curr_clr_opaque)
        gl->disable(gl, GGL_BLEND);

    x0_disp = ROTATION_X_DISP(x, y, gr_draw->width);
    y0_disp = ROTATION_Y_DISP(x, y, gr_draw->height);
    x1_disp = ROTATION_X_DISP(x + w, y + h, gr_draw->width);
    y1_disp = ROTATION_Y_DISP(x + w, y + h, gr_draw->height);
    l_disp = std::min(x0_disp, x1_disp);
    r_disp = std::max(x0_disp, x1_disp);
    t_disp = std::min(y0_disp, y1_disp);
    b_disp = std::max(y0_disp, y1_disp);
    gl->recti(gl, l_disp, t_disp, r_disp, b_disp);

    if(gr_is_curr_clr_opaque)
        gl->enable(gl, GGL_BLEND);

    if (fox_learn) {
        uint32_t px = *(uint32_t *)(gr_draw->data +
            (size_t)fox_ly * (size_t)gr_draw->row_bytes + (size_t)fox_lx * 4);
        fox_fill_learn(gr_current_r, gr_current_g, gr_current_b, px);
    }
}

void gr_blit(gr_surface source, int sx, int sy, int w, int h, int dx, int dy)
{
    if (gr_context == NULL) {
        return;
    }

    // Fox fps_boost: blit census (count + pixels + ms per 20k-blit
    // window, slow and fast paths alike). Temporary, remove before merge.
    struct timespec fox_b0, fox_b1;
    clock_gettime(CLOCK_MONOTONIC, &fox_b0);

    if (fox_fast_blit(source, sx, sy, w, h, dx, dy)) {
        clock_gettime(CLOCK_MONOTONIC, &fox_b1);
        fox_blit_account(&fox_b0, &fox_b1,
            (unsigned long long)(w > 0 ? w : 0) * (unsigned long long)(h > 0 ? h : 0));
        return;
    }

    GGLContext *gl = gr_context;
    GGLSurface *surface = (GGLSurface*)source;

    if(surface->format == GGL_PIXEL_FORMAT_RGBX_8888)
        gl->disable(gl, GGL_BLEND);

    int dx0_disp, dy0_disp, dx1_disp, dy1_disp;
    int l_disp, r_disp, t_disp, b_disp;

    // Figuring out display coordinates works for gr_rotation == 0 too,
    // and isn't as expensive as allocating and rotating another surface,
    // so we do this anyway.
    dx0_disp = ROTATION_X_DISP(dx, dy, gr_draw->width);
    dy0_disp = ROTATION_Y_DISP(dx, dy, gr_draw->height);
    dx1_disp = ROTATION_X_DISP(dx + w, dy + h, gr_draw->width);
    dy1_disp = ROTATION_Y_DISP(dx + w, dy + h, gr_draw->height);
    l_disp = std::min(dx0_disp, dx1_disp);
    r_disp = std::max(dx0_disp, dx1_disp);
    t_disp = std::min(dy0_disp, dy1_disp);
    b_disp = std::max(dy0_disp, dy1_disp);

    GGLSurface surface_rotated;
    if (gr_rotation != 0) {
        // Do not perform relatively expensive operation if not needed
        surface_rotated.version = sizeof(surface_rotated);
        // Skip the **(gr_rotation == 0)** || (gr_rotation == 180) check
        // because we are under a gr_rotation != 0 conditional compilation statement
        surface_rotated.width   = (gr_rotation == 180) ? surface->width  : surface->height;
        surface_rotated.height  = (gr_rotation == 180) ? surface->height : surface->width;
        surface_rotated.stride  = surface_rotated.width;
        surface_rotated.format  = surface->format;
        surface_rotated.data    = (GGLubyte*) malloc(surface_rotated.stride * surface_rotated.height * 4);
        surface_ROTATION_transform((gr_surface) &surface_rotated, (const gr_surface) surface, 4);

        gl->bindTexture(gl, &surface_rotated);
    } else {
        gl->bindTexture(gl, surface);
    }

    gl->texEnvi(gl, GGL_TEXTURE_ENV, GGL_TEXTURE_ENV_MODE, GGL_REPLACE);
    gl->texGeni(gl, GGL_S, GGL_TEXTURE_GEN_MODE, GGL_ONE_TO_ONE);
    gl->texGeni(gl, GGL_T, GGL_TEXTURE_GEN_MODE, GGL_ONE_TO_ONE);
    gl->enable(gl, GGL_TEXTURE_2D);
    gl->texCoord2i(gl, sx - l_disp, sy - t_disp);
    gl->recti(gl, l_disp, t_disp, r_disp, b_disp);
    gl->disable(gl, GGL_TEXTURE_2D);

    if (gr_rotation != 0)
        free(surface_rotated.data);

    if(surface->format == GGL_PIXEL_FORMAT_RGBX_8888)
        gl->enable(gl, GGL_BLEND);

    clock_gettime(CLOCK_MONOTONIC, &fox_b1);
    fox_blit_account(&fox_b0, &fox_b1,
        (unsigned long long)(r_disp - l_disp) * (unsigned long long)(b_disp - t_disp));
}

unsigned int gr_get_width(gr_surface surface) {
    if (surface == NULL) {
        return 0;
    }
    return ((GGLSurface*) surface)->width;
}

unsigned int gr_get_height(gr_surface surface) {
    if (surface == NULL) {
        return 0;
    }
    return ((GGLSurface*) surface)->height;
}

void gr_flip() {
    gr_draw = gr_backend->flip(gr_backend);
    // On double buffered back ends, when we flip, we need to tell
    // pixel flinger to draw to the other buffer
    gr_mem_surface.data = (GGLubyte*)gr_draw->data;
    gr_context->colorBuffer(gr_context, &gr_mem_surface);
}

static void get_memory_surface(GGLSurface* ms) {
    ms->version = sizeof(*ms);
    ms->width = gr_draw->width;
    ms->height = gr_draw->height;
    ms->stride = gr_draw->row_bytes / gr_draw->pixel_bytes;
    ms->data = (GGLubyte*)gr_draw->data;
    ms->format = gr_draw->format;
}

int gr_init(void)
{
    gr_draw = NULL;

    char gr_rotation_string[PROPERTY_VALUE_MAX];
    char default_rotation[4];
#ifdef OF_LANDSCAPE_MODE
    snprintf(default_rotation, 4, "%d", 270);
#else
    snprintf(default_rotation, 4, "%d", TW_ROTATION);
#endif
    property_get("persist.twrp.rotation", gr_rotation_string, default_rotation);
    gr_rotation = atoi(gr_rotation_string);
    if (!(gr_rotation == 90 || gr_rotation == 180 || gr_rotation == 270))
        gr_rotation = 0;

#ifdef MSM_BSP
    gr_backend = open_overlay();
    if (gr_backend) {
        gr_draw = gr_backend->init(gr_backend);
        if (!gr_draw) {
            gr_backend->exit(gr_backend);
        } else
            printf("Using overlay graphics.\n");
    }
#endif

#ifdef HAS_DRM
    if (!gr_backend || !gr_draw) {
        gr_backend = open_drm();
        gr_draw = gr_backend->init(gr_backend);
        if (gr_draw)
            printf("Using drm graphics.\n");
    }
#else
    printf("Skipping drm graphics -- not present in build tree\n");
#endif

    if (!gr_backend || !gr_draw) {
        gr_backend = open_fbdev();
        gr_draw = gr_backend->init(gr_backend);
        if (gr_draw == NULL) {
            return -1;
        } else
            printf("Using fbdev graphics.\n");
    }

    overscan_offset_x = gr_draw->width * overscan_percent / 100;
    overscan_offset_y = gr_draw->height * overscan_percent / 100;

    // Set up pixelflinger
    get_memory_surface(&gr_mem_surface);
    gglInit(&gr_context);
    GGLContext *gl = gr_context;
    gl->colorBuffer(gl, &gr_mem_surface);

    gl->activeTexture(gl, 0);
    gl->enable(gl, GGL_BLEND);
    gl->blendFunc(gl, GGL_SRC_ALPHA, GGL_ONE_MINUS_SRC_ALPHA);

    gr_flip();
    gr_flip();

    return 0;
}

void gr_exit(void)
{
    gr_backend->exit(gr_backend);
}

int gr_fb_width(void)
{
    return (gr_rotation == 0 || gr_rotation == 180) ?
            gr_draw->width  - 2 * overscan_offset_x :
            gr_draw->height - 2 * overscan_offset_y;
}

int gr_fb_height(void)
{
    return (gr_rotation == 0 || gr_rotation == 180) ?
            gr_draw->height - 2 * overscan_offset_y :
            gr_draw->width  - 2 * overscan_offset_x;
}

void gr_fb_blank(bool blank)
{
    gr_backend->blank(gr_backend, blank);
}

int gr_get_surface(gr_surface* surface)
{
    GGLSurface* ms = (GGLSurface*)malloc(sizeof(GGLSurface));
    if (!ms)    return -1;

    // Allocate the data
    get_memory_surface(ms);
    ms->data = (GGLubyte*)malloc(ms->stride * ms->height * gr_draw->pixel_bytes);

    // Now, copy the data
    memcpy(ms->data, gr_mem_surface.data, gr_draw->width * gr_draw->height * gr_draw->pixel_bytes / 8);

    *surface = (gr_surface*) ms;
    return 0;
}

int gr_free_surface(gr_surface surface)
{
    if (!surface)
        return -1;

    GGLSurface* ms = (GGLSurface*) surface;
    free(ms->data);
    free(ms);
    return 0;
}

void gr_write_frame_to_file(int fd)
{
    write(fd, gr_mem_surface.data, gr_draw->width * gr_draw->height * gr_draw->pixel_bytes / 8);
}
