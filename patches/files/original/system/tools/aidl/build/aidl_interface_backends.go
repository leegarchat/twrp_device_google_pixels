// Copyright (C) 2021 The Android Open Source Project
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
	"android/soong/android"
	"android/soong/cc"
	"android/soong/java"
	"android/soong/rust"

	"fmt"
	"path/filepath"
	"strings"

	"github.com/google/blueprint/proptools"
)

func addLibrary(mctx android.DefaultableHookContext, i *aidlInterface, version string, lang string, notFrozen bool, requireFrozenReason string) string {
	if lang == langJava {
		return addJavaLibrary(mctx, i, version, notFrozen, requireFrozenReason)
	} else if lang == langRust {
		return addRustLibrary(mctx, i, version, notFrozen, requireFrozenReason)
	} else if lang == langCppAnalyzer {
		return addCppAnalyzerLibrary(mctx, i, version, notFrozen, requireFrozenReason)
	} else if lang == langCpp || lang == langNdk || lang == langNdkPlatform {
		return addCppLibrary(mctx, i, version, lang, notFrozen, requireFrozenReason)
	} else {
		panic(fmt.Errorf("unsupported language backend %q\n", lang))
	}
}

func addCppLibrary(mctx android.DefaultableHookContext, i *aidlInterface, version string, lang string, notFrozen bool, requireFrozenReason string) string {
	cppSourceGen := i.versionedName(version) + "-" + lang + "-source"
	cppModuleGen := i.versionedName(version) + "-" + lang

	srcs, aidlRoot := i.srcsForVersion(mctx, version)
	if len(srcs) == 0 {
		// This can happen when the version is about to be frozen; the version
		// directory is created but API dump hasn't been copied there.
		// Don't create a library for the yet-to-be-frozen version.
		return ""
	}

	var commonProperties *CommonNativeBackendProperties
	if lang == langCpp {
		commonProperties = &i.properties.Backend.Cpp.CommonNativeBackendProperties
	} else if lang == langNdk || lang == langNdkPlatform {
		commonProperties = &i.properties.Backend.Ndk.CommonNativeBackendProperties
	}

	genLog := proptools.Bool(commonProperties.Gen_log)
	genTrace := i.genTrace(lang)
	aidlFlags := i.flagsForAidlGenRule(version)

	mctx.CreateModule(aidlGenFactory, &nameProperties{
		Name: proptools.StringPtr(cppSourceGen),
	}, &aidlGenProperties{
		Srcs:                srcs,
		AidlRoot:            aidlRoot,
		Imports:             i.getImportsForVersion(version),
		Headers:             i.properties.Headers,
		Stability:           i.properties.Stability,
		Min_sdk_version:     i.minSdkVersion(lang),
		Lang:                lang,
		BaseName:            i.ModuleBase.Name(),
		GenLog:              genLog,
		Version:             i.versionForInitVersionCompat(version),
		GenTrace:            genTrace,
		Unstable:            i.properties.Unstable,
		NotFrozen:           notFrozen,
		RequireFrozenReason: requireFrozenReason,
		Flags:               aidlFlags,
		UseUnfrozen:         i.useUnfrozen(mctx),
	})

	importExportDependencies := []string{}
	sharedLibDependency := commonProperties.Additional_shared_libraries
	var headerLibs []string
	var sdkVersion *string
	var stl *string
	var cpp_std *string
	var hostSupported *bool
	addCflags := commonProperties.Cflags
	targetProp := ccTargetProperties{}

	if lang == langCpp {
		importExportDependencies = append(importExportDependencies, "libbinder", "libutils")
		if genTrace {
			sharedLibDependency = append(sharedLibDependency, "libcutils")
		}
		hostSupported = i.properties.Host_supported
	} else if lang == langNdk || lang == langNdkPlatform {
		importExportDependencies = append(importExportDependencies, "libbinder_ndk")
		nonAppProps := imageProperties{
			Cflags: []string{"-DBINDER_STABILITY_SUPPORT"},
		}
		targetProp.Platform = nonAppProps
		targetProp.Vendor = nonAppProps
		targetProp.Product = nonAppProps
		hostSupported = i.properties.Host_supported
		if lang == langNdk && i.shouldGenerateAppNdkBackend() {
			sdkVersion = i.properties.Backend.Ndk.Sdk_version
			if sdkVersion == nil {
				sdkVersion = proptools.StringPtr("current")
			}

			// Don't worry! This maps to libc++.so for the platform variant.
			stl = proptools.StringPtr("c++_shared")
		}
	} else {
		panic("Unrecognized language: " + lang)
	}

	vendorAvailable := i.properties.Vendor_available
	odmAvailable := i.properties.Odm_available
	productAvailable := i.properties.Product_available
	recoveryAvailable := i.properties.Recovery_available
	if lang == langCpp {
		// Vendor and product modules cannot use the libbinder (cpp) backend of AIDL in a
		// way that is stable. So, in order to prevent accidental usage of these library by
		// vendor and product forcibly disabling this version of the library.
		//
		// It may be the case in the future that we will want to enable this (if some generic
		// helper should be used by both libbinder vendor things using /dev/vndbinder as well
		// as those things using /dev/binder + libbinder_ndk to talk to stable interfaces).

		// As libbinder is not available for the product processes, we must not create
		// product variant for the aidl_interface
		productAvailable = nil
	}

	langProps := &aidlLanguageModuleProperties{}
	langProps.Aidl_internal_props.Lang = lang
	langProps.Aidl_internal_props.AidlInterfaceName = i.ModuleBase.Name()
	langProps.Aidl_internal_props.Version = version
	langProps.Aidl_internal_props.Imports = i.getImportsForVersion(version)

	mctx.CreateModule(wrapLibraryFactory(aidlCcModuleFactory), langProps, &ccProperties{
		Name: proptools.StringPtr(cppModuleGen),
		Enabled: android.CreateSelectOsToBool(map[string]*bool{
			"":       nil,
			"darwin": proptools.BoolPtr(false),
		}),
		Vendor_available:          vendorAvailable,
		Odm_available:             odmAvailable,
		Product_available:         productAvailable,
		Recovery_available:        recoveryAvailable,
		Host_supported:            hostSupported,
		Cmake_snapshot_supported:  i.properties.Cmake_snapshot_supported,
		Defaults:                  []string{"aidl-cpp-module-defaults"},
		Double_loadable:           i.properties.Double_loadable,
		Generated_sources:         []string{cppSourceGen},
		Generated_headers:         []string{cppSourceGen},
		Export_generated_headers:  []string{cppSourceGen},
		Shared_libs:               append(importExportDependencies, sharedLibDependency...),
		Header_libs:               headerLibs,
		Export_shared_lib_headers: importExportDependencies,
		Sdk_version:               sdkVersion,
		Stl:                       stl,
		Cpp_std:                   cpp_std,
		Cflags:                    append(addCflags, "-Wextra", "-Wall", "-Werror", "-Wextra-semi"),
		Ldflags:                   commonProperties.Ldflags,
		Apex_available:            commonProperties.Apex_available,
		Min_sdk_version:           i.minSdkVersion(lang),
		Target:                    targetProp,
		Tidy:                      proptools.BoolPtr(true),
		// Do the tidy check only for the generated headers
		Tidy_flags: []string{"--header-filter=" + android.PathForOutput(mctx).String() + ".*"},
		Tidy_checks_as_errors: []string{
			"*",
			"-clang-analyzer-deadcode.DeadStores", // b/253079031
			"-clang-analyzer-cplusplus.NewDeleteLeaks",  // b/253079031
			"-clang-analyzer-optin.performance.Padding", // b/253079031
		},
		Include_build_directory: proptools.BoolPtr(false), // b/254682497
		AidlInterface: struct {
			Sources  []string
			AidlRoot string
			Lang     string
			Flags    []string
		}{
			Sources:  srcs,
			AidlRoot: aidlRoot,
			Lang:     lang,
			Flags:    aidlFlags,
		},
	})

	return cppModuleGen
}

