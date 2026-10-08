// Copyright (C) 2017 The Android Open Source Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

package aidl

import (
	"github.com/google/blueprint/proptools"
)

type nameProperties struct {
	Name *string
}

type hostProperties struct {
	Cflags []string
}

type imageProperties struct {
	Shared_libs         []string
	Header_libs         []string
	Exclude_shared_libs []string
	Cflags              []string
}

type ccTargetProperties struct {
	Host     hostProperties
	Platform imageProperties
	Vendor   imageProperties
	Product  imageProperties
}

type ccProperties struct {
	Name                      *string
	Enabled                   proptools.Configurable[bool]
	Owner                     *string
	Defaults                  []string
	Double_loadable           *bool
	Vendor_available          *bool
	Odm_available             *bool
	Product_available         *bool
	Recovery_available        *bool
	Host_supported            *bool
	Cmake_snapshot_supported  *bool
	Installable               *bool
	Generated_sources         []string
	Generated_headers         []string
	Shared_libs               []string
	Static_libs               []string
	Export_shared_lib_headers []string
	Export_generated_headers  []string
	Header_libs               []string
	Sdk_version               *string
	Stl                       *string
	Cpp_std                   *string
	Cflags                    []string
	Ldflags                   []string
	Stem                      *string
	Apex_available            []string
	Min_sdk_version           *string
	Target                    ccTargetProperties
	Tidy                      *bool
	Tidy_flags                []string
	Tidy_checks_as_errors     []string
	Include_build_directory   *bool
	AidlInterface             struct {
		Sources  []string
		AidlRoot string
		Lang     string
		Flags    []string
	}
}

type javaProperties struct {
	Name            *string
	Owner           *string
	Defaults        []string
	Installable     *bool
	Sdk_version     *string
	Srcs            []string
	Static_libs     []string
	Apex_available  []string
	Min_sdk_version *string
	Is_stubs_module *bool
}

type rustProperties struct {
	Name              *string
	Enabled           proptools.Configurable[bool]
	Crate_name        string
	Owner             *string
	Defaults          []string
	Host_supported    *bool
	Vendor_available  *bool
	Product_available *bool
	Srcs              []string
	Rustlibs          []string
	Stem              *string
	Apex_available    []string
	Min_sdk_version   *string
}

type phonyProperties struct {
	Name     *string
	Required []string
}
