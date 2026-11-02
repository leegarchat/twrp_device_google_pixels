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
 *
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
#include <pixelflinger/pixelflinger.h>
#include "gui/placement.h"
#include "minuitwrp/minui.h"
#include "graphics.h"
// For std::min and std::max
#include <algorithm>
#include "minuitwrp/truetype.hpp"

// for fox_16.0
#include <math.h>

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

// Active clip-region stack (unrotated logical coords). Empty == no bounds, so
// gr_clip() behaves exactly as before. When non-empty, gr_clip() and
// gr_clip_push() intersect with the top so nested viewports compose.
#define GR_CLIP_STACK_MAX 8
static struct { int x, y, w, h; } gr_clip_stack[GR_CLIP_STACK_MAX];
// True logical nesting depth: always incremented on push / decremented on pop so
// the stack can never desync, even past GR_CLIP_STACK_MAX. Stored regions stop at
// GR_CLIP_STACK_MAX; deeper levels reuse the last stored region (a superset).
static int gr_clip_stack_depth = 0;

// Index of the active stored region, clamped to the storage capacity.
static int gr_clip_top_index()
{
    int d = gr_clip_stack_depth < GR_CLIP_STACK_MAX ? gr_clip_stack_depth : GR_CLIP_STACK_MAX;
    return d - 1;
}

// Apply a logical (unrotated) rectangle to the GL scissor.
static void gr_apply_scissor(int x, int y, int w, int h)
{
    GGLContext *gl = gr_context;

    if (w < 0) w = 0;
    if (h < 0) h = 0;

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
}

// Intersect (x,y,w,h) with the active clip bounds, if any.
static void gr_clip_intersect_active(int* x, int* y, int* w, int* h)
{
    if (gr_clip_stack_depth <= 0)
        return;

    int top = gr_clip_top_index();
    int bx = gr_clip_stack[top].x;
    int by = gr_clip_stack[top].y;
    int bw = gr_clip_stack[top].w;
    int bh = gr_clip_stack[top].h;

    int x0 = *x > bx ? *x : bx;
    int y0 = *y > by ? *y : by;
    int x1 = (*x + *w) < (bx + bw) ? (*x + *w) : (bx + bw);
    int y1 = (*y + *h) < (by + bh) ? (*y + *h) : (by + bh);

    *x = x0;
    *y = y0;
    *w = x1 - x0;
    *h = y1 - y0;
}

void gr_clip(int x, int y, int w, int h)
{
    // Honor an active pushed region so child widgets can't draw outside it.
    gr_clip_intersect_active(&x, &y, &w, &h);
    gr_apply_scissor(x, y, w, h);
}

void gr_clip_push(int x, int y, int w, int h)
{
    // Intersect with the current top before storing, so the stack always holds
    // the effective (already-clamped) region.
    gr_clip_intersect_active(&x, &y, &w, &h);

    if (gr_clip_stack_depth < GR_CLIP_STACK_MAX) {
        gr_clip_stack[gr_clip_stack_depth].x = x;
        gr_clip_stack[gr_clip_stack_depth].y = y;
        gr_clip_stack[gr_clip_stack_depth].w = w;
        gr_clip_stack[gr_clip_stack_depth].h = h;
    }
    // Always advance the logical depth so a matching pop stays balanced even when
    // nesting exceeds the storage capacity.
    gr_clip_stack_depth++;
    gr_apply_scissor(x, y, w, h);
}

void gr_clip_pop()
{
    if (gr_clip_stack_depth > 0)
        gr_clip_stack_depth--;

    if (gr_clip_stack_depth > 0) {
        int top = gr_clip_top_index();
        gr_apply_scissor(gr_clip_stack[top].x,
                         gr_clip_stack[top].y,
                         gr_clip_stack[top].w,
                         gr_clip_stack[top].h);
    } else {
        gr_noclip();
    }
}

void gr_noclip()
{
    GGLContext *gl = gr_context;
    // While a region is pushed, "no clip" means "back to that region", not the
    // whole screen - otherwise a child widget could escape its viewport.
    if (gr_clip_stack_depth > 0) {
        int top = gr_clip_top_index();
        gr_apply_scissor(gr_clip_stack[top].x,
                         gr_clip_stack[top].y,
                         gr_clip_stack[top].w,
                         gr_clip_stack[top].h);
        return;
    }
    gl->scissor(gl, 0, 0,
                gr_draw->width - 2 * overscan_offset_x,
                gr_draw->height - 2 * overscan_offset_y);
    gl->disable(gl, GGL_SCISSOR_TEST);
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
    const uint32_t px = (a << 24) | (b << 16) | (g << 8) | r;
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
#if defined(RECOVERY_ARGB) || defined(RECOVERY_BGRA) || defined(RECOVERY_ABGR)
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
}

