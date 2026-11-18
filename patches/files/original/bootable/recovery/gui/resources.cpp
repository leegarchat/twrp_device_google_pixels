/*
    Copyright 2012 to 2020 TeamWin
	This file is part of TWRP/TeamWin Recovery Project.

	TWRP is free software: you can redistribute it and/or modify
	it under the terms of the GNU General Public License as published by
	the Free Software Foundation, either version 3 of the License, or
	(at your option) any later version.

	TWRP is distributed in the hope that it will be useful,
	but WITHOUT ANY WARRANTY; without even the implied warranty of
	MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
	GNU General Public License for more details.

	You should have received a copy of the GNU General Public License
	along with TWRP.  If not, see <http://www.gnu.org/licenses/>.
*/

// resource.cpp - Source to manage GUI resources

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
#include <ziparchive/zip_archive.h>
#include <android-base/unique_fd.h>

extern "C" {
#include "../twcommon.h"
#include "gui.h"
}

#include "minuitwrp/truetype.hpp"
#include "minuitwrp/minui.h"

#include "rapidxml.hpp"
#include "objects.hpp"
#include "nanosvg.hpp"
#include "nanosvgrast.h"

#define TMP_RESOURCE_NAME   "/tmp/extract.bin"

Resource::Resource(xml_node<>* node, ZipArchiveHandle pZip __unused)
{
	if (node && node->first_attribute("name"))
		mName = node->first_attribute("name")->value();
}

int Resource::ExtractResource(ZipArchiveHandle pZip, std::string folderName, std::string fileName, std::string fileExtn, std::string destFile)
{
	if (!pZip)
		return -1;

	std::string src = folderName + "/" + fileName + fileExtn;
	ZipEntry binary_entry;
	if (FindEntry(pZip, src, &binary_entry) == 0) {
		android::base::unique_fd fd(
			open(destFile.c_str(), O_CREAT | O_WRONLY | O_TRUNC | O_CLOEXEC, 0666));
		if (fd == -1) {
			return -1;
		}
		int32_t err = ExtractEntryToFile(pZip, &binary_entry, fd);
		if (err != 0)
			return -1;
	} else {
		return -1;
	}
	return 0;
}

bool endsWith(std::string const &fullString, std::string const &ending) {
    if (fullString.length() >= ending.length()) {
        return (0 == fullString.compare (fullString.length() - ending.length(), ending.length(), ending));
    } else {
        return false;
    }
}

void Resource::LoadImage(ZipArchiveHandle pZip, std::string file, gr_surface* surface, float scale)
{
	int rc = 0;
	if (ExtractResource(pZip, "images", file, ".png", TMP_RESOURCE_NAME) == 0)
	{
		rc = res_create_surface(TMP_RESOURCE_NAME, surface);
		unlink(TMP_RESOURCE_NAME);
	}
	else if (ExtractResource(pZip, "images", file, "", TMP_RESOURCE_NAME) == 0)
	{
		// JPG includes the .jpg extension in the filename so extension should be blank
		rc = res_create_surface(TMP_RESOURCE_NAME, surface);
		unlink(TMP_RESOURCE_NAME);
	}
	else if (!pZip)
	{
		// File name in xml may have included .png so try without adding .png
		rc = res_create_surface(file.c_str(), surface);
		if (rc == 0) return;
		
		if (endsWith(file, ".svg")) {
			std::string nam = std::string(TWRES) + std::string("images/") + file;
			rc = res_create_svg_surface(nam.c_str(), scale, surface);
		}
	}
	if (rc != 0)
		LOGINFO("Failed to load image from %s%s, error %d\n", file.c_str(), pZip ? " (zip)" : "", rc);
}

void Resource::CheckAndScaleImage(gr_surface source, gr_surface* destination, int retain_aspect)
{
	if (!source) {
		*destination = nullptr;
		return;
	}
	if (get_scale_w() != 0 && get_scale_h() != 0) {
		float scale_w = get_scale_w(), scale_h = get_scale_h();
		if (retain_aspect) {
			if (scale_w < scale_h)
				scale_h = scale_w;
			else
				scale_w = scale_h;
		}
		if (res_scale_surface(source, destination, scale_w, scale_h)) {
			LOGINFO("Error scaling image, using regular size.\n");
			*destination = source;
		}
	} else {
		*destination = source;
	}
}