func addCppAnalyzerLibrary(mctx android.DefaultableHookContext, i *aidlInterface, version string, notFrozen bool, requireFrozenReason string) string {
	cppAnalyzerSourceGen := i.versionedName("") + "-cpp-analyzer-source"
	cppAnalyzerModuleGen := i.versionedName("") + "-cpp-analyzer"

	srcs, aidlRoot := i.srcsForVersion(mctx, version)
	if len(srcs) == 0 {
		return ""
	}

	mctx.CreateModule(aidlGenFactory, &nameProperties{
		Name: proptools.StringPtr(cppAnalyzerSourceGen),
	}, &aidlGenProperties{
		Srcs:                srcs,
		AidlRoot:            aidlRoot,
		Imports:             i.getImportsForVersion(version),
		Stability:           i.properties.Stability,
		Min_sdk_version:     i.minSdkVersion(langCpp),
		Lang:                langCppAnalyzer,
		BaseName:            i.ModuleBase.Name(),
		Version:             i.versionForInitVersionCompat(version),
		Unstable:            i.properties.Unstable,
		NotFrozen:           notFrozen,
		RequireFrozenReason: requireFrozenReason,
		Flags:               i.flagsForAidlGenRule(version),
		UseUnfrozen:         i.useUnfrozen(mctx),
	})

	importExportDependencies := []string{}
	var hostSupported *bool
	var addCflags []string // not using cpp backend cflags for now
	targetProp := ccTargetProperties{}

	importExportDependencies = append(importExportDependencies, "libbinder", "libutils")
	hostSupported = i.properties.Host_supported

	vendorAvailable := i.properties.Vendor_available
	odmAvailable := i.properties.Odm_available
	productAvailable := i.properties.Product_available
	recoveryAvailable := i.properties.Recovery_available
	productAvailable = nil

	commonProperties := &i.properties.Backend.Cpp.CommonNativeBackendProperties

	props := &ccProperties{
		Name: proptools.StringPtr(cppAnalyzerModuleGen),
		Enabled: android.CreateSelectOsToBool(map[string]*bool{
			"":       nil,
			"darwin": proptools.BoolPtr(false),
		}),
		Vendor_available:          vendorAvailable,
		Odm_available:             odmAvailable,
		Product_available:         productAvailable,
		Recovery_available:        recoveryAvailable,
		Host_supported:            hostSupported,
		Defaults:                  []string{"aidl-cpp-module-defaults"},
		Double_loadable:           i.properties.Double_loadable,
		Installable:               proptools.BoolPtr(true),
		Generated_sources:         []string{cppAnalyzerSourceGen},
		Generated_headers:         []string{cppAnalyzerSourceGen},
		Export_generated_headers:  []string{cppAnalyzerSourceGen},
		Shared_libs:               append(append(importExportDependencies, i.versionedName(version)+"-"+langCpp), commonProperties.Additional_shared_libraries...),
		Static_libs:               []string{"aidl-analyzer-main"},
		Export_shared_lib_headers: importExportDependencies,
		Cflags:                    append(addCflags, "-Wextra", "-Wall", "-Werror", "-Wextra-semi"),
		Ldflags:                   commonProperties.Ldflags,
		Min_sdk_version:           i.minSdkVersion(langCpp),
		Target:                    targetProp,
		Tidy:                      proptools.BoolPtr(true),
		// Do the tidy check only for the generated headers
		Tidy_flags: []string{"--header-filter=" + android.PathForOutput(mctx).String() + ".*"},
		Tidy_checks_as_errors: []string{
			"*",
			"-clang-diagnostic-deprecated-declarations", // b/253081572
			"-clang-analyzer-deadcode.DeadStores",       // b/253079031
			"-clang-analyzer-cplusplus.NewDeleteLeaks",  // b/253079031
			"-clang-analyzer-optin.performance.Padding", // b/253079031
		},
	}

	mctx.CreateModule(wrapLibraryFactory(cc.BinaryFactory), props)
	return cppAnalyzerModuleGen
}

