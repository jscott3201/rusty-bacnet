#!/usr/bin/env python3
"""Check the release artifacts before anything is published (#943, #944, #1472).

    check_artifacts.py DIR --release-version 0.12.0 --glibc 2.17 --python 3.11 3.12 3.13 3.14 \
        --wheel-platform manylinux_2_17_x86_64.manylinux2014_x86_64 macosx_11_0_arm64 win_amd64 \
        --cli bacnet-linux-amd64=linux_x86_64 bacnet-macos-arm64=macosx_11_0_arm64 \
              bacnet-windows-amd64.exe=win_amd64 \
        --notices THIRD-PARTY-NOTICES

DIR holds the wheels, the sdist and the CLI binaries. A platform is a wheel
platform tag; a Linux CLI binary's is linux_<arch>, since --glibc sets its
limit. The script fails unless:
- there is exactly one sdist, and one wheel for each Python and platform, all
  with the release version in PEP 440 form (Cargo's 0.12.0-rc.1 is Python's
  0.12.0rc1);
- with --notices, each wheel has that file in its .dist-info/licenses/ and the
  sdist has it too, byte for byte;
- each wheel's extension module is named for its own Python and platform;
- every binary (each wheel's extension module, each CLI binary) is for its
  platform's architecture, and
  - Linux (ELF: `readelf`, `objdump`): needs no glibc symbol version newer than
    --glibc, and no CLI binary loads libpcap at run time (capture links a
    static libpcap);
  - macOS (Mach-O: `llvm-objdump`): its minimum macOS is the platform tag's;
    an arm64 file carries a code signature; it loads libSystem and may load
    libiconv. A CLI binary loads nothing else and leaves no symbol to a flat
    lookup at load time. An extension module may also load IOKit and
    CoreFoundation, for its serial port listing, and leaves Python's C API
    (`_Py*`), and nothing else, to a flat lookup (`-undefined
    dynamic_lookup`): it binds everything else to the library that has it;
  - Windows (PE: `llvm-readobj`): a console program or a DLL as expected, which
    imports kernel32.dll, otherwise only Windows system DLLs, and an extension
    module also the Universal CRT, VCRUNTIME140.dll, its own Python's
    pythonXY.dll, and SetupAPI and cfgmgr32 for its serial port listing. A CLI binary imports no CRT DLL: it links the C runtime
    statically.

Tool output that parses to nothing fails: the bind tables must list a
libSystem bind, an extension module must have Python flat lookups, and every
PE file must import kernel32.dll.
"""

import argparse
import re
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

GLIBC = re.compile(r"\bGLIBC_(\d+(?:\.\d+)+)\b")
WHEEL = re.compile(r"^rusty_bacnet-(?P<ver>[^-]+)-(?P<py>[^-]+)-(?P<abi>[^-]+)-(?P<plat>[^-]+)\.whl$")
SDIST = re.compile(r"^rusty_bacnet-(?P<ver>[^-]+)\.tar\.gz$")
ARCHES = ("x86_64", "aarch64")
# `readelf -h` names every machine; an x86_64 objdump calls aarch64 "elf64-little".
READELF_MACHINE = {"x86_64": "Advanced Micro Devices X86-64", "aarch64": "AArch64"}
MACHO_CPU = {"x86_64": "X86_64", "aarch64": "ARM64"}
PE_MACHINE = {"x86_64": "IMAGE_FILE_MACHINE_AMD64", "aarch64": "IMAGE_FILE_MACHINE_ARM64"}
# Wheel tags name the architectures their own way.
MACOS_ARCH = {"x86_64": "x86_64", "arm64": "aarch64"}
WINDOWS_ARCH = {"amd64": "x86_64", "arm64": "aarch64"}
MACOS_SYSTEM = {"/usr/lib/libSystem.B.dylib", "/usr/lib/libiconv.2.dylib"}
IOKIT = "/System/Library/Frameworks/IOKit.framework/Versions/A/IOKit"
COREFOUNDATION = "/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"
# Also allowed in an extension module, not in the CLI: serialport's port
# listing (MS/TP), which list_serial_ports() uses.
MACOS_EXTENSION_FRAMEWORKS = {IOKIT, COREFOUNDATION}
# Python's C API, which the interpreter provides: an extension module leaves it
# to a flat lookup.
PYTHON_SYMBOL = re.compile(r"^__?Py")
MACHO_LOAD_DYLIB = {"LC_LOAD_DYLIB", "LC_LOAD_WEAK_DYLIB", "LC_REEXPORT_DYLIB", "LC_LAZY_LOAD_DYLIB",
                    "LC_LOAD_UPWARD_DYLIB"}