FontResource::FontResource(xml_node<>* node, ZipArchiveHandle pZip)
 : Resource(node, pZip)
{
	origFontSize = 0;
	origFont = NULL;
	LoadFont(node, pZip);
}

void FontResource::LoadFont(xml_node<>* node, ZipArchiveHandle pZip)
{
	std::string file;
	xml_attribute<>* attr;

	mFont = NULL;
	if (!node)
		return;

	attr = node->first_attribute("filename");
	if (!attr)
		return;

	file = attr->value();

	if (file.size() >= 4 && file.compare(file.size()-4, 4, ".ttf") == 0)
	{
		int font_size = 0;

		if (origFontSize != 0) {
			attr = node->first_attribute("scale");
			if (attr == NULL)
				return;
			font_size = origFontSize * atoi(attr->value()) / 100;
		} else {
			attr = node->first_attribute("size");
			if (attr == NULL)
				return;
			font_size = scale_theme_min(atoi(attr->value()));
			origFontSize = font_size;
		}

		int dpi = 300;

		attr = node->first_attribute("dpi");
		if (attr)
			dpi = atoi(attr->value());

		// we can't use TMP_RESOURCE_NAME here because the ttf subsystem is caching the name and scaling needs to reload the font
		std::string tmpname = "/tmp/" + file;
		if (ExtractResource(pZip, "fonts", file, "", tmpname) == 0)
		{
			mFont = twrpTruetype::gr_ttf_loadFont(tmpname.c_str(), font_size, dpi);
		}
		else
		{
			file = std::string(TWRES "fonts/") + file;
			mFont = twrpTruetype::gr_ttf_loadFont(file.c_str(), font_size, dpi);
		}
	}
	else
	{
		LOGERR("Non-TTF fonts are no longer supported.\n");
	}
}

void FontResource::DeleteFont() {
	if (mFont) {
		twrpTruetype::gr_ttf_freeFont(mFont);
	}
	mFont = NULL;
	if (origFont) {
		twrpTruetype::gr_ttf_freeFont(origFont);
	}
	origFont = NULL;
}

void FontResource::Override(xml_node<>* node, ZipArchiveHandle pZip) {
	if (!origFont) {
		origFont = mFont;
	} else if (mFont) {
		twrpTruetype::gr_ttf_freeFont(mFont);
		mFont = NULL;
	}
	LoadFont(node, pZip);
}

FontResource::~FontResource()
{
	DeleteFont();
}

ImageResource::ImageResource(xml_node<>* node, ZipArchiveHandle pZip)
 : Resource(node, pZip)
{
	std::string file;
	gr_surface temp_surface = nullptr;

	mSurface = NULL;
	if (!node) {
		LOGERR("ImageResource node is NULL\n");
		return;
	}

	if (node->first_attribute("filename"))
		file = node->first_attribute("filename")->value();
	else {
		LOGERR("No filename specified for image resource.\n");
		return;
	}

	bool retain_aspect = (node->first_attribute("retainaspect") != NULL);
	float scale = LoadAttrFloat(node, "scale", 1.0);
	// the value does not matter, if retainaspect is present, we assume that we want to retain it
	LoadImage(pZip, file, &temp_surface, scale);

	bool found_color_attr = false;
	COLOR color = LoadAttrColor(node, "tint", &found_color_attr);

	if (found_color_attr && temp_surface) {
		GGLSurface *surface = (GGLSurface*)temp_surface;
		int size = surface->width * surface->height;
		uint32_t *data = (uint32_t *)surface->data;

		const uint32_t tint_rgb = (color.blue << 16) | (color.green << 8) | color.red;

		for (int i = 0; i < size; i++) {
			uint8_t alpha = (*(data + i) >> 24) & 0xFF;

			if (alpha > 0) {
				*(data + i) = (alpha << 24) | tint_rgb;
			}
		}
	}

	CheckAndScaleImage(temp_surface, &mSurface, retain_aspect);
}