func addJavaLibrary(mctx android.DefaultableHookContext, i *aidlInterface, version string, notFrozen bool, requireFrozenReason string) string {
	javaSourceGen := i.versionedName(version) + "-java-source"
	javaModuleGen := i.versionedName(version) + "-java"
	srcs, aidlRoot := i.srcsForVersion(mctx, version)
	if len(srcs) == 0 {
		// This can happen when the version is about to be frozen; the version
		// directory is created but API dump hasn't been copied there.
		// Don't create a library for the yet-to-be-frozen version.
		return ""
	}
	minSdkVersion := i.minSdkVersion(langJava)
	sdkVersion := i.properties.Backend.Java.Sdk_version
	if !proptools.Bool(i.properties.Backend.Java.Platform_apis) && sdkVersion == nil {
		// platform apis requires no default
		sdkVersion = proptools.StringPtr("system_current")
	}
	// use sdkVersion if minSdkVersion is not set
	if sdkVersion != nil && minSdkVersion == nil {
		minSdkVersion = proptools.StringPtr(android.SdkSpecFrom(mctx, *sdkVersion).ApiLevel.String())
	}

	mctx.CreateModule(aidlGenFactory, &nameProperties{
		Name: proptools.StringPtr(javaSourceGen),
	}, &aidlGenProperties{
		Srcs:                srcs,
		AidlRoot:            aidlRoot,
		Imports:             i.getImportsForVersion(version),
		Headers:             i.properties.Headers,
		Stability:           i.properties.Stability,
		Min_sdk_version:     minSdkVersion,
		Platform_apis:       proptools.Bool(i.properties.Backend.Java.Platform_apis),
		Lang:                langJava,
		BaseName:            i.ModuleBase.Name(),
		Version:             version,
		GenRpc:              proptools.Bool(i.properties.Backend.Java.Gen_rpc),
		GenTrace:            i.genTrace(langJava),
		Unstable:            i.properties.Unstable,
		NotFrozen:           notFrozen,
		RequireFrozenReason: requireFrozenReason,
		Flags:               i.flagsForAidlGenRule(version),
		UseUnfrozen:         i.useUnfrozen(mctx),
	})

	langProps := &aidlLanguageModuleProperties{}
	langProps.Aidl_internal_props.Lang = langJava
	langProps.Aidl_internal_props.AidlInterfaceName = i.ModuleBase.Name()
	langProps.Aidl_internal_props.Version = version
	langProps.Aidl_internal_props.Imports = i.getImportsForVersion(version)

	mctx.CreateModule(wrapLibraryFactory(aidlJavaModuleFactory), &javaProperties{
		Name:            proptools.StringPtr(javaModuleGen),
		Installable:     proptools.BoolPtr(true),
		Defaults:        []string{"aidl-java-module-defaults"},
		Sdk_version:     sdkVersion,
		Srcs:            []string{":" + javaSourceGen},
		Apex_available:  i.properties.Backend.Java.Apex_available,
		Min_sdk_version: i.minSdkVersion(langJava),
		Static_libs:     i.properties.Backend.Java.Additional_libs,
		Is_stubs_module: proptools.BoolPtr(true),
	},
		&i.properties.Backend.Java.LintProperties,
		langProps,
	)

	return javaModuleGen
}