# Windows' own DLLs that the binaries import, and the API sets that map onto
# them. A new one fails the check, so a new run-time dependency is noticed.
WINDOWS_SYSTEM = {"kernel32.dll", "ntdll.dll", "user32.dll", "advapi32.dll", "ws2_32.dll", "iphlpapi.dll",
                  "bcryptprimitives.dll",
                  # The CLI's TUI finds its config folder through `directories`
                  # (SHGetKnownFolderPath, then CoTaskMemFree), #974.
                  "shell32.dll", "combase.dll"}
# Also allowed in an extension module, not in the CLI: serialport's port
# listing, which list_serial_ports() uses (#951), as the macOS extension
# modules alone may load IOKit and CoreFoundation.
WINDOWS_EXTENSION_SYSTEM = {"setupapi.dll", "cfgmgr32.dll"}
WINDOWS_API_SET = re.compile(r"^api-ms-win-core-[a-z0-9-]+\.dll$")
# The C runtime an extension module shares with Python, which ships VCRUNTIME140.dll
# and relies on the Universal CRT that Windows 10 and later include.
WINDOWS_CRT = re.compile(r"^(api-ms-win-crt-[a-z0-9-]+\.dll|vcruntime140\.dll)$")
SEMVER = re.compile(r"^(\d+\.\d+\.\d+)(?:-([0-9A-Za-z.-]+))?$")
PRE_RELEASE = {"alpha": "a", "a": "a", "beta": "b", "b": "b", "rc": "rc", "c": "rc", "pre": "rc",
               "preview": "rc", "dev": ".dev"}


def pep440(cargo_version):
    """The Python version maturin gives a Cargo version: 1.2.3-rc.1 is 1.2.3rc1."""
    m = SEMVER.match(cargo_version)
    if not m:
        raise ValueError(f"can't map Cargo version {cargo_version!r} to PEP 440")
    release, pre = m.groups()
    if pre is None:
        return release
    kind, _, number = pre.partition(".")
    if kind.lower() not in PRE_RELEASE or not (number or "0").isdigit():
        raise ValueError(f"can't map Cargo pre-release {pre!r} to PEP 440")
    return f"{release}{PRE_RELEASE[kind.lower()]}{int(number or 0)}"


def version_tuple(text):
    return tuple(int(part) for part in text.split("."))


def shown(version):
    return ".".join(map(str, version)) if version else "none"


def parse_platform(tag):
    """(os, arch, minimum macOS or None) for a platform tag; ValueError if unknown."""
    m = (re.fullmatch(r"manylinux_\d+_\d+_(\w+?)(?:\.manylinux\w+)?", tag)
         or re.fullmatch(r"linux_(\w+)", tag))
    if m and m.group(1) in ARCHES:
        return "linux", m.group(1), None
    m = re.fullmatch(r"macosx_(\d+)_(\d+)_(\w+)", tag)
    if m and m.group(3) in MACOS_ARCH:
        return "macos", MACOS_ARCH[m.group(3)], (int(m.group(1)), int(m.group(2)))
    m = re.fullmatch(r"win_(\w+)", tag)
    if m and m.group(1) in WINDOWS_ARCH:
        return "windows", WINDOWS_ARCH[m.group(1)], None
    raise ValueError(f"unknown platform {tag!r}")


def max_glibc(objdump_text):
    """The highest GLIBC_x.y version named in `objdump -T` output, or None."""
    found = [version_tuple(v) for v in GLIBC.findall(objdump_text)]
    return max(found) if found else None


def expected_wheel_tags(pythons, platforms):
    """(python tag, abi tag, platform tag) for each wheel."""
    return {("cp" + py.replace(".", ""),) * 2 + (plat,) for py in pythons for plat in platforms}


