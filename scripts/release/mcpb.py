#!/usr/bin/env python3
"""MCP bundles (.mcpb) of the release CLI's MCP server, for Claude Desktop (#2304).

    mcpb.py pack --binary out/bacnet-macos-arm64 --platform macos-arm64 --version 0.16.0 \\
        --license LICENSE --notices THIRD-PARTY-NOTICES --out out
    mcpb.py check out/bacnet-macos-arm64.mcpb --binary out/bacnet-macos-arm64 --platform macos-arm64 \\
        --version 0.16.0 [--notices THIRD-PARTY-NOTICES]
    mcpb.py smoke out/bacnet-macos-arm64.mcpb

The format is the MCP Bundle manifest 0.3 (github.com/modelcontextprotocol/mcpb,
MANIFEST.md, "Current version: 0.3"; the schema of @anthropic-ai/mcpb 2.1.2,
which 0.4 only extends with the `uv` server type). A bundle is a zip holding
`manifest.json` at its root. Each one here is a `binary` bundle for one
release platform: the release's own `bacnet` executable, byte for byte, as
`server/bacnet` (`server/bacnet.exe` on Windows), the license and the
third-party notices, and a manifest that runs `bacnet mcp serve --stdio
--profile <profile>` with the profile the user names in the host's settings.
Its `compatibility.platforms` names the bundle's one OS; the format has no
field for the architecture, so the file name says it.

- `pack` writes `<cli asset name without .exe>.mcpb` from `mcpb/manifest.json`,
  reproducibly: entries in a fixed order, the SOURCE_DATE_EPOCH time (or
  1980-01-01), Unix modes (755 for the executable, 644 otherwise).
- `check` checks a bundle statically: its entries, the manifest against the
  0.3 schema (`SCHEMA`, a transcription) plus what `mcpb validate` adds (the
  entry point is there and executable, and `mcp_config` uses only the
  variables a host substitutes), its version and platform, and that its
  executable is the CLI asset itself.
- `smoke` does what a host does: unzips it (restoring the modes), runs the
  manifest's command with a profile filled in, and checks that a missing or
  unauthorized profile stops the server at once with a message naming
  `bacnet mcp authorize`, and that the authorized read-only profile
  `mcpb/smoke-profile.toml` serves `initialize` and `tools/list` with only
  read tools: the manifest's list, each annotated read-only.
"""

import argparse
import json
import os
import queue
import re
import stat
import subprocess
import sys
import tempfile
import threading
import time
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
TEMPLATE = HERE / "mcpb" / "manifest.json"
SMOKE_PROFILE = HERE / "mcpb" / "smoke-profile.toml"
MANIFEST_VERSION = "0.3"
# Release platform (the build matrix's name): the manifest's OS and the
# executable's suffix.
PLATFORMS = {
    "linux-x86_64": ("linux", ""),
    "linux-aarch64": ("linux", ""),
    "macos-x86_64": ("darwin", ""),
    "macos-arm64": ("darwin", ""),
    "windows-x86_64": ("win32", ".exe"),
}
LICENSE = "LICENSE"
NOTICES = "THIRD-PARTY-NOTICES"
SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")
# The variables `mcpb validate` accepts in mcp_config (src/node/validate.ts).
VARIABLE = re.compile(r"\$\{([^}]*)\}")
HOST_VARIABLES = {"__dirname", "pathSeparator", "/"}
WRITE_TOOLS = {"bacnet_write", "bacnet_revert"}


# --- The 0.3 manifest schema ------------------------------------------------
# A transcription of mcpb-manifest-v0.3.schema.json: every object is closed
# (additionalProperties false) unless `extra` gives the schema of the others.

def _string(**rules):
    return {"type": "string", **rules}


def _obj(props, required=(), extra=False):
    return {"type": "object", "props": props, "required": tuple(required), "extra": extra}


def _array(items):
    return {"type": "array", "items": items}