func addRustLibrary(mctx android.DefaultableHookContext, i *aidlInterface, version string, notFrozen bool, requireFrozenReason string) string {
	rustSourceGen := i.versionedName(version) + "-rust-source"
	rustModuleGen := i.versionedName(version) + "-rust"
	srcs, aidlRoot := i.srcsForVersion(mctx, version)
	if len(srcs) == 0 {
		// This can happen when the version is about to be frozen; the version
		// directory is created but API dump hasn't been copied there.
		// Don't create a library for the yet-to-be-frozen version.
		return ""
	}

	mctx.CreateModule(aidlGenFactory, &nameProperties{
		Name: proptools.StringPtr(rustSourceGen),
	}, &aidlGenProperties{
		Srcs:                srcs,
		AidlRoot:            aidlRoot,
		Imports:             i.getImportsForVersion(version),
		Headers:             i.properties.Headers,
		Stability:           i.properties.Stability,
		Min_sdk_version:     i.minSdkVersion(langRust),
		Lang:                langRust,
		BaseName:            i.ModuleBase.Name(),
		Version:             i.versionForInitVersionCompat(version),
		Unstable:            i.properties.Unstable,
		NotFrozen:           notFrozen,
		RequireFrozenReason: requireFrozenReason,
		Flags:               i.flagsForAidlGenRule(version),
		UseUnfrozen:         i.useUnfrozen(mctx),
		GenMockall:          proptools.Bool(i.properties.Backend.Rust.Gen_mockall),
	})

	versionedRustName := fixRustName(i.versionedName(version))
	rustCrateName := fixRustName(i.ModuleBase.Name())

	mctx.CreateModule(wrapLibraryFactory(aidlRustLibraryFactory), &rustProperties{
		Name: proptools.StringPtr(rustModuleGen),
		Enabled: android.CreateSelectOsToBool(map[string]*bool{
			"darwin": proptools.BoolPtr(false),
			"":       nil,
		}),
		Crate_name:        rustCrateName,
		Stem:              proptools.StringPtr("lib" + versionedRustName),
		Defaults:          []string{"aidl-rust-module-defaults"},
		Host_supported:    i.properties.Host_supported,
		Vendor_available:  i.properties.Vendor_available,
		Product_available: i.properties.Product_available,
		Apex_available:    i.properties.Backend.Rust.Apex_available,
		Min_sdk_version:   i.minSdkVersion(langRust),
		Rustlibs:          i.properties.Backend.Rust.Additional_rustlibs,
	}, &rust.SourceProviderProperties{
		Source_stem: proptools.StringPtr(versionedRustName),
	}, &aidlRustSourceProviderProperties{
		SourceGen:         rustSourceGen,
		Imports:           i.getImportsForVersion(version),
		Version:           version,
		AidlInterfaceName: i.ModuleBase.Name(),
	})

	return rustModuleGen
}

