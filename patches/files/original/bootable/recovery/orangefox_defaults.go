/*
	Copyright (C) 2021-2025 OrangeFox Recovery Project
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
*/

package twrp

import (
	"android/soong/android"
)

func fox_globalFlags(ctx android.BaseContext) []string {
	var foxflags []string

	if ctx.AConfig().Getenv("FOX_USE_NANO_EDITOR") == "1" {
		foxflags = append(foxflags, "-DFOX_USE_NANO_EDITOR=1")
	}

	if ctx.AConfig().Getenv("OF_ENABLE_LAB") == "1" {
		foxflags = append(foxflags, "-DOF_ENABLE_LAB=1")
	}

	if ctx.AConfig().Getenv("OF_SUPPORT_OZIP_DECRYPTION") == "1" {
		foxflags = append(foxflags, "-DOF_SUPPORT_OZIP_DECRYPTION=1")
	}

	if ctx.AConfig().Getenv("FOX_ALLOW_EARLY_SETTINGS_LOAD") == "1" {
		foxflags = append(foxflags, "-DFOX_ALLOW_EARLY_SETTINGS_LOAD=1")
	}

	if ctx.AConfig().Getenv("FOX_SETTINGS_ROOT_DIRECTORY") != "" {
		foxflags = append(foxflags, "-DFOX_SETTINGS_ROOT_DIRECTORY="+"\""+ctx.AConfig().Getenv("FOX_SETTINGS_ROOT_DIRECTORY")+"\"")
	}

	if ctx.AConfig().Getenv("FOX_MISCELLANEOUS_ROOT_DIRECTORY") != "" {
		foxflags = append(foxflags, "-DFOX_MISCELLANEOUS_ROOT_DIRECTORY="+"\""+ctx.AConfig().Getenv("FOX_MISCELLANEOUS_ROOT_DIRECTORY")+"\"")
	}

	if ctx.AConfig().Getenv("FOX_USE_DATA_RECOVERY_FOR_SETTINGS") == "1" {
		foxflags = append(foxflags, "-DFOX_USE_DATA_RECOVERY_FOR_SETTINGS=1")
		foxflags = append(foxflags, "-DFOX_SETTINGS_ROOT_DIRECTORY=\"/data/recovery\"")
		foxflags = append(foxflags, "-DFOX_MISCELLANEOUS_ROOT_DIRECTORY=\"/data/recovery\"")
	}

	if ctx.AConfig().Getenv("FOX_USE_MEIZU_TOUCH_MAPPING") == "1" {
		foxflags = append(foxflags, "-DFOX_USE_MEIZU_TOUCH_MAPPING=1")
	}

	return foxflags
}