_STR = _string()
_URI = _string(uri=True)
_COMMAND = {"command": _STR, "args": _array(_STR), "env": _obj({}, extra=_STR)}
SCHEMA = _obj({
    "$schema": _STR,
    "dxt_version": _string(const=MANIFEST_VERSION),
    "manifest_version": _string(const=MANIFEST_VERSION),
    "name": _STR,
    "display_name": _STR,
    "version": _STR,
    "description": _STR,
    "long_description": _STR,
    "author": _obj({"name": _STR, "email": _STR, "url": _URI}, ["name"]),
    "repository": _obj({"type": _STR, "url": _URI}, ["type", "url"]),
    "homepage": _URI,
    "documentation": _URI,
    "support": _URI,
    "icon": _STR,
    "icons": _array(_obj({"src": _STR, "size": _string(pattern=r"^\d+x\d+$"), "theme": _string(min=1)},
                         ["src", "size"])),
    "screenshots": _array(_STR),
    "localization": _obj({"resources": _string(pattern=r"\$\{locale\}"),
                          "default_locale": _string(pattern=r"^[A-Za-z0-9]{2,8}(?:-[A-Za-z0-9]{1,8})*$")},
                         ["resources", "default_locale"]),
    "server": _obj({
        "type": _string(enum=("python", "node", "binary")),
        "entry_point": _STR,
        "mcp_config": _obj({**_COMMAND, "platform_overrides": _obj({}, extra=_obj(_COMMAND))}, ["command"]),
    }, ["type", "entry_point", "mcp_config"]),
    "tools": _array(_obj({"name": _STR, "description": _STR}, ["name"])),
    "tools_generated": {"type": "boolean"},
    "prompts": _array(_obj({"name": _STR, "description": _STR, "arguments": _array(_STR), "text": _STR},
                           ["name", "text"])),
    "prompts_generated": {"type": "boolean"},
    "keywords": _array(_STR),
    "license": _STR,
    "privacy_policies": _array(_URI),
    "compatibility": _obj({
        "claude_desktop": _STR,
        "platforms": _array(_string(enum=("darwin", "win32", "linux"))),
        "runtimes": _obj({"python": _STR, "node": _STR}),
    }),
    "user_config": _obj({}, extra=_obj({
        "type": _string(enum=("string", "number", "boolean", "directory", "file")),
        "title": _STR,
        "description": _STR,
        "required": {"type": "boolean"},
        "default": {"any": ({"type": "string"}, {"type": "number"}, {"type": "boolean"}, _array(_STR))},
        "multiple": {"type": "boolean"},
        "sensitive": {"type": "boolean"},
        "min": {"type": "number"},
        "max": {"type": "number"},
    }, ["type", "title", "description"])),
    "_meta": _obj({}, extra=_obj({}, extra={})),
}, ["name", "version", "description", "author", "server"])

_TYPES = {
    "string": lambda v: isinstance(v, str),
    "boolean": lambda v: isinstance(v, bool),
    "number": lambda v: isinstance(v, (int, float)) and not isinstance(v, bool),
    "object": lambda v: isinstance(v, dict),
    "array": lambda v: isinstance(v, list),
}


def schema_errors(value, schema=SCHEMA, where="manifest"):
    """Where `value` breaks `schema`, as messages; [] when it conforms."""
    if "any" in schema:
        if all(schema_errors(value, option, where) for option in schema["any"]):
            return [f"{where}: matches none of its allowed types"]
        return []
    kind = schema.get("type")
    if kind and not _TYPES[kind](value):
        return [f"{where}: expected a {kind}, found {type(value).__name__}"]
    errors = []
    if "const" in schema and value != schema["const"]:
        errors.append(f"{where}: must be {schema['const']!r}, not {value!r}")
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{where}: {value!r} is not one of {', '.join(schema['enum'])}")
    if "pattern" in schema and not re.search(schema["pattern"], value):
        errors.append(f"{where}: {value!r} doesn't match {schema['pattern']}")
    if schema.get("min") and len(value) < schema["min"]:
        errors.append(f"{where}: is empty")
    if schema.get("uri") and not re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:\S+$", value):
        errors.append(f"{where}: {value!r} is not a URI")
    if kind == "array":
        for i, item in enumerate(value):
            errors += schema_errors(item, schema["items"], f"{where}[{i}]")
    if kind == "object" and "props" in schema:
        for key in schema["required"]:
            if key not in value:
                errors.append(f"{where}: lacks {key}")
        for key, item in value.items():
            sub = schema["props"].get(key, schema["extra"])
            if sub is False:
                errors.append(f"{where}: unknown field {key}")
            else:
                errors += schema_errors(item, sub, f"{where}.{key}")
    return errors