// [f/d] draw antialiased circles, rectangles, rounded rectangles
// radius == -1 - fully rounded sides
// stroke == 0 - filled shape
// It's here because modifying minuitwrp requires full project rebuild
uint32_t* createShape(int w, int h, int radius, int stroke, COLOR color, RoundedCornerFlags corners_to_round) {
    const int malloc_size = w * h * 4;
    uint32_t *data = (uint32_t *)malloc(malloc_size);
    if (!data) return nullptr; // Handle allocation failure
    memset(data, 0, malloc_size);

    const uint32_t px = (color.alpha << 24) | (color.blue << 16) | (color.green << 8) | color.red;

    // --- Simplification for non-rounded, filled rectangle ---
    if (radius == 0 && stroke == 0 && corners_to_round == RoundedCornerFlags::NONE) {
        if (color.alpha != 0) {
            for (int i = 0; i < w * h; i++) {
                *(data + i) = px;
            }
        }
        return data;
    }
    // --- --- --- --- --- --- --- --- --- --- --- --- --- ---

    int min_side = std::min(w, h) / 2;
    if (radius < 0 || radius > min_side) radius = min_side; // Fully rounded if radius is invalid or too large
    if (stroke < 0 || stroke > min_side) stroke = 0;      // Treat invalid stroke as fill

    // Optimization: if no corners need rounding, behave as if radius is 0
    if (corners_to_round == RoundedCornerFlags::NONE && radius > 0) {
        radius = 0;
    }

    // Handle the completely non-rounded case (either radius 0 or no corners selected)
    if (radius == 0) {
        if (stroke == 0) { // Filled rectangle
             if (color.alpha != 0) {
                for (int i = 0; i < w * h; i++) {
                    *(data + i) = px;
                }
            }
        } else { // Stroked rectangle
             const uint32_t px_aa = ((color.alpha / 2) << 24) | (color.blue << 16) | (color.green << 8) | color.red; // Simple AA for stroke
             // Top & Bottom lines
            for (int x = 0; x < w; ++x) {
                for(int s=0; s<stroke; ++s){
                    if(color.alpha != 0) {
                        *(data + x + s * w) = px; // Top
                        *(data + x + (h - 1 - s) * w) = px; // Bottom
                    }
                    // Add very basic AA on the inner edge
                    if(s == stroke-1 && stroke > 0 && h > stroke*2) {
                        *(data + x + (s+1) * w) = px_aa; // Below top
                        *(data + x + (h - 1 - s - 1) * w) = px_aa; // Above bottom
                    }
                }
            }
            // Left & Right lines
            for (int y = stroke; y < h - stroke; ++y) {
                 for(int s=0; s<stroke; ++s){
                     if(color.alpha != 0) {
                        *(data + y * w + s) = px; // Left
                        *(data + y * w + (w - 1 -s)) = px; // Right
                     }
                     // Add very basic AA on the inner edge
                     if(s == stroke-1 && stroke > 0 && w > stroke*2) {
                         *(data + y * w + s + 1) = px_aa; // Right of Left
                         *(data + y * w + (w - 1 - s - 1)) = px_aa; // Left of Right
                     }
                 }
            }
        }
        return data; // Return the non-rounded shape
    }


    // --- Rounded corner logic proceeds if radius > 0 and at least one corner needs rounding ---
    const int diameter = radius * 2;
    const float radius2 = radius - 0.5f; // For centering circle calculations

    const float radius_sq_outer = radius2 * radius2; // Base radius squared
    // Adjusted checks for smoother anti-aliasing
    const float radius_check = radius_sq_outer + radius2 * 0.6f;
    const float radius_check_aa = radius_sq_outer + radius2 * 1.4f;

    const uint32_t px_aa = ((color.alpha / 2) << 24) | (color.blue << 16) | (color.green << 8) | color.red;

    // Calculate inner radius checks only if stroking
    float radius_check_hollow = 0;
    float radius_check_hollow_aa = 0;
    bool is_stroking = (stroke > 0);
    if (is_stroking) {
        // Subtract squared stroke effect from outer radius squared
        float inner_radius = std::max(0.0f, radius2 - stroke);
        float inner_radius_sq = inner_radius * inner_radius;
        radius_check_hollow = inner_radius_sq - inner_radius * 0.4f;
         radius_check_hollow_aa = inner_radius_sq - inner_radius * 1.0f;
    }


    // Iterate through the entire potential shape buffer
    for (int y = 0; y < h; ++y) {
        for (int x = 0; x < w; ++x) {
            uint32_t final_px = 0; // Pixel color for this position, start with transparent

            // Determine relative coordinates to the nearest rounded corner center (if applicable)
            float rx = x - radius2;          // Relative to left edge circle center X
            float ry = y - radius2;          // Relative to top edge circle center Y
            float rx_right = x - (w - radius2 - 1); // Relative to right edge circle center X
            float ry_bottom = y - (h - radius2 - 1); // Relative to bottom edge circle center Y

            bool in_top_left_corner = (x < radius && y < radius && (corners_to_round & RoundedCornerFlags::TOP_LEFT));
            bool in_top_right_corner = (x >= w - radius && y < radius && (corners_to_round & RoundedCornerFlags::TOP_RIGHT));
            bool in_bottom_left_corner = (x < radius && y >= h - radius && (corners_to_round & RoundedCornerFlags::BOTTOM_LEFT));
            bool in_bottom_right_corner = (x >= w - radius && y >= h - radius && (corners_to_round & RoundedCornerFlags::BOTTOM_RIGHT));

            float dist_sq = -1.0f; // Negative indicates not in a rounded corner check area

            // Calculate squared distance only if inside a potential corner area
            if (in_top_left_corner) {
                dist_sq = rx * rx + ry * ry;
            } else if (in_top_right_corner) {
                dist_sq = rx_right * rx_right + ry * ry;
            } else if (in_bottom_left_corner) {
                dist_sq = rx * rx + ry_bottom * ry_bottom;
            } else if (in_bottom_right_corner) {
                dist_sq = rx_right * rx_right + ry_bottom * ry_bottom;
            }

            // --- Determine pixel color based on fill/stroke and position ---

            if (dist_sq >= 0) { // Pixel is inside a rounded corner's quadrant
                 if (!is_stroking) { // --- Filled Shape ---
                    if (dist_sq <= radius_check) final_px = px;
                    else if (dist_sq <= radius_check_aa) final_px = px_aa;
                    // else final_px remains 0 (transparent)
                } else { // --- Stroked Shape ---
                    if (dist_sq <= radius_check && dist_sq > radius_check_hollow) final_px = px;
                    else if (dist_sq <= radius_check_aa && dist_sq > radius_check_hollow_aa) final_px = px_aa; // Anti-alias edges
                    // else final_px remains 0 (transparent outside stroke or inside hole)
                }
            } else { // Pixel is in the straight part of the rectangle
                 if (!is_stroking) { // --- Filled Shape ---
                    final_px = px;
                 } else { // --- Stroked Shape ---
                     // Check if pixel falls within the stroke thickness on any straight edge
                    if (x < stroke || x >= w - stroke || y < stroke || y >= h - stroke) {
                         // Basic check, add AA possibility near inner edge
                         final_px = px;
                        // Very basic AA attempt on inner edge of straight stroke (can be improved)
                        if (x == stroke || x == w - stroke - 1 || y == stroke || y == h - stroke-1) {
                           final_px = px_aa;
                        }
                    }
                    // else final_px remains 0 (transparent center for stroke)
                 }
            }

            *(data + y * w + x) = final_px;
        }
    }

    return data;
}

