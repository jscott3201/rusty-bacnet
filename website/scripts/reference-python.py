#!/usr/bin/env python3
"""Generate the Python reference pages from the type stub (#1989).

Reads ``crates/rusty-bacnet/rusty_bacnet.pyi`` with the ``ast`` module (the
extension is never imported) and writes one page per group under
``website/src/content/docs/reference/python/`` (all generated), and the
group index between the ``generated:start`` and ``generated:end`` markers
of ``website/src/content/docs/reference/python.md`` (the text around the
markers is authored and kept).

    python3 website/scripts/reference-python.py          # rewrite the pages
    python3 website/scripts/reference-python.py --check  # exit 1 if stale

The stub's section comments (``# Client``, ``# Server``, ...) pick each
item's group; a section this script doesn't know fails the run, so a new
part of the API gets a deliberate place. Errors (subclasses of an
exception), enumerations and module functions get groups of their own
wherever the stub puts them. The group pages' note names the release in
``website/src/lib/site.mjs``, so a release bump makes them stale too.
"""

from __future__ import annotations

import argparse
import ast
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STUB = ROOT / "crates/rusty-bacnet/rusty_bacnet.pyi"
INDEX = ROOT / "website/src/content/docs/reference/python.md"
GROUP_DIR = ROOT / "website/src/content/docs/reference/python"
START = "<!-- generated:start"
END = "<!-- generated:end -->"

# Group key (its page's slug) -> heading, the sentence under it and the
# page's description, in index order.
GROUPS = {
    "client": ("Client", "The client, its options and what its methods return.",
               "BACnetClient and its options, results and SC credential inspection, generated from the type stub."),
    "server": ("Server", "The server, its policies and counters, and the BACnet/SC hub.",
               "BACnetServer, its policies and counters, and ScHub, generated from the type stub."),
    "router": ("Router", "Owned B/IP routing between numbered networks.",
               "BACnetRouter and its ports and routes, generated from the type stub."),
    "endpoints": ("Shared endpoints", "One transport that runs both the client and server roles.",
                  "The B/IP, SC and MS/TP endpoints and their client and server roles, generated from the type stub."),
    "types": ("Types and values", "Identifiers, property values, structured values and B/IP modes.",
              "Object identifiers, property values, structured values and B/IP modes, generated from the type stub."),
    "enums": ("Enumerations", "Named BACnet constants. Each also takes vendor or unknown values from their number.",
              "The BACnet enumerations and their constants, generated from the type stub."),
    "errors": ("Errors", "The exceptions methods raise.",
               "The exceptions rusty_bacnet raises, generated from the type stub."),
    "functions": ("Module functions", "Functions at the top of the `rusty_bacnet` module.",
                  "The functions at the top of the rusty_bacnet module, generated from the type stub."),
}

# The classes each group opens with, in this order; the rest follow in stub order.
PRIMARY = {
    "client": ["BACnetClient"],
    "server": ["BACnetServer", "ScHub"],
    "router": ["BACnetRouter"],
    "endpoints": ["BipEndpoint", "ScEndpoint", "MstpEndpoint", "EndpointClient", "EndpointServer"],
}

# The stub's section comments, by the start of their text, and their groups.
SECTIONS = {
    "Enum types": "enums",
    "Core types": "types",
    "Exceptions": "errors",
    "Serial ports": "types",
    "B/IP modes": "types",
    "Client": "client",
    "Server": "server",
    "SC Hub": "server",
    "Endpoint": "endpoints",
    "Owned B/IP routing": "router",
}

# Dunder methods every value class has; the page leaves them out.
SKIPPED_DUNDERS = {"__repr__", "__str__", "__eq__", "__ne__", "__hash__", "__reduce__"}
WIDTH = 88


@dataclass
class Item:
    """One documented name: a class, a function, a constant or an alias."""

    name: str
    kind: str  # class, typeddict, enum, error, function, alias, constant
    group: str
    lines: list[str] = field(default_factory=list)