# --- Pack -------------------------------------------------------------------

def bundle_name(cli_name):
    """bacnet-windows-amd64.exe -> bacnet-windows-amd64.mcpb."""
    return cli_name.removesuffix(".exe") + ".mcpb"


def entry_point(platform):
    return "server/bacnet" + PLATFORMS[platform][1]


def manifest_for(version, platform, template=TEMPLATE):
    """The manifest of `platform`'s bundle at `version`, from the template."""
    base = json.loads(Path(template).read_text(encoding="utf-8"))
    entry = entry_point(platform)
    out = {}
    for key, value in base.items():
        out[key] = value
        if key == "name":
            out["version"] = version
    server = out["server"]
    out["server"] = {"type": server["type"], "entry_point": entry,
                     "mcp_config": {"command": "${__dirname}/" + entry, **server["mcp_config"]}}
    out["compatibility"] = {"platforms": [PLATFORMS[platform][0]]}
    return out


def zip_time():
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH") or 0)
    # A zip can't hold a time before 1980, and keeps it in 2-second steps.
    return time.gmtime(max(epoch, 315532800))[:6]


def _entry(name, mode, when):
    info = zipfile.ZipInfo(name, date_time=when)
    info.create_system = 3  # Unix, so hosts restore the mode
    info.external_attr = (stat.S_IFREG | mode) << 16
    info.compress_type = zipfile.ZIP_DEFLATED
    return info


def pack(binary, platform, version, license_file, notices, out_dir, template=TEMPLATE):
    """Write the bundle; returns its path."""
    if platform not in PLATFORMS:
        raise ValueError(f"unknown platform {platform}; one of {', '.join(PLATFORMS)}")
    if not SEMVER.match(version):
        raise ValueError(f"{version!r} is not a semantic version")
    manifest = manifest_for(version, platform, template)
    errors = schema_errors(manifest)
    if errors:
        raise ValueError("the manifest breaks the 0.3 schema: " + "; ".join(errors))
    when = zip_time()
    path = Path(out_dir) / bundle_name(Path(binary).name)
    path.parent.mkdir(parents=True, exist_ok=True)
    entries = [("manifest.json", 0o644, (json.dumps(manifest, indent=2) + "\n").encode()),
               (manifest["server"]["entry_point"], 0o755, Path(binary).read_bytes()),
               (LICENSE, 0o644, Path(license_file).read_bytes()),
               (NOTICES, 0o644, Path(notices).read_bytes())]
    with zipfile.ZipFile(path, "w") as bundle:
        for name, mode, data in entries:
            bundle.writestr(_entry(name, mode, when), data, compresslevel=9)
    return path


# --- Check ------------------------------------------------------------------

def _mode(info):
    return (info.external_attr >> 16) & 0o777


def _variables(manifest):
    """Errors for variables in mcp_config that a host doesn't substitute."""
    config = manifest["server"]["mcp_config"]
    keys = set(manifest.get("user_config", {}))
    strings = [("command", config.get("command", ""))]
    strings += [(f"args[{i}]", arg) for i, arg in enumerate(config.get("args", []))]
    strings += [(f"env.{k}", v) for k, v in config.get("env", {}).items()]
    errors = []
    for where, text in strings:
        for name in VARIABLE.findall(text):
            known = name in HOST_VARIABLES or (name.startswith("user_config.") and name[12:] in keys)
            if not known:
                errors.append(f"mcp_config.{where} uses ${{{name}}}, which no host substitutes")
    return errors