// [f/d] constructor for fake images that are actually shapes
// Usage: <resources><shape name="img_name" color="#FFFFFF" w="100" h="50" radius="10" stroke="0" /> </resources>
// Then, reference it like normal image: <image resource="img_name"/>
ImageResource::ImageResource(xml_node<>* node) : Resource(node, NULL)
{
	if (!node) {
		LOGERR("ImageResource node is NULL\n");
		return;
	}

	int original_radius = LoadAttrInt(node, "radius", 0),
		w = LoadAttrIntScaleX(node, "w", 1),
		h = LoadAttrIntScaleY(node, "h", 1),
		r = original_radius <= 0 ? original_radius : scale_theme_x(original_radius), //don't scale -1 value
		s = LoadAttrIntScaleY(node, "stroke", 0);

	COLOR color = LoadAttrColor(node, "color", COLOR(0,0,0));
	
	GGLSurface *surface;
	surface = (GGLSurface *)malloc(sizeof(GGLSurface));
	memset(surface, 0, sizeof(GGLSurface));

	surface->version = sizeof(surface);
	surface->width = w;
	surface->height = h;
	surface->stride = w;
	surface->data = (GGLubyte*)createShape(w, h, r, s, color);
	surface->format = res_get_pixel_format();

	mSurface = (gr_surface)surface;
}
// [/f/d]