def sections(source: str) -> list[tuple[int, str]]:
    """(line, title) for each banner comment: a title between two rule lines."""
    lines = source.splitlines()
    found = []
    for i in range(1, len(lines) - 1):
        if lines[i - 1].startswith("# ---") and lines[i + 1].startswith("# ---") and lines[i].startswith("# "):
            found.append((i + 1, lines[i][2:].strip()))
    return found


def section_group(line: int, banners: list[tuple[int, str]]) -> str:
    title = None
    for start, name in banners:
        if start <= line:
            title = name
    if title is None:
        return "types"  # module constants above the first section
    for prefix, group in SECTIONS.items():
        if title == prefix or title.startswith(prefix + " ") or title.startswith(prefix + ":"):
            return group
    raise SystemExit(f"{STUB.name}:{line}: unknown section '{title}'; add it to SECTIONS in {Path(__file__).name}")


def comments_above(source_lines: list[str], lineno: int) -> str:
    """The run of ``#`` comment lines right above a statement, as one paragraph."""
    out = []
    i = lineno - 2
    while i >= 0 and source_lines[i].strip().startswith("#") and not source_lines[i].strip().startswith("# ---"):
        out.append(source_lines[i].strip().lstrip("#").strip())
        i -= 1
    return " ".join(reversed(out))


def escape(text: str) -> str:
    """Escape Markdown in prose, leaving code spans alone."""
    parts = text.split("`")
    for i in range(0, len(parts), 2):
        part = parts[i]
        part = part.replace("\\", "\\\\")
        for char in "<>*_[]":
            part = part.replace(char, "\\" + char)
        parts[i] = part
    return "`".join(parts)


def docstring_markdown(doc: str | None) -> list[str]:
    """Turn a reStructuredText-flavoured docstring into Markdown lines."""
    if not doc:
        return []
    out: list[str] = []
    lines = doc.splitlines()
    paragraph: list[str] = []

    def flush() -> None:
        # ``code`` spans may run across lines; join them before escaping.
        text = re.sub(r"``(.+?)``", lambda m: "`" + " ".join(m.group(1).replace("`", "").split()) + "`",
                      "\n".join(paragraph), flags=re.S)
        out.extend(prose(line) for line in text.splitlines())
        paragraph.clear()

    i = 0
    while i < len(lines):
        stripped = lines[i].strip()
        if not stripped:
            if paragraph:
                flush()
            out.append("")
            i += 1
            continue
        if not stripped.endswith("::"):
            paragraph.append(stripped)
            i += 1
            continue
        # A literal block follows: the indented lines after the blank line.
        lead = stripped[:-2].rstrip()
        if lead:
            paragraph.append(lead + ":")
        flush()
        out.append("")
        i += 1
        while i < len(lines) and not lines[i].strip():
            i += 1
        block = []
        while i < len(lines) and (not lines[i].strip() or lines[i].startswith((" ", "\t"))):
            block.append(lines[i])
            i += 1
        while block and not block[-1].strip():
            block.pop()
        indent = min((len(b) - len(b.lstrip()) for b in block if b.strip()), default=0)
        out.append("```python")
        out.extend(b[indent:] for b in block)
        out.append("```")
        out.append("")
    if paragraph:
        flush()
    while out and not out[-1]:
        out.pop()
    return out


def prose(line: str) -> str:
    """One line of prose as Markdown: ``code`` to `code`, the rest escaped."""
    line = re.sub(r"``(.+?)``", lambda m: "`" + m.group(1).replace("`", "") + "`", line)
    bullet = re.match(r"^([-*]|\d+\.)\s+", line)
    if bullet:
        marker = bullet.group(0)
        return ("- " if marker.strip() in "-*" else marker) + escape(line[len(marker):])
    text = escape(line)
    return "\\" + text if text.startswith("#") else text


SOURCE = ""  # the stub's text, for source segments; render() sets it


def code(node: ast.expr) -> str:
    """An expression as the stub writes it, or normalized when it spans lines."""
    segment = ast.get_source_segment(SOURCE, node)
    return segment if segment and "\n" not in segment else ast.unparse(node)