def check_bundle(path, platform, version, binary=None, notices=None):
    """Errors for one bundle; [] when it is sound. `binary` and `notices`,
    when given, are the files it must hold byte for byte."""
    label = Path(path).name
    try:
        bundle = zipfile.ZipFile(path)
    except (OSError, zipfile.BadZipFile) as err:
        return [f"{label}: not a zip ({err})"]
    with bundle:
        infos = bundle.infolist()
        names = [i.filename for i in infos]
        entry = entry_point(platform)
        want = ["manifest.json", entry, LICENSE, NOTICES]
        errors = []
        if sorted(names) != sorted(want):
            errors.append(f"{label}: holds {names}, expected {want}")
        if len(set(names)) != len(names):
            errors.append(f"{label}: has duplicate entries")
        if "manifest.json" not in names:
            return errors
        try:
            manifest = json.loads(bundle.read("manifest.json"))
        except ValueError as err:
            return errors + [f"{label}: manifest.json is not JSON ({err})"]
        found = schema_errors(manifest)
        errors += [f"{label}: {e}" for e in found]
        if found:
            return errors
        server = manifest["server"]
        if manifest.get("manifest_version") != MANIFEST_VERSION:
            errors.append(f"{label}: manifest_version must be {MANIFEST_VERSION}")
        if manifest["version"] != version:
            errors.append(f"{label}: version {manifest['version']}, the release {version}")
        if manifest.get("compatibility", {}).get("platforms") != [PLATFORMS[platform][0]]:
            errors.append(f"{label}: compatibility.platforms must be [{PLATFORMS[platform][0]!r}]")
        if server["type"] != "binary" or server["entry_point"] != entry:
            errors.append(f"{label}: server must be the binary {entry}")
        if server["mcp_config"]["command"] != "${__dirname}/" + entry:
            errors.append(f"{label}: mcp_config.command must run ${{__dirname}}/{entry}")
        if server["mcp_config"].get("args", [])[:3] != ["mcp", "serve", "--stdio"]:
            errors.append(f"{label}: mcp_config.args must start mcp serve --stdio")
        if not manifest.get("user_config", {}).get("profile", {}).get("required"):
            errors.append(f"{label}: user_config.profile must be required")
        errors += [f"{label}: {e}" for e in _variables(manifest)]
        tools = [t["name"] for t in manifest.get("tools", [])]
        if not tools or set(tools) & WRITE_TOOLS or len(set(tools)) != len(tools):
            errors.append(f"{label}: tools must list the read tools once each, no write tool")
        if entry in names:
            info = bundle.getinfo(entry)
            if info.create_system != 3 or _mode(info) & 0o111 != 0o111:
                errors.append(f"{label}: {entry} is not stored executable (mode {_mode(info):o})")
            if binary is not None and not Path(binary).is_file():
                errors.append(f"{label}: no {binary} to compare {entry} with")
            elif binary is not None and bundle.read(entry) != Path(binary).read_bytes():
                errors.append(f"{label}: {entry} differs from {Path(binary).name}")
        if notices is not None and not Path(notices).is_file():
            errors.append(f"{label}: no {notices} to compare {NOTICES} with")
        elif notices is not None and NOTICES in names and bundle.read(NOTICES) != Path(notices).read_bytes():
            errors.append(f"{label}: {NOTICES} differs from {notices}")
        if LICENSE in names and not bundle.read(LICENSE).strip():
            errors.append(f"{label}: {LICENSE} is empty")
    return errors


# --- Smoke ------------------------------------------------------------------

def extract(path, into):
    """Unzip as a host does, restoring the Unix modes; refuses a path that
    leaves `into`."""
    into = Path(into).resolve()
    with zipfile.ZipFile(path) as bundle:
        for info in bundle.infolist():
            target = (into / info.filename).resolve()
            if into not in target.parents:
                raise ValueError(f"{info.filename} leaves the bundle directory")
            bundle.extract(info, into)
            if info.create_system == 3 and _mode(info):
                os.chmod(target, _mode(info))
    return json.loads((into / "manifest.json").read_text(encoding="utf-8"))