def check_wheel_set(names, pythons, platforms, version):
    """Return (errors, wheels) for the file names in the artifact dir."""
    errors = []
    sdists = [m for m in map(SDIST.match, names) if m]
    if len(sdists) != 1:
        errors.append(f"expected one rusty_bacnet sdist, found {len(sdists)}")
    for m in sdists:
        if m["ver"] != version:
            errors.append(f"{m.string} has version {m['ver']}, the release {version}")
    wheels = {}
    for name in names:
        if not name.endswith(".whl"):
            continue
        m = WHEEL.match(name)
        if not m:
            errors.append(f"unexpected wheel name {name}")
            continue
        if m["ver"] != version:
            errors.append(f"{name} has version {m['ver']}, the release {version}")
        wheels[(m["py"], m["abi"], m["plat"])] = name
    want = expected_wheel_tags(pythons, platforms)
    for tag in sorted(want - wheels.keys()):
        errors.append("missing wheel for tag " + "-".join(tag))
    for tag in sorted(wheels.keys() - want):
        errors.append(f"unexpected wheel {wheels[tag]}")
    return errors, {tag: name for tag, name in wheels.items() if tag in want}


def extension_suffix(py_tag, platform):
    """The extension module suffix CPython uses for this Python tag and platform tag."""
    system, arch, _ = parse_platform(platform)
    if system == "linux":
        return f".cpython-{py_tag[2:]}-{arch}-linux-gnu.so"
    if system == "macos":
        return f".cpython-{py_tag[2:]}-darwin.so"
    return f".{py_tag}-{platform}.pyd"


def run(tool, path, *flags):
    return subprocess.run([tool, *flags, str(path)], check=True, capture_output=True, text=True).stdout


def elf_machine(readelf_header):
    """The Machine field of `readelf -h` output."""
    m = re.search(r"^\s*Machine:\s*(.+?)\s*$", readelf_header, re.MULTILINE)
    return m.group(1) if m else None


def needed_libraries(readelf_dynamic):
    """The NEEDED entries of `readelf -d` output."""
    return re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", readelf_dynamic)


def macho_headers(text):
    """{cpu, platform, minos, dylibs, chained, signed} from `llvm-objdump --macho
    --private-headers` output: the CPU type, the build platform and minimum OS
    version, the libraries it loads, and whether it uses chained fixups and
    carries a code signature (LC_CODE_SIGNATURE)."""
    cpus = re.findall(r"^\s*MH_MAGIC(?:_64)?\s+(\S+)", text, re.MULTILINE)
    info = {"cpu": cpus[0] if len(cpus) == 1 else None, "platform": None, "minos": None, "dylibs": [],
            "chained": False, "signed": False}
    for block in re.split(r"^Load command \d+\s*$", text, flags=re.MULTILINE)[1:]:
        fields = dict(re.findall(r"^\s*(\S+(?: version)?) (.+?)\s*$", block, re.MULTILINE))
        cmd = fields.get("cmd")
        if cmd in MACHO_LOAD_DYLIB:
            info["dylibs"].append(re.sub(r" \(offset \d+\)$", "", fields.get("name", "")))
        elif cmd == "LC_BUILD_VERSION":
            info["platform"], info["minos"] = fields.get("platform"), fields.get("minos")
        elif cmd == "LC_VERSION_MIN_MACOSX":
            info["platform"], info["minos"] = "macos", fields.get("version")
        elif cmd == "LC_DYLD_CHAINED_FIXUPS":
            info["chained"] = True
        elif cmd == "LC_CODE_SIGNATURE":
            info["signed"] = True
    return info


def flat_lookups(bind_text):
    """The symbols `llvm-objdump --macho --bind --lazy-bind --weak-bind` lists
    as bound by flat lookup."""
    return sorted({line.split()[-1] for line in bind_text.splitlines() if " flat-namespace " in line})


def binds_parsed(bind_text):
    """Whether the bind output has its table and at least one libSystem bind,
    which every binary here has. Otherwise its format changed, and an empty
    list of flat lookups would prove nothing."""
    return "Bind table:" in bind_text and any("libSystem" in line.split() for line in bind_text.splitlines())


def check_macho(path, label, arch, minos, extension):
    errors = []
    info = macho_headers(run("llvm-objdump", path, "--macho", "--private-headers"))
    if info["cpu"] != MACHO_CPU[arch]:
        errors.append(f"{label} is a {info['cpu']} Mach-O, not {arch}")
    if info["platform"] != "macos" or info["minos"] is None:
        errors.append(f"{label} names no minimum macOS (platform {info['platform']})")
    elif version_tuple(info["minos"]) != minos:
        errors.append(f"{label} needs macOS {info['minos']}, its platform tag says {shown(minos)}")
    if arch == "aarch64" and not info["signed"]:
        errors.append(f"{label} has no code signature, which macOS requires on arm64")
    allowed = MACOS_SYSTEM | (MACOS_EXTENSION_FRAMEWORKS if extension else set())
    print(f"{label}: macOS {info['minos']}, signed {info['signed']}, loads {', '.join(info['dylibs'])}")
    for dylib in sorted(set(info["dylibs"]) - allowed):
        errors.append(f"{label} loads {dylib}, which the release doesn't expect")
    if "/usr/lib/libSystem.B.dylib" not in info["dylibs"]:
        errors.append(f"{label} doesn't load libSystem")
    if info["chained"]:
        errors.append(f"{label} uses chained fixups, whose flat lookups this script can't list; update it")
        return errors
    binds = run("llvm-objdump", path, "--macho", "--bind", "--lazy-bind", "--weak-bind")
    if not binds_parsed(binds):
        errors.append(f"{label}: llvm-objdump's bind output has no bind table or libSystem bind; check its format")
        return errors
    return errors + check_flat_lookups(label, flat_lookups(binds), extension)