def signature(node: ast.FunctionDef | ast.AsyncFunctionDef) -> str:
    """The function's signature as Python, wrapped one parameter a line when long."""
    prefix = "async def " if isinstance(node, ast.AsyncFunctionDef) else "def "
    decorators = [f"@{ast.unparse(d)}" for d in node.decorator_list if ast.unparse(d) != "property"]
    returns = f" -> {code(node.returns)}" if node.returns else ""
    params = parameters(node.args)
    text = f"{prefix}{node.name}({', '.join(params)}){returns}"
    if len(text) > WIDTH:
        text = f"{prefix}{node.name}(\n" + "".join(f"    {p},\n" for p in params) + f"){returns}"
    return "\n".join(decorators + [text])


def parameter(arg: ast.arg, default: ast.expr | None, star: str = "") -> str:
    """One parameter as PEP 8 writes it: `name: T = default`, or `name=default`."""
    text = star + arg.arg
    if arg.annotation is not None:
        text += f": {code(arg.annotation)}"
    if default is not None:
        text += f" = {code(default)}" if arg.annotation is not None else f"={code(default)}"
    return text


def parameters(args: ast.arguments) -> list[str]:
    """The parameter list, with its `/` and `*` markers."""
    positional = args.posonlyargs + args.args
    defaults = [None] * (len(positional) - len(args.defaults)) + list(args.defaults)
    out = []
    for index, (arg, default) in enumerate(zip(positional, defaults)):
        out.append(parameter(arg, default))
        if args.posonlyargs and index == len(args.posonlyargs) - 1:
            out.append("/")
    if args.vararg is not None:
        out.append(parameter(args.vararg, None, "*"))
    elif args.kwonlyargs:
        out.append("*")
    for arg, default in zip(args.kwonlyargs, args.kw_defaults):
        out.append(parameter(arg, default))
    if args.kwarg is not None:
        out.append(parameter(args.kwarg, None, "**"))
    return out


def is_property(node: ast.AST) -> bool:
    return isinstance(node, ast.FunctionDef) and any(ast.unparse(d) == "property" for d in node.decorator_list)


def base_names(node: ast.ClassDef) -> list[str]:
    return [ast.unparse(b) for b in node.bases]


def is_enum(node: ast.ClassDef) -> bool:
    """An IntEnum, or a class whose constants are annotated with the class itself."""
    if any(b.endswith("Enum") for b in base_names(node)):
        return True
    constants = [s for s in node.body if isinstance(s, ast.AnnAssign) and ast.unparse(s.annotation) == node.name]
    return len(constants) >= 2


def render_class(node: ast.ClassDef, group: str, errors: set[str], source_lines: list[str]) -> Item:
    bases = base_names(node)
    if any(b in errors or b in {"Exception", "OSError"} for b in bases):
        kind, group = "error", "errors"
    elif "TypedDict" in bases:
        kind = "typeddict"
    elif is_enum(node):
        kind, group = "enum", "enums"
    else:
        kind = "class"
    item = Item(node.name, kind, group)
    out = item.lines
    out.append(f"## {node.name}")
    out.append("")
    # The bases as one code span: a code block per class would double the page's weight.
    parents = bases + [f"{keyword.arg}={ast.unparse(keyword.value)}" for keyword in node.keywords]
    if parents:
        out += [f"`class {node.name}({', '.join(parents)})`", ""]
    if node.name.startswith("_"):
        out += ["Not importable: the stub defines it only to type what a method returns.", ""]
    doc = docstring_markdown(ast.get_docstring(node))
    if doc:
        out += doc + [""]

    constants, fields, properties, methods = [], [], [], []
    body = node.body
    for index, stmt in enumerate(body):
        if isinstance(stmt, ast.AnnAssign) and isinstance(stmt.target, ast.Name):
            if kind == "enum" and ast.unparse(stmt.annotation) == node.name:
                constants.append(stmt.target.id)
                continue
            note = comments_above(source_lines, stmt.lineno)
            following = body[index + 1] if index + 1 < len(body) else None
            if isinstance(following, ast.Expr) and isinstance(following.value, ast.Constant) and isinstance(following.value.value, str):
                note = " ".join(filter(None, [note, " ".join(following.value.value.split())]))
            fields.append((stmt.target.id, code(stmt.annotation), note))
        elif isinstance(stmt, (ast.FunctionDef, ast.AsyncFunctionDef)):
            if stmt.name in SKIPPED_DUNDERS:
                continue
            if stmt.name.startswith("_") and not stmt.name.startswith("__"):
                continue
            (properties if is_property(stmt) else methods).append(stmt)

    if constants:
        out.append(f"{len(constants)} constants:")
        out.append("")
        out.append(", ".join(f"`{name}`" for name in constants))
        out.append("")
    if fields:
        out.append("Keys:" if kind == "typeddict" else "Attributes:")
        out.append("")
        for name, annotation, note in fields:
            line = f"- `{name}: {annotation}`"
            if note:
                line += ": " + prose(note)
            out.append(line)
        out.append("")
    if properties:
        out.append("Properties:")
        out.append("")
        for prop in properties:
            returns = code(prop.returns) if prop.returns else "Any"
            line = f"- `{prop.name}: {returns}`"
            doc = ast.get_docstring(prop)
            if doc:
                line += ": " + prose(" ".join(doc.split()))
            out.append(line)
        out.append("")
    for method in methods:
        out.append(f"### {node.name}.{method.name}")
        out.append("")
        out += ["```python", signature(method), "```", ""]
        doc = docstring_markdown(ast.get_docstring(method))
        if doc:
            out += doc + [""]
    return item


