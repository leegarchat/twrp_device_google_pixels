// install-desktop.cpp — Windows (x64) GUI-friendly launcher for
// `bootsmasher install`. Does what install-desktop.bat does, as a real
// .exe: double-click it, or drag-and-drop a recovery cpio (.lz4) onto it.
//
// Behavior (mirrors install-desktop.bat one to one):
//   - finds the installer binary next to itself:
//       bin\windows\install[-small]-windows-<arch>.exe (full wins),
//     then bootsmasher[.]exe legacy spots, then PATH.
//   - pins --export to export.txt next to itself unless the caller
//     passed --export already.
//   - drag-and-drop: when the FIRST argument is an existing file (not a
//     flag), it is consumed as the recovery payload and forwarded as
//     --recovery-img (overrides export.txt RECOVERY_IMG for this run,
//     no editing needed).
//   - no payload at all (nothing dropped, export.txt entry missing) ->
//     a system file picker (GetOpenFileDialog) asks for it, unless the
//     run is scripted (--force) or a non-payload flow (--file/-h/--help
//     or an explicit --recovery-img).
//   - UTF-8 console, forwards everything else unchanged, prints the
//     menu hint on interactive runs, waits for Enter at the end
//     (except --force / -h / --help) so a double-clicked window never
//     vanishes before the result is read, exits with the installer's
//     code.
//
// Build (any one line; no third-party deps, Win32 API only):
//   MSVC (x64 Native Tools prompt):
//     cl /O2 /EHsc install-desktop.cpp /Feinstall-desktop.exe
//   MinGW cross from Linux:
//     x86_64-w64-mingw32-g++ -O2 -mconsole -municode -static
//       -static-libgcc -static-libstdc++ -o install-desktop.exe install-desktop.cpp -lshell32 -lcomdlg32
// Drop the built install-desktop.exe next to install-desktop.bat
// (same folder: export.txt + bin\ next to it) and run / double-click
// / drop a payload onto it.

#ifndef UNICODE
#define UNICODE
#endif
#ifndef _UNICODE
#define _UNICODE
#endif

#include <windows.h>
#include <shellapi.h>
#include <commdlg.h>
#include <stdio.h>
#include <string>
#include <vector>

// Quote one argv token the way CommandLineToArgvW parses it back.
static std::wstring quote_arg(const std::wstring &a) {
    if (a.empty())
        return L"\"\"";
    bool need = false;
    for (wchar_t c : a) {
        if (c == L' ' || c == L'\t' || c == L'"' || c == L'\n' || c == L'\v') {
            need = true;
            break;
        }
    }
    if (!need)
        return a;
    std::wstring out = L"\"";
    size_t bs = 0;
    for (wchar_t c : a) {
        if (c == L'\\') {
            bs++;
            continue;
        }
        if (c == L'"') {
            out.append(bs * 2 + 1, L'\\');
            out += L'"';
            bs = 0;
            continue;
        }
        if (bs) {
            out.append(bs, L'\\');
            bs = 0;
        }
        out += c;
    }
    if (bs)
        out.append(bs * 2, L'\\');
    out += L'"';
    return out;
}

static bool exact_in(const std::vector<std::wstring> &args, const wchar_t *t) {
    for (const auto &a : args) {
        if (a == t)
            return true;
    }
    return false;
}

// True when the caller already pinned an export file.
static bool has_export(const std::vector<std::wstring> &args) {
    for (size_t i = 0; i < args.size(); i++) {
        if (args[i] == L"--export")
            return true; // value rides in args[i+1], presence is enough
        if (args[i].compare(0, 9, L"--export=") == 0)
            return true;
    }
    return false;
}