def check_flat_lookups(label, flat, extension):
    """Errors for the symbols a Mach-O file leaves to a flat lookup: none for a
    CLI binary; for an extension module, Python's C API, which must be there,
    and nothing else."""
    if not extension:
        return [f"{label} leaves {len(flat)} symbols to a flat lookup, such as {', '.join(flat[:5])}"] if flat else []
    errors = []
    if not any(PYTHON_SYMBOL.match(s) for s in flat):
        errors.append(f"{label} leaves no Python symbol to a flat lookup; the bind output didn't parse")
    others = [s for s in flat if not PYTHON_SYMBOL.match(s)]
    if others:
        errors.append(f"{label} leaves symbols other than Python's to a flat lookup ({len(others)}):"
                      f" {', '.join(others[:5])}")
    return errors


def pe_headers(text):
    """{machine, dll, subsystem, imports} from `llvm-readobj --file-headers --coff-imports`."""
    machine = re.search(r"^\s*Machine:\s*(\S+)", text, re.MULTILINE)
    subsystem = re.search(r"^\s*Subsystem:\s*(\S+)", text, re.MULTILINE)
    imports = re.findall(r"^(?:Delay)?Import \{\s*\n\s*Name:\s*(\S+)", text, re.MULTILINE)
    return {"machine": machine.group(1) if machine else None, "dll": "IMAGE_FILE_DLL " in text,
            "subsystem": subsystem.group(1) if subsystem else None, "imports": imports}


def check_pe(path, label, arch, python_dll=None):
    """python_dll: the pythonXY.dll an extension module must import; None for a CLI binary."""
    errors = []
    info = pe_headers(run("llvm-readobj", path, "--file-headers", "--coff-imports"))
    if info["machine"] != PE_MACHINE[arch]:
        errors.append(f"{label} is a {info['machine']} PE file, not {arch}")
    if python_dll and not info["dll"]:
        errors.append(f"{label} is not a DLL")
    if not python_dll and (info["dll"] or info["subsystem"] != "IMAGE_SUBSYSTEM_WINDOWS_CUI"):
        errors.append(f"{label} is not a console program (subsystem {info['subsystem']})")
    imports = sorted({name.lower() for name in info["imports"]})
    print(f"{label}: imports {', '.join(imports)}")
    if "kernel32.dll" not in imports:
        errors.append(f"{label} doesn't import kernel32.dll; llvm-readobj's import list didn't parse")
    pythons = [name for name in imports if re.fullmatch(r"python\d*\.dll", name)]
    if pythons != ([python_dll] if python_dll else []):
        errors.append(f"{label} imports {pythons or 'no Python DLL'}, expected {python_dll or 'none'}")
    allowed = WINDOWS_SYSTEM | (WINDOWS_EXTENSION_SYSTEM if python_dll else set())
    for name in imports:
        if name in allowed or WINDOWS_API_SET.match(name) or name in pythons:
            continue
        if WINDOWS_CRT.match(name):
            if not python_dll:
                errors.append(f"{label} imports the C runtime DLL {name}; the CLI links the CRT statically")
            continue
        errors.append(f"{label} imports {name}, which the release doesn't expect")
    return errors


def check_elf(path, label, glibc_limit, arch):
    errors = []
    machine = elf_machine(run("readelf", path, "-h"))
    if machine != READELF_MACHINE[arch]:
        errors.append(f"{label} is a {machine} ELF, not {arch}")
    needed = max_glibc(run("objdump", path, "-T"))
    print(f"{label}: needs glibc {shown(needed)}")
    if needed is None or needed > glibc_limit:
        errors.append(f"{label} needs glibc {shown(needed)}, above {shown(glibc_limit)}")
    return errors