def render(source: str) -> dict[str, list[Item]]:
    """The stub's documented names, by group, in page order."""
    global SOURCE
    SOURCE = source
    tree = ast.parse(source)
    banners = sections(source)
    source_lines = source.splitlines()
    # Exceptions first, so a subclass anywhere in the stub is found.
    errors: set[str] = {"Exception", "OSError"}
    changed = True
    while changed:
        changed = False
        for node in tree.body:
            if isinstance(node, ast.ClassDef) and node.name not in errors and any(b in errors for b in base_names(node)):
                errors.add(node.name)
                changed = True

    items: list[Item] = []
    body = tree.body
    for index, node in enumerate(body):
        name = getattr(node, "name", None) or (node.target.id if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name) else None)
        if name is None and isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            name = node.targets[0].id
        # A private name is left out, except a TypedDict the stub uses in a
        # public signature (`_DeviceReadBatchResult`, say).
        private_typed = isinstance(node, ast.ClassDef) and "TypedDict" in base_names(node)
        if name is None or (name.startswith("_") and not private_typed):
            continue
        group = section_group(node.lineno, banners)
        if isinstance(node, ast.ClassDef):
            items.append(render_class(node, group, errors, source_lines))
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            item = Item(name, "function", "functions")
            item.lines += [f"## {name}", "", "```python", signature(node), "```", ""]
            doc = docstring_markdown(ast.get_docstring(node))
            if doc:
                item.lines += doc + [""]
            items.append(item)
        elif isinstance(node, ast.AnnAssign):
            item = Item(name, "constant", group)
            item.lines += [f"## {name}", "", "```python", f"{name}: {code(node.annotation)}", "```", ""]
            following = body[index + 1] if index + 1 < len(body) else None
            if isinstance(following, ast.Expr) and isinstance(following.value, ast.Constant) and isinstance(following.value.value, str):
                item.lines += docstring_markdown(following.value.value) + [""]
            items.append(item)
        elif isinstance(node, ast.Assign):
            item = Item(name, "alias", group)
            item.lines += [f"## {name}", "", "```python", ast.get_source_segment(SOURCE, node), "```", ""]
            note = comments_above(source_lines, node.lineno)
            if note:
                item.lines += [prose(note), ""]
            items.append(item)


    order = {"class": 0, "error": 0, "enum": 0, "function": 0, "constant": 1, "typeddict": 2, "alias": 3}
    groups: dict[str, list[Item]] = {}
    for key in GROUPS:
        primary = PRIMARY.get(key, [])
        members = [i for i in items if i.group == key]
        missing = set(primary) - {i.name for i in members}
        if missing:
            raise SystemExit(f"PRIMARY names {sorted(missing)}, not in group '{key}'; update {Path(__file__).name}")
        if not members:
            raise SystemExit(f"group '{key}' is empty; update GROUPS in {Path(__file__).name}")
        members.sort(key=lambda i: (primary.index(i.name) if i.name in primary else len(primary), order[i.kind]))
        groups[key] = members
    return groups


