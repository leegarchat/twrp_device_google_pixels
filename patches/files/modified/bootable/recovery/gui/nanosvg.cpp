/*
	Copyright (C) 2020-2023 OrangeFox Recovery Project
	This file is part of the OrangeFox Recovery Project.
	
	OrangeFox is free software: you can redistribute it and/or modify
	it under the terms of the GNU General Public License as published by
	the Free Software Foundation, either version 3 of the License, or
	(at your option) any later version.
	
	OrangeFox is distributed in the hope that it will be useful,
	but WITHOUT ANY WARRANTY; without even the implied warranty of
	MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
	GNU General Public License for more details.
	
	You should have received a copy of the GNU General Public License
	along with OrangeFox.  If not, see <http://www.gnu.org/licenses/>.

	This file is neeeded because of twinstall being moved to Soong,
	such that conditions from Android.mk etc, are not being processed.
	So we use boolean functions here to do that job.
	
*/

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include <string>
#include <sstream>
#include <iostream>
#include <iomanip>
#include <fcntl.h>
#include <cutils/properties.h>

//#define NANOSVG_IMPLEMENTATION	1	// Expands implementation
//#define NANOSVGRAST_CPLUSPLUS 1
#include "nanosvg.h"
#include "nanosvgrast.h"

#include "nanosvg.hpp"
#include "../twcommon.h"
#include "minuitwrp/minui.h"


struct NSVGrasterizer* rast = nsvgCreateRasterizer();

// Fox AIO runtime R/B-swap selector (same contract as fox_rb_swap() in
// minuitwrp/graphics.cpp: ro.recovery.rb_swap="1" for malibu/laguna,
// "0" otherwise, absent = no swap). Separate TU, same one-line cache.
static int g_fox_rb_swap = -1;
static bool fox_rb_swap(void)
{
	if (g_fox_rb_swap < 0) {
		char v[PROPERTY_VALUE_MAX];
		g_fox_rb_swap = (property_get("ro.recovery.rb_swap", v, "") > 0 && v[0] == '1') ? 1 : 0;
		printf("fox_rb_swap(svg): ro.recovery.rb_swap=%s -> %s\n", v[0] ? v : "(absent)",
			g_fox_rb_swap ? "SWAP" : "no-swap");
	}
	return g_fox_rb_swap == 1;
}

int res_create_svg_surface(const char* name, float scale, gr_surface* pSurface) {
	*pSurface = NULL;

	if (rast == NULL) {
		LOGINFO("Could not init rasterizer.\n");
		return -1;
	}

	NSVGimage* image = NULL;
	image = nsvgParseFromFile(name, "px", 96);
	if (!image) {
		LOGINFO("Failed to parse target svg image.\n");
		return -2;
	}

	int w = (int)image->width * scale,
		h = (int)image->height * scale;

	uint32_t* img = (uint32_t*)malloc(w*h*4);
	if (img == NULL) {
		LOGINFO("Could not alloc image buffer.\n");
		return -3;
	}

	nsvgRasterize(rast, image, 0, 0, scale, (unsigned char*)img, w, h, w*4);

	// Fox AIO: the rasterizer always emits RGBA byte order, but the PNG/JPG
	// decoders swap R/B at load for malibu/laguna -- so swap here too, or SVG
	// icons disagree with every other pixel. Default path leaves the buffer
	// untouched (bit-identical to today).
	if (fox_rb_swap()) {
		unsigned char* p = (unsigned char*)img;
		for (int i = 0; i < w * h; i++, p += 4) {
			unsigned char t = p[0];
			p[0] = p[2];
			p[2] = t;
		}
	}

	GGLSurface *surface;
	surface = (GGLSurface *)malloc(sizeof(GGLSurface));
	memset(surface, 0, sizeof(GGLSurface));

	surface->version = sizeof(surface);
	surface->width = w;
	surface->height = h;
	surface->stride = w;
	surface->data = (GGLubyte*)img;
	surface->format = res_get_pixel_format();

	*pSurface = (gr_surface*)surface;

	return 0;
}