def command(manifest, directory, profile):
    """The host's command line for `profile`, from the manifest."""
    config = manifest["server"]["mcp_config"]
    values = {"__dirname": str(directory), "pathSeparator": os.sep, "/": os.sep,
              "user_config.profile": profile}
    words = [config["command"], *config.get("args", [])]
    out = [VARIABLE.sub(lambda m: values.get(m.group(1), m.group(0)), w) for w in words]
    left = [w for w in out if VARIABLE.search(w)]
    if left:
        raise ValueError(f"unsubstituted variables in {left}")
    return out


class Server:
    """A server process with its stdout lines and stderr read on threads."""

    def __init__(self, argv, env):
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, env=env)
        self.lines = queue.Queue()
        self.stderr = []
        self.readers = [threading.Thread(target=self._pump, args=(self.proc.stdout, self.lines.put), daemon=True),
                        threading.Thread(target=self._pump, args=(self.proc.stderr, self.stderr.append), daemon=True)]
        for reader in self.readers:
            reader.start()

    @staticmethod
    def _pump(stream, sink):
        for line in iter(stream.readline, b""):
            sink(line.decode("utf-8", "replace").rstrip("\r\n"))
        sink(None)

    def send(self, message):
        self.proc.stdin.write((json.dumps(message) + "\n").encode())
        self.proc.stdin.flush()

    def answer(self, request_id, timeout):
        """The response to `request_id`; every stdout line must be JSON-RPC."""
        deadline = time.monotonic() + timeout
        while True:
            try:
                line = self.lines.get(timeout=max(0.0, deadline - time.monotonic()))
            except queue.Empty:
                raise SmokeError(f"no answer to request {request_id} within {timeout} s") from None
            if line is None:
                raise SmokeError(f"the server closed stdout before answering request {request_id}")
            try:
                message = json.loads(line)
            except ValueError:
                raise SmokeError(f"stdout line is not JSON-RPC: {line[:200]}") from None
            if message.get("jsonrpc") != "2.0":
                raise SmokeError(f"stdout line is not JSON-RPC 2.0: {line[:200]}")
            if message.get("id") == request_id:
                return message

    def wait(self, timeout):
        """The exit code, once both streams are read to their end; None (and
        the process killed) if it doesn't exit within `timeout`."""
        try:
            code = self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
            code = None
        for reader in self.readers:
            reader.join(timeout)
        for stream in (self.proc.stdin, self.proc.stdout, self.proc.stderr):
            try:
                stream.close()
            except OSError:
                pass  # stdin, when the server is gone
        return code

    def log(self):
        return "\n".join(line for line in self.stderr if line is not None)


class SmokeError(Exception):
    pass


def _refuses(argv, env, profile, phrase, timeout):
    """The server must stop by itself, with stdin still open, and say why."""
    server = Server(argv, env)
    code = server.wait(timeout)
    log = server.log()
    if code is None:
        raise SmokeError(f"profile {profile}: the server kept running for {timeout} s instead of refusing")
    if code == 0:
        raise SmokeError(f"profile {profile}: the server exited 0 instead of refusing")
    if server.lines.get(timeout=timeout) is not None:
        raise SmokeError(f"profile {profile}: the refusal wrote to stdout")
    for want in (phrase, f"bacnet mcp authorize --profile {profile}"):
        if want not in log:
            raise SmokeError(f"profile {profile}: the refusal doesn't say {want!r}: {log}")
    print(f"profile {profile}: refused at start (exit {code}): {log.strip()}")


def unauthorized(profile_text):
    """The smoke profile without its consent."""
    text = profile_text.split("[mcp.authorized]")[0]
    return "".join(line for line in text.splitlines(True) if not line.startswith("authorized_"))


