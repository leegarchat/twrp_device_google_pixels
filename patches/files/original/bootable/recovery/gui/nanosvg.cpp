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

//#define NANOSVG_IMPLEMENTATION	1	// Expands implementation
//#define NANOSVGRAST_CPLUSPLUS 1
#include "nanosvg.h"
#include "nanosvgrast.h"

#include "nanosvg.hpp"
#include "../twcommon.h"
#include "minuitwrp/minui.h"


struct NSVGrasterizer* rast = nsvgCreateRasterizer();

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