ImageResource::~ImageResource()
{
	if (mSurface)
		res_free_surface(mSurface);
}

AnimationResource::AnimationResource(xml_node<>* node, ZipArchiveHandle pZip)
 : Resource(node, pZip)
{
	std::string file;
	int fileNum = 1;

	if (!node)
		return;

	if (node->first_attribute("filename"))
		file = node->first_attribute("filename")->value();
	else {
		LOGERR("No filename specified for image resource.\n");
		return;
	}

	bool retain_aspect = (node->first_attribute("retainaspect") != NULL);
	// the value does not matter, if retainaspect is present, we assume that we want to retain it
	for (;;)
	{
		std::ostringstream fileName;
		fileName << file << std::setfill ('0') << std::setw (3) << fileNum;

		gr_surface surface = nullptr;
		gr_surface temp_surface = nullptr;
		LoadImage(pZip, fileName.str(), &temp_surface);
		CheckAndScaleImage(temp_surface, &surface, retain_aspect);
		if (surface) {
			mSurfaces.push_back(surface);
			fileNum++;
		} else
			break; // Done loading animation images
	}
}

AnimationResource::~AnimationResource()
{
	std::vector<gr_surface>::iterator it;

	for (it = mSurfaces.begin(); it != mSurfaces.end(); ++it)
		res_free_surface(*it);

	mSurfaces.clear();
}

FontResource* ResourceManager::FindFont(const std::string& name) const
{
	for (std::vector<FontResource*>::const_iterator it = mFonts.begin(); it != mFonts.end(); ++it)
		if (name == (*it)->GetName())
			return *it;
	return NULL;
}

ImageResource* ResourceManager::FindImage(const std::string& name) const
{
	for (std::vector<ImageResource*>::const_iterator it = mImages.begin(); it != mImages.end(); ++it)
		if (name == (*it)->GetName())
			return *it;
	return NULL;
}

AnimationResource* ResourceManager::FindAnimation(const std::string& name) const
{
	for (std::vector<AnimationResource*>::const_iterator it = mAnimations.begin(); it != mAnimations.end(); ++it)
		if (name == (*it)->GetName())
			return *it;
	return NULL;
}

std::string ResourceManager::FindString(const std::string& name) const
{
	//if (this != NULL) {
		std::map<std::string, string_resource_struct>::const_iterator it = mStrings.find(name);
		if (it != mStrings.end())
			return it->second.value;
		LOGERR("String resource '%s' not found. No default value.\n", name.c_str());
		PageManager::AddStringResource("NO DEFAULT", name, "[" + name + ("]"));
	/*} else {
		LOGINFO("String resources not loaded when looking for '%s'. No default value.\n", name.c_str());
	}*/
	return "[" + name + ("]");
}

std::string ResourceManager::FindString(const std::string& name, const std::string& default_string) const
{
	//if (this != NULL) {
		std::map<std::string, string_resource_struct>::const_iterator it = mStrings.find(name);
		if (it != mStrings.end())
			return it->second.value;
		LOGERR("String resource '%s' not found. Using default value.\n", name.c_str());
		PageManager::AddStringResource("DEFAULT", name, default_string);
	/*} else {
		LOGINFO("String resources not loaded when looking for '%s'. Using default value.\n", name.c_str());
	}*/
	return default_string;
}