def check_binary(path, label, platform, glibc_limit, py_tag=None):
    """Errors for one extension module (py_tag set) or CLI binary on a platform."""
    system, arch, minos = parse_platform(platform)
    if system == "macos":
        return check_macho(path, label, arch, minos, extension=py_tag is not None)
    if system == "windows":
        return check_pe(path, label, arch, f"python{py_tag[2:]}.dll" if py_tag else None)
    errors = check_elf(path, label, glibc_limit, arch)
    if py_tag is None:
        needed_libs = needed_libraries(run("readelf", path, "-d"))
        print(f"{label}: loads {', '.join(needed_libs)}")
        if any("pcap" in lib for lib in needed_libs):
            errors.append(f"{label} loads libpcap dynamically; the release links it statically")
    return errors


def check_notices(directory, wheels, version, notices):
    """Errors for wheels or an sdist whose THIRD-PARTY-NOTICES is missing or differs."""
    errors = []
    want = notices.read_bytes()
    member = f"rusty_bacnet-{version}.dist-info/licenses/{notices.name}"
    for name in sorted(wheels.values()):
        with zipfile.ZipFile(directory / name) as whl:
            if member not in whl.namelist():
                errors.append(f"{name} lacks {member}")
            elif whl.read(member) != want:
                errors.append(f"{name}'s {member} differs from {notices}")
    sdist = directory / f"rusty_bacnet-{version}.tar.gz"
    if sdist.is_file():
        with tarfile.open(sdist) as tar:
            found = [m for m in tar.getmembers() if m.isfile() and m.name.endswith("/" + notices.name)]
            if len(found) != 1:
                errors.append(f"{sdist.name} has {len(found)} copies of {notices.name}, expected one")
            elif tar.extractfile(found[0]).read() != want:
                errors.append(f"{sdist.name}'s {found[0].name} differs from {notices}")
    return errors


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("dir", type=Path)
    parser.add_argument("--release-version", required=True, help="the workspace version, e.g. 0.12.0")
    parser.add_argument("--notices", type=Path, help="THIRD-PARTY-NOTICES every wheel and the sdist must carry")
    parser.add_argument("--glibc", required=True, help="highest glibc version allowed, e.g. 2.17")
    parser.add_argument("--python", nargs="+", required=True, help="CPython versions, e.g. 3.11")
    parser.add_argument("--wheel-platform", nargs="+", required=True, help="wheel platform tags")
    parser.add_argument("--cli", nargs="+", default=[], help="NAME=PLATFORM for each CLI binary")
    args = parser.parse_args(argv)
    for platform in args.wheel_platform + [spec.partition("=")[2] for spec in args.cli]:
        try:
            parse_platform(platform)
        except ValueError as err:
            parser.error(str(err))
    limit = version_tuple(args.glibc)
    try:
        version = pep440(args.release_version)
    except ValueError as err:
        parser.error(str(err))

    names = sorted(p.name for p in args.dir.iterdir() if p.is_file())
    errors, wheels = check_wheel_set(names, args.python, args.wheel_platform, version)
    print(f"Python package version: {version}")
    if args.notices:
        errors += check_notices(args.dir, wheels, version, args.notices)
    with tempfile.TemporaryDirectory() as tmp:
        for (py, _abi, plat), name in sorted(wheels.items()):
            with zipfile.ZipFile(args.dir / name) as whl:
                modules = [n for n in whl.namelist() if n.endswith((".so", ".pyd"))]
                want = extension_suffix(py, plat)
                if len(modules) != 1 or not modules[0].endswith(want):
                    errors.append(f"{name}: extension modules {modules}, expected one ending {want}")
                    continue
                extracted = Path(whl.extract(modules[0], Path(tmp, name)))
            errors += check_binary(extracted, f"{name} ({modules[0]})", plat, limit, py_tag=py)
    for spec in args.cli:
        cli_name, _, platform = spec.partition("=")
        path = args.dir / cli_name
        if not path.is_file():
            errors.append(f"missing CLI binary {cli_name}")
            continue
        errors += check_binary(path, cli_name, platform, limit)

    for err in errors:
        print(f"error: {err}", file=sys.stderr)
    if errors:
        return 1
    notices = f", {args.notices.name} in each" if args.notices else ""
    print(f"OK: {len(wheels)} wheels, one sdist, {len(args.cli)} CLI binaries, glibc <= {args.glibc}{notices}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