def smoke(path, timeout=30.0, profile_file=SMOKE_PROFILE):
    """Run the bundle as a host would; raises SmokeError on a failure."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        manifest = extract(path, root / "bundle")
        platforms = manifest.get("compatibility", {}).get("platforms", [])
        if sys.platform not in platforms:
            raise SmokeError(f"{Path(path).name} is for {platforms}, not this {sys.platform}")
        profiles = root / "config" / "profiles"
        profiles.mkdir(parents=True)
        text = Path(profile_file).read_text(encoding="utf-8")
        (profiles / "smoke.toml").write_text(text, encoding="utf-8")
        (profiles / "unauthorized.toml").write_text(unauthorized(text), encoding="utf-8")
        env = {k: v for k, v in os.environ.items() if k != "RUST_LOG"}
        env["BACNET_CONFIG_DIR"] = str(root / "config")
        bundle = root / "bundle"
        _refuses(command(manifest, bundle, "unauthorized"), env, "unauthorized", "not authorized", timeout)
        _refuses(command(manifest, bundle, "missing"), env, "missing", "no profile named missing", timeout)

        server = Server(command(manifest, bundle, "smoke"), env)
        try:
            server.send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "mcpb-smoke", "version": "1"}}})
            hello = server.answer(1, timeout)
            server.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
            server.send({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
            listed = server.answer(2, timeout)
        except SmokeError as err:
            server.proc.kill()
            server.wait(timeout)
            raise SmokeError(f"{err}\n{server.log()}") from None
        server.proc.stdin.close()
        code = server.wait(timeout)
        info = hello.get("result", {}).get("serverInfo", {})
        tools = listed.get("result", {}).get("tools", [])
        names = [t.get("name") for t in tools]
        problems = []
        if code != 0:
            problems.append(f"exit {code} after stdin closed")
        if "read-only" not in str(info.get("title", "")):
            problems.append(f"the server doesn't say it is read-only: {info}")
        if sorted(names) != sorted(t["name"] for t in manifest["tools"]):
            problems.append(f"tools/list gave {names}, the manifest lists {[t['name'] for t in manifest['tools']]}")
        for tool in tools:
            notes = tool.get("annotations", {})
            if notes.get("readOnlyHint") is not True or notes.get("destructiveHint") is not False:
                problems.append(f"{tool.get('name')} is not annotated read-only: {notes}")
        if problems:
            raise SmokeError("; ".join(problems) + "\n" + server.log())
        print(f"profile smoke: {info.get('name')} {info.get('version')} ({info.get('title')}), "
              f"{len(names)} read-only tools: {', '.join(names)}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="action", required=True)
    p = sub.add_parser("pack", help="write a platform's bundle")
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--platform", choices=sorted(PLATFORMS), required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--license", type=Path, required=True)
    p.add_argument("--notices", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    c = sub.add_parser("check", help="check a bundle statically")
    c.add_argument("bundle", type=Path)
    c.add_argument("--platform", choices=sorted(PLATFORMS), required=True)
    c.add_argument("--version", required=True)
    c.add_argument("--binary", type=Path)
    c.add_argument("--notices", type=Path)
    s = sub.add_parser("smoke", help="run a bundle as a host does")
    s.add_argument("bundle", type=Path)
    s.add_argument("--timeout", type=float, default=30.0)
    args = parser.parse_args(argv)
    if args.action == "pack":
        path = pack(args.binary, args.platform, args.version, args.license, args.notices, args.out)
        print(f"{path}: {path.stat().st_size} bytes")
        return 0
    if args.action == "check":
        errors = check_bundle(args.bundle, args.platform, args.version, args.binary, args.notices)
        for err in errors:
            print(f"error: {err}", file=sys.stderr)
        if not errors:
            print(f"OK: {args.bundle.name}")
        return 1 if errors else 0
    try:
        smoke(args.bundle, args.timeout)
    except (SmokeError, ValueError, OSError) as err:
        print(f"error: {args.bundle.name}: {err}", file=sys.stderr)
        return 1
    print(f"OK: {args.bundle.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