void ResourceManager::DumpStrings() const
{
	/*if (this == NULL) {
		gui_print("No string resources\n");
		return;
	}*/
	std::map<std::string, string_resource_struct>::const_iterator it;
	gui_print("Dumping all strings:\n");
	for (it = mStrings.begin(); it != mStrings.end(); it++)
		gui_print("source: %s: '%s' = '%s'\n", it->second.source.c_str(), it->first.c_str(), it->second.value.c_str());
	gui_print("Done dumping strings\n");
}

ResourceManager::ResourceManager()
{
}

void ResourceManager::AddStringResource(std::string resource_source, std::string resource_name, std::string value)
{
	string_resource_struct res;
	res.source = resource_source;
	res.value = value;
	mStrings[resource_name] = res;
}

void ResourceManager::LoadResources(xml_node<>* resList, ZipArchiveHandle pZip, std::string resource_source)
{
	if (!resList)
		return;

	for (xml_node<>* child = resList->first_node(); child; child = child->next_sibling())
	{
		std::string type = child->name();
		if (type == "resource") {
			// legacy format : <resource type="...">
			xml_attribute<>* attr = child->first_attribute("type");
			type = attr ? attr->value() : "*unspecified*";
		}

		bool error = false;
		if (type == "font")
		{
			FontResource* res = new FontResource(child, pZip);
			if (res && res->GetResource())
				mFonts.push_back(res);
			else {
				error = true;
				delete res;
			}
		}
		else if (type == "fontoverride")
		{
			if (mFonts.size() != 0 && child && child->first_attribute("name")) {
				string FontName = child->first_attribute("name")->value();
				size_t font_count = mFonts.size(), i;
				bool found = false;

				for (i = 0; i < font_count; i++) {
					if (mFonts[i]->GetName() == FontName) {
						mFonts[i]->Override(child, pZip);
						found = true;
						break;
					}
				}
				if (!found) {
					LOGERR("Unable to locate font '%s' for override.\n", FontName.c_str());
				}
			} else if (mFonts.size() != 0)
				LOGERR("Unable to locate font name for type fontoverride.\n");
		}
		else if (type == "image")
		{
			ImageResource* res = new ImageResource(child, pZip);
			if (res && res->GetResource())
				mImages.push_back(res);
			else {
				error = true;
				delete res;
			}
		}
		else if (type == "shape")
		{
			ImageResource* res = new ImageResource(child);
			if (res && res->GetResource())
				mImages.push_back(res);
			else {
				error = true;
				delete res;
			}
		}
		else if (type == "animation")
		{
			AnimationResource* res = new AnimationResource(child, pZip);
			if (res && res->GetResourceCount())
				mAnimations.push_back(res);
			else {
				error = true;
				delete res;
			}
		}
		else if (type == "string")
		{
			if (xml_attribute<>* attr = child->first_attribute("name")) {
				string_resource_struct res;
				res.source = resource_source;
				res.value = child->value();
				mStrings[attr->value()] = res;
			} else
				error = true;
		}
		else
		{
			LOGERR("Resource type (%s) not supported.\n", type.c_str());
			error = true;
		}

		if (error)
		{
			std::string res_name;
			if (child->first_attribute("name"))
				res_name = child->first_attribute("name")->value();
			if (res_name.empty() && child->first_attribute("filename"))
				res_name = child->first_attribute("filename")->value();

			if (!res_name.empty()) {
				LOGERR("Resource (%s)-(%s) failed to load\n", type.c_str(), res_name.c_str());
			} else
				LOGERR("Resource type (%s) failed to load\n", type.c_str());
		}
	}
}

ResourceManager::~ResourceManager()
{
	for (std::vector<FontResource*>::iterator it = mFonts.begin(); it != mFonts.end(); ++it)
		delete *it;

	for (std::vector<ImageResource*>::iterator it = mImages.begin(); it != mImages.end(); ++it)
		delete *it;

	for (std::vector<AnimationResource*>::iterator it = mAnimations.begin(); it != mAnimations.end(); ++it)
		delete *it;
}