static bool file_exists_not_dir(const std::wstring &p) {
    DWORD a = GetFileAttributesW(p.c_str());
    return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

static std::wstring trim_ws(const std::wstring &s) {
    size_t b = 0, e = s.size();
    while (b < e && (s[b] == L' ' || s[b] == L'\t' || s[b] == L'\r' || s[b] == L'\n'))
        b++;
    while (e > b && (s[e - 1] == L' ' || s[e - 1] == L'\t' || s[e - 1] == L'\r' || s[e - 1] == L'\n'))
        e--;
    return s.substr(b, e - b);
}

// Last RECOVERY_IMG value from an export.txt file (Rust parse_export
// semantics: last KEY= wins, quotes stripped), resolved against the
// export file's own directory. Empty when absent/unreadable.
static std::wstring export_payload(const std::wstring &export_file) {
    FILE *f = _wfopen(export_file.c_str(), L"r");
    if (!f)
        return L"";
    std::wstring value;
    wchar_t line[32768];
    while (fgetws(line, 32768, f)) {
        std::wstring t = trim_ws(line);
        if (t.empty() || t[0] == L'#')
            continue;
        size_t eq = t.find(L'=');
        if (eq == std::wstring::npos)
            continue;
        if (trim_ws(t.substr(0, eq)) != L"RECOVERY_IMG")
            continue;
        value = trim_ws(t.substr(eq + 1));
    }
    fclose(f);
    if (value.size() >= 2 &&
        ((value.front() == L'"' && value.back() == L'"') ||
         (value.front() == L'\'' && value.back() == L'\'')))
        value = value.substr(1, value.size() - 2);
    if (value.empty())
        return L"";
    if ((value.size() > 1 && value[1] == L':') ||
        (value.size() > 1 && value[0] == L'\\' && value[1] == L'\\'))
        return value; // absolute (drive or UNC)
    size_t bs = export_file.find_last_of(L"\\/");
    std::wstring dir = (bs == std::wstring::npos) ? L"." : export_file.substr(0, bs);
    return dir + L"\\" + value;
}

// System file picker (native dialog, no console needed).
static std::wstring open_dialog(const std::wstring &export_file) {
    size_t bs = export_file.find_last_of(L"\\/");
    std::wstring dir = (bs == std::wstring::npos) ? L"." : export_file.substr(0, bs);
    static const wchar_t kFilter[] =
        L"Recovery payload (*.lz4;*.cpio;*.img)\0*.lz4;*.cpio;*.img\0All files (*.*)\0*.*\0";
    wchar_t file[32768] = L"";
    OPENFILENAMEW ofn = {};
    ofn.lStructSize = sizeof(ofn);
    ofn.lpstrFilter = kFilter;
    ofn.nFilterIndex = 1;
    ofn.lpstrFile = file;
    ofn.nMaxFile = 32768;
    ofn.lpstrInitialDir = dir.c_str();
    ofn.lpstrTitle = L"OrangeFox recovery payload";
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if (GetOpenFileNameW(&ofn))
        return std::wstring(file);
    return L"";
}

static std::wstring arch_name() {
    SYSTEM_INFO si;
    GetNativeSystemInfo(&si);
    if (si.wProcessorArchitecture == PROCESSOR_ARCHITECTURE_ARM64)
        return L"arm64";
    return L"x86_64"; // AMD64 and anything else: x86_64 lookup first
}

static bool try_bin(const std::wstring &p) {
    DWORD a = GetFileAttributesW(p.c_str());
    return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

int wmain() {
    // Own directory (this .exe lives next to export.txt + bin\).
    wchar_t self[MAX_PATH];
    DWORD n = GetModuleFileNameW(NULL, self, MAX_PATH);
    if (n == 0 || n >= MAX_PATH) {
        fwprintf(stderr, L"cannot locate own path\n");
        return 1;
    }
    std::wstring root(self, n);
    size_t bs = root.find_last_of(L"\\/");
    root = (bs == std::wstring::npos) ? L"." : root.substr(0, bs);

    std::wstring arch = arch_name();

    // Binary search, same order as install-desktop.bat.
    std::vector<std::wstring> probes;
    probes.push_back(root + L"\\bin\\windows\\install-windows-" + arch + L".exe");
    probes.push_back(root + L"\\bin\\windows\\install-small-windows-" + arch + L".exe");
    probes.push_back(root + L"\\bootsmasher.exe");
    probes.push_back(root + L"\\bootsmasher-x86_64.exe");
    probes.push_back(root + L"\\..\\target\\release\\bootsmasher.exe");
    probes.push_back(root + L"\\..\\target\\release\\bootsmasher-x86_64.exe");
    std::wstring bin;
    for (const auto &p : probes) {
        if (try_bin(p)) {
            bin = p;
            break;
        }
    }
    if (bin.empty()) {
        wchar_t on_path[MAX_PATH];
        if (SearchPathW(NULL, L"bootsmasher.exe", NULL, MAX_PATH, on_path, NULL))
            bin = on_path;
    }

    // Caller args (argv[0] skipped; CommandLineToArgvW already unquotes).
    int argc = 0;
    LPWSTR *argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    std::vector<std::wstring> args;
    for (int i = 1; i < argc; i++)
        args.push_back(argv[i]);
    LocalFree(argv);

    bool pause = !exact_in(args, L"--force") && !exact_in(args, L"-h") &&
                 !exact_in(args, L"--help");
    bool interactive = pause && !exact_in(args, L"--file");

    if (bin.empty()) {
        fwprintf(stderr,
                 L"no installer binary (expected bin\\windows\\install[-small]-windows-<arch>.exe next to this .exe)\n"
                 L"-- diagnostic: arch=%ls root=%ls\n",
                 arch.c_str(), root.c_str());
        if (pause) {
            fwprintf(stdout, L"\nPress Enter to exit... ");
            fflush(stdout);
            wchar_t b[8];
            fgetws(b, 8, stdin);
        }
        return 1;
    }

    // Drag-and-drop payload: first arg as an existing non-flag file.
    std::wstring drop;
    if (!args.empty() && !args[0].empty() && args[0][0] != L'-' &&
        file_exists_not_dir(args[0])) {
        wchar_t full[32768];
        DWORD fl = GetFullPathNameW(args[0].c_str(), 32768, full, NULL);
        drop = (fl > 0 && fl < 32768) ? std::wstring(full) : args[0];
        args.erase(args.begin());
        fwprintf(stdout, L"Payload (dropped file): %ls\n", drop.c_str());
    }

    // System file picker: no drop, no explicit --recovery-img, not a
    // scripted/non-payload flow, and the export.txt payload is missing.
    // A cancelled dialog proceeds and the binary reports its clear error.
    bool need_pick = drop.empty();
    for (const auto &a : args) {
        if (a == L"--recovery-img" || a.compare(0, 15, L"--recovery-img=") == 0 ||
            a == L"--force" || a == L"-h" || a == L"--help" || a == L"--file") {
            need_pick = false;
            break;
        }
    }
    if (need_pick) {
        std::wstring export_file = root + L"\\export.txt";
        for (size_t i = 0; i < args.size(); i++) {
            if (args[i] == L"--export" && i + 1 < args.size()) {
                export_file = args[i + 1];
                break;
            }
            if (args[i].compare(0, 9, L"--export=") == 0) {
                export_file = args[i].substr(9);
                break;
            }
        }
        if (!file_exists_not_dir(export_payload(export_file))) {
            std::wstring picked = open_dialog(export_file);
            if (!picked.empty()) {
                drop = picked;
                fwprintf(stdout, L"Payload (file picker): %ls\n", drop.c_str());
            }
        }
    }

    if (interactive) {
        fwprintf(stdout,
                 L"Arrow-key menus: Up/Down to move, Enter to choose, q/Esc to exit.\n\n");
    }

    // argv for the child: install [--export P] [--recovery-img D] rest...
    std::wstring cmd = quote_arg(bin) + L" install";
    if (!has_export(args))
        cmd += L" --export " + quote_arg(root + L"\\export.txt");
    if (!drop.empty())
        cmd += L" --recovery-img " + quote_arg(drop);
    for (const auto &a : args)
        cmd += L" " + quote_arg(a);

    SetConsoleOutputCP(CP_UTF8);

    STARTUPINFOW si_start = {};
    si_start.cb = sizeof(si_start);
    PROCESS_INFORMATION pi = {};
    // CreateProcessW may write into the buffer: pass a mutable copy.
    std::wstring mutable_cmd = cmd;
    if (!CreateProcessW(NULL, &mutable_cmd[0], NULL, NULL, TRUE, 0, NULL, NULL,
                        &si_start, &pi)) {
        fwprintf(stderr, L"cannot start installer (error %lu)\n", GetLastError());
        if (pause) {
            fwprintf(stdout, L"\nPress Enter to exit... ");
            fflush(stdout);
            wchar_t b[8];
            fgetws(b, 8, stdin);
        }
        return 1;
    }
    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD code = 1;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);

    if (pause) {
        fwprintf(stdout, L"\nPress Enter to exit... ");
        fflush(stdout);
        wchar_t b[8];
        fgetws(b, 8, stdin);
    }
    return (int)code;
}