def site_release() -> str:
    """The release website/src/lib/site.mjs names."""
    text = (ROOT / "website/src/lib/site.mjs").read_text(encoding="utf-8")
    match = re.search(r"^export const release = '([^']+)';$", text, re.M)
    if not match:
        raise SystemExit("website/src/lib/site.mjs names no release")
    return match.group(1)


def callout(release: str) -> list[str]:
    """The note every generated Python page opens with."""
    return [
        ":::note[On `dev` after the release]",
        "This page is generated from the type stub on the `dev` branch and tracks the next release, so it can list"
        f" classes, methods and keywords merged after v{release} that the released wheel doesn't have. The"
        f" [v{release} release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v{release}) and the"
        " [changes waiting for the next release](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d)"
        " say what changed.",
        ":::",
        "",
    ]


def group_page(key: str, members: list[Item], release: str) -> str:
    """One group's page, all generated."""
    title, blurb, description = GROUPS[key]
    out = [
        "---",
        f'title: "Python {title.lower()}"',
        f'description: "{description}"',
        "---",
        "",
        *callout(release),
        f"{blurb} The [Python reference](/rusty-bacnet/reference/python/) lists every group. Each page opens"
        " with its main classes, then the other classes, the dictionaries (`TypedDict`) and the type aliases,"
        " in stub order.",
        "",
        f"{START} by website/scripts/reference-python.py; do not edit this page -->",
        "",
    ]
    for member in members:
        out += member.lines
    while out and not out[-1]:
        out.pop()
    return "\n".join(out + [END, ""])


def index_region(groups: dict[str, list[Item]]) -> str:
    """The index page's generated region: each group and the names on it."""
    out = []
    for key, members in groups.items():
        title, blurb, _ = GROUPS[key]
        out += [f"## [{title}](/rusty-bacnet/reference/python/{key}/)", "", blurb, ""]
        names = [f"[`{m.name}`](/rusty-bacnet/reference/python/{key}/#{m.name.lower()})" for m in members]
        out += [", ".join(names), ""]
    while out and not out[-1]:
        out.pop()
    return "\n".join(out)


def splice(page: str, region: str) -> str:
    start = page.find(START)
    if start < 0:
        raise SystemExit(f"{INDEX}: no generated:start marker")
    opened = page.index("-->", start) + 3
    end = page.find(END, opened)
    if end < 0:
        raise SystemExit(f"{INDEX}: no generated:end marker after generated:start")
    return f"{page[:opened]}\n\n{region}\n{page[end:]}"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="exit 1 when a page is stale; change nothing")
    args = parser.parse_args()
    groups = render(STUB.read_text(encoding="utf-8"))
    release = site_release()
    wanted = {INDEX: splice(INDEX.read_text(encoding="utf-8"), index_region(groups))}
    for key, members in groups.items():
        wanted[GROUP_DIR / f"{key}.md"] = group_page(key, members, release)
    extra = [path for path in GROUP_DIR.glob("*.md*") if path not in wanted]
    stale = [path for path, text in wanted.items() if not path.exists() or path.read_text(encoding="utf-8") != text]
    name = lambda path: path.relative_to(ROOT).as_posix()
    if args.check:
        for path in stale:
            print(f"{name(path)} is stale", file=sys.stderr)
        for path in extra:
            print(f"{name(path)} is not a group page", file=sys.stderr)
        if stale or extra:
            print("run python3 website/scripts/reference-python.py", file=sys.stderr)
            return 1
        print("the Python reference pages are current")
        return 0
    GROUP_DIR.mkdir(exist_ok=True)
    for path in stale:
        path.write_text(wanted[path], encoding="utf-8", newline="\n")
        print(f"wrote {name(path)}")
    for path in extra:
        path.unlink()
        print(f"removed {name(path)}")
    if not stale and not extra:
        print("the Python reference pages are current")
    return 0


if __name__ == "__main__":
    sys.exit(main())