void gr_blit(gr_surface source, int sx, int sy, int w, int h, int dx, int dy)
{
    if (gr_context == NULL) {
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
}

void gr_blit_rotated(gr_surface source, int sx, int sy, int w, int h, int dx, int dy, int angle)
{
	if (!source || w <= 0 || h <= 0)
		return;

	if (!gr_draw || !gr_draw->data || gr_draw->pixel_bytes <= 0 || gr_draw->row_bytes <= 0)
		return;

	while (angle < 0)
		angle += 360;

	angle %= 360;

	const int bpp = gr_draw->pixel_bytes;
	const int temp_row_bytes = w * bpp;
	const size_t temp_size = static_cast<size_t>(temp_row_bytes) * static_cast<size_t>(h);

	if (temp_size == 0)
		return;

	unsigned char* before_data = static_cast<unsigned char*>(calloc(1, temp_size));
	unsigned char* after_data = static_cast<unsigned char*>(calloc(1, temp_size));

	if (!before_data || !after_data) {
		if (before_data)
			free(before_data);
		if (after_data)
			free(after_data);

		gr_blit(source, sx, sy, w, h, dx, dy);
		return;
	}

	/*
	 * Save the current framebuffer rectangle behind the spinner.
	 */
	for (int y = 0; y < h; ++y) {
		const int screen_y = dy + y;

		if (screen_y < 0 || screen_y >= gr_draw->height)
			continue;

		for (int x = 0; x < w; ++x) {
			const int screen_x = dx + x;

			if (screen_x < 0 || screen_x >= gr_draw->width)
				continue;

			unsigned char* dst =
				before_data + (y * temp_row_bytes) + (x * bpp);

			const unsigned char* src =
				gr_draw->data + (screen_y * gr_draw->row_bytes) + (screen_x * bpp);

			memcpy(dst, src, bpp);
		}
	}

	/*
	 * Draw the source normally using the existing working image/SVG path.
	 */
	gr_blit(source, sx, sy, w, h, dx, dy);

	/*
	 * Capture the rectangle after normal gr_blit().
	 */
	for (int y = 0; y < h; ++y) {
		const int screen_y = dy + y;

		if (screen_y < 0 || screen_y >= gr_draw->height)
			continue;

		for (int x = 0; x < w; ++x) {
			const int screen_x = dx + x;

			if (screen_x < 0 || screen_x >= gr_draw->width)
				continue;

			unsigned char* dst =
				after_data + (y * temp_row_bytes) + (x * bpp);

			const unsigned char* src =
				gr_draw->data + (screen_y * gr_draw->row_bytes) + (screen_x * bpp);

			memcpy(dst, src, bpp);
		}
	}

	/*
	 * Restore the original background so the unrotated icon is removed.
	 */
	for (int y = 0; y < h; ++y) {
		const int screen_y = dy + y;

		if (screen_y < 0 || screen_y >= gr_draw->height)
			continue;

		for (int x = 0; x < w; ++x) {
			const int screen_x = dx + x;

			if (screen_x < 0 || screen_x >= gr_draw->width)
				continue;

			unsigned char* dst =
				gr_draw->data + (screen_y * gr_draw->row_bytes) + (screen_x * bpp);

			const unsigned char* src =
				before_data + (y * temp_row_bytes) + (x * bpp);

			memcpy(dst, src, bpp);
		}
	}

	const double radians = -static_cast<double>(angle) * 3.14159265358979323846 / 180.0;
	const double cs = cos(radians);
	const double sn = sin(radians);

	const double cx = static_cast<double>(w - 1) / 2.0;
	const double cy = static_cast<double>(h - 1) / 2.0;

	/*
	 * Rotate only pixels changed by gr_blit().
	 * Unchanged pixels are treated as transparent/empty.
	 */
	for (int y = 0; y < h; ++y) {
		const int screen_y = dy + y;

		if (screen_y < 0 || screen_y >= gr_draw->height)
			continue;

		for (int x = 0; x < w; ++x) {
			const int screen_x = dx + x;

			if (screen_x < 0 || screen_x >= gr_draw->width)
				continue;

			const double dx_local = static_cast<double>(x) - cx;
			const double dy_local = static_cast<double>(y) - cy;

			const double src_x_f = (cs * dx_local) - (sn * dy_local) + cx;
			const double src_y_f = (sn * dx_local) + (cs * dy_local) + cy;

			const int src_x = static_cast<int>(floor(src_x_f + 0.5));
			const int src_y = static_cast<int>(floor(src_y_f + 0.5));

			if (src_x < 0 || src_x >= w || src_y < 0 || src_y >= h)
				continue;

			const unsigned char* before_pixel =
				before_data + (src_y * temp_row_bytes) + (src_x * bpp);

			const unsigned char* after_pixel =
				after_data + (src_y * temp_row_bytes) + (src_x * bpp);

			if (memcmp(before_pixel, after_pixel, bpp) == 0)
				continue;

			unsigned char* dst_pixel =
				gr_draw->data + (screen_y * gr_draw->row_bytes) + (screen_x * bpp);

			memcpy(dst_pixel, after_pixel, bpp);
		}
	}

	free(before_data);
	free(after_data);
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