// This function returns module name with version. Assume that there is foo of which latest version is 2
// Version -> Module name
// "1"->foo-V1
// "2"->foo-V2
// "3"->foo-V3
// And assume that there is 'bar' which is an 'unstable' interface.
// ""->bar
func (i *aidlInterface) versionedName(version string) string {
	name := i.ModuleBase.Name()
	if version == "" {
		return name
	}
	return name + "-V" + version
}

func (i *aidlInterface) srcsForVersion(mctx android.EarlyModuleContext, version string) (srcs []string, aidlRoot string) {
	if version == i.nextVersion() {
		return i.properties.Srcs, i.properties.Local_include_dir
	} else {
		aidlRoot = filepath.Join(aidlApiDir, i.ModuleBase.Name(), version)
		full_paths, err := mctx.GlobWithDeps(filepath.Join(mctx.ModuleDir(), aidlRoot, "**/*.aidl"), nil)
		if err != nil {
			panic(err)
		}
		for _, path := range full_paths {
			// Here, we need path local to the module
			srcs = append(srcs, strings.TrimPrefix(path, mctx.ModuleDir()+"/"))
		}
		return srcs, aidlRoot
	}
}

// For certain backend, avoid a difference between the initial version of a versioned
// interface and an unversioned interface. This ensures that prebuilts can't prevent
// an interface from switching from unversioned to versioned.
func (i *aidlInterface) versionForInitVersionCompat(version string) string {
	if !i.hasVersion() {
		return ""
	}
	return version
}

func (i *aidlInterface) flagsForAidlGenRule(version string) (flags []string) {
	// For the latest unfrozen version of an interface we turn on all warnings and use
	// all flags supplied by the 'flags' field in the aidl_interface module
	if version == i.nextVersion() && !i.isFrozen() {
		flags = append(flags, "-Weverything -Wno-missing-permission-annotation")
		flags = append(flags, i.properties.Flags...)
	}
	return
}

// importing aidl_interface's version  | imported aidl_interface | imported aidl_interface's version
// --------------------------------------------------------------------------------------------------
// whatever                            | unstable                | unstable version
// ToT version(including unstable)     | whatever                | ToT version(unstable if unstable)
// otherwise                           | whatever                | the latest stable version
// In the case that import specifies the version which it wants to use, use that version.
func (i *aidlInterface) getImportWithVersion(version string, anImport string, other *aidlInterface) string {
	if hasVersionSuffix(anImport) {
		return anImport
	}
	if proptools.Bool(other.properties.Unstable) {
		return anImport
	}
	if version == i.nextVersion() || !other.hasVersion() {
		return other.versionedName(other.nextVersion())
	}
	return other.versionedName(other.latestVersion())
}

// Assuming that the context module has deps to its original aidl_interface and imported
// aidl_interface modules with interfaceDepTag and importInterfaceDepTag, returns the list of
// imported interfaces with versions.
func getImportsWithVersion(ctx android.BaseModuleContext, interfaceName, version string) []string {
	// We're using VisitDirectDepsWithTag instead of GetDirectDepWithTag because GetDirectDepWithTag
	// has weird behavior: if you're using a ModuleContext, it will find a dep based off the
	// ModuleBase name, but if you're using a BaseModuleContext, it will find a dep based off of
	// the outer module's name. We need the behavior to be consistent because we call this method
	// with both types of contexts.
	var i *aidlInterface
	ctx.VisitDirectDepsWithTag(interfaceDep, func(visited android.Module) {
		if i == nil && visited.Name() == interfaceName+aidlInterfaceSuffix {
			i = visited.(*aidlInterface)
		}
	})
	if i == nil {
		ctx.ModuleErrorf("expected to find the aidl interface via an interfaceDep, but did not.")
		return nil
	}
	var imports []string
	ctx.VisitDirectDeps(func(dep android.Module) {
		if tag, ok := ctx.OtherModuleDependencyTag(dep).(importInterfaceDepTag); ok {
			other := dep.(*aidlInterface)
			imports = append(imports, i.getImportWithVersion(version, tag.anImport, other))
		}
	})
	return imports
}

func aidlCcModuleFactory() android.Module {
	m := cc.LibraryFactory()
	m.AddProperties(&aidlLanguageModuleProperties{})
	return m
}

func aidlJavaModuleFactory() android.Module {
	m := java.LibraryFactory()
	m.AddProperties(&aidlLanguageModuleProperties{})
	return m
}

type aidlLanguageModuleProperties struct {
	// Because we add this property struct to regular cc/java modules, move all the props under an
	// "Aidl_internal_props" struct so we're sure they don't accidentally share the same name as a
	// core cc/java property.
	Aidl_internal_props struct {
		Lang              string
		AidlInterfaceName string
		Version           string
		Imports           []string
	}
}
