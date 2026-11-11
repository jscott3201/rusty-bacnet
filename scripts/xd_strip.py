#!/usr/bin/env python3
"""Strip ASHRAE's BACnet XD files down to names, numbers and flags (#2435).

ASHRAE publishes the BACnet datatypes and object types as XML ("XD" files,
CSML namespace http://bacnet.org/csml/1.2) under the Apache License 2.0.
Their XML comments and comment attributes carry text from the standard,
which this repository must not hold, so the originals are not committed.
This script reads manually downloaded copies (the site refuses automated
clients) and writes a derivative that keeps only facts:

- every enumeration and bit string with a named member list: its path (the
  names of the definitions it sits in, joined with "/"), its range or length
  attributes, and each member's value and name;
- every object type: each property's identifier number, name, datatype, the
  size of a fixed-size array, and conformance flags.

No comment, description or other prose is copied. The output is sorted and
formatted the same way on every run. Anything the script doesn't recognise
(an element, attribute or flag value) stops it with an error, so a new file
revision can't drop information silently.

    python3 scripts/xd_strip.py DATA_TYPES_XML OBJECT_TYPES_XML OUT_DIR

writes OUT_DIR/xd-data-types.txt and OUT_DIR/xd-object-types.txt. The
cross-checks in bacnet-types read both (src/enums/tests/xd_crosscheck.rs the
first, src/datatype/standard/xd_crosscheck.rs the second, #2452), and one in
bacnet-objects reads the second (src/property_metadata_audit/xd_conformance.rs).
"""

import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

NS = "{http://bacnet.org/csml/1.2}"
REGEN = "python3 scripts/xd_strip.py <bacnet-data-types.xml> <bacnet-object-types.xml> testdata/ashrae-xd"

# Table attributes the data-types file uses on an Enumerated or BitString that
# defines its members. "comment" is prose and is dropped; "contextTag" and
# "optional" describe the field that holds the table, not the table.
TABLE_KEEP = ("minimum", "maximum", "length")
TABLE_DROP = {"name", "comment", "contextTag", "optional"}
MEMBER_ATTRS = {"Unsigned": ("value", {"name", "value", "comment"}), "Bit": ("bit", {"name", "bit", "comment"})}
LIST_TAGS = {"Enumerated": "NamedValues", "BitString": "NamedBits"}
# Values the source names twice. Both names are written out; any other repeat
# is an error. Revision 24 gives units value 47910 two names.
KNOWN_DUPLICATES = {("0-BACnetEngineeringUnits", 47910)}

# Object type property attributes: kept as flags, kept as a datatype, or
# dropped (variability, string lengths, units and value constraints are left
# for a later step). An array's size is kept as one number: the file gives a
# fixed-size array the same minimum and maximum, and any other pairing stops
# the script.
PROP_BOOL_FLAGS = ("optional", "writable", "commandable", "readable")
PROP_DROP = {
    "name", "propertyIdentifier", "type", "memberType", "variability", "minimumSize",
    "maximumSize", "minimumLength", "value", "associatedWith", "units", "minimum",
    "maximum", "requiredWith",
}
DATATYPE_TAGS = {
    "Any", "Array", "BitString", "Boolean", "Choice", "Date", "DatePattern", "DateTime",
    "DateTimePattern", "Double", "Enumerated", "Integer", "List", "ObjectIdentifier",
    "OctetString", "Real", "Sequence", "String", "Time", "TimePattern", "Unsigned",
}


def fail(msg):
    sys.exit(f"xd_strip: {msg}")


def tag(e):
    if not e.tag.startswith(NS):
        fail(f"element outside the CSML namespace: {e.tag}")
    return e.tag[len(NS):]


def load(path):
    """Parse one XD file; return (root, revision, published, copyright line)."""
    raw = Path(path).read_text(encoding="utf-8")
    if "Licensed under the Apache License, Version 2.0" not in raw:
        fail(f"{path}: no Apache-2.0 licence header")
    m = re.search(r"Copyright (\d{4}) ASHRAE", raw)
    if not m:
        fail(f"{path}: no ASHRAE copyright line")
    root = ET.fromstring(raw)  # ElementTree drops XML comments
    if tag(root) != "CSML":
        fail(f"{path}: root is {root.tag}, not CSML")
    rev, published = root.get("dataRev"), root.get("published")
    if not rev or not published:
        fail(f"{path}: CSML root lacks dataRev or published")
    defs = list(root)
    if len(defs) != 1 or tag(defs[0]) != "Definitions":
        fail(f"{path}: expected one Definitions element under CSML")
    return defs[0], rev, published, f"Copyright {m.group(1)} ASHRAE"


def check_bool(where, key, value):
    if value not in ("true", "false"):
        fail(f"{where}: {key}={value!r} is not true or false")


def collect_tables(e, path, out):
    """Walk the data-types tree; append (kind, path, attrs, members)."""
    t = tag(e)
    name = e.get("name")
    here = path + [name] if name else path
    lists = [c for c in e if tag(c) in ("NamedValues", "NamedBits")]
    if lists:
        where = "/".join(here) or "<unnamed>"
        if t not in LIST_TAGS or len(lists) != 1 or tag(lists[0]) != LIST_TAGS[t]:
            fail(f"{where}: unexpected member list layout under {t}")
        if not name:
            fail(f"{where}: a member table without a name")
        extra = [tag(c) for c in e if c is not lists[0]]
        if extra:
            fail(f"{where}: unexpected children beside the member list: {extra}")
        unknown = set(e.attrib) - TABLE_DROP - set(TABLE_KEEP)
        if unknown:
            fail(f"{where}: unknown table attributes {sorted(unknown)}")
        if "optional" in e.attrib:
            check_bool(where, "optional", e.get("optional"))
        members = []
        for m in lists[0]:
            mt = tag(m)
            if mt not in MEMBER_ATTRS or (t == "Enumerated") != (mt == "Unsigned"):
                fail(f"{where}: unexpected member element {mt}")
            key, allowed = MEMBER_ATTRS[mt]
            unknown = set(m.attrib) - allowed
            if unknown or len(m):
                fail(f"{where}: unexpected member attributes or children {sorted(unknown)}")
            mname, raw = m.get("name"), m.get(key)
            if not mname or not raw or not raw.isdigit() or not re.fullmatch(r"[a-z0-9][a-z0-9-]*", mname):
                fail(f"{where}: member {mname!r}={raw!r} is not a kebab name and a number")
            members.append((int(raw), mname))
        values = [v for v, _ in members]
        for v in sorted(set(values)):
            if values.count(v) > 1 and (where, v) not in KNOWN_DUPLICATES:
                fail(f"{where}: value {v} named twice")
        if len({n for _, n in members}) != len(members):
            fail(f"{where}: a member name repeats")
        kind = "enum" if t == "Enumerated" else "bits"
        attrs = [(k, e.get(k)) for k in TABLE_KEEP if e.get(k) is not None]
        for k, v in attrs:
            if not v.isdigit():
                fail(f"{where}: {k}={v!r} is not a number")
        out.append((kind, "/".join(here), attrs, members))
        return
    if t in LIST_TAGS and len(e) and not any(tag(c) in ("NamedValues", "NamedBits") for c in e):
        # An Enumerated or BitString with children but no member list.
        fail(f"{'/'.join(here)}: {t} with children {[tag(c) for c in e]} and no member list")
    for c in e:
        collect_tables(c, here, out)


def header(kind, sources, rev, published, copyright_line):
    names = ", ".join(sources)
    return [
        f"# Stripped BACnet XD {kind}, generated by scripts/xd_strip.py (#2435). Do not edit.",
        f"# Source: {names} from data.ashrae.org, protocol revision {rev}, published {published}.",
        f"# {copyright_line}. Licensed under the Apache License, Version 2.0; a copy is in",
        "# LICENSE-APACHE-2.0 beside this file, and at http://www.apache.org/licenses/LICENSE-2.0.",
        "# Distributed on an \"AS IS\" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND.",
        "# Modified by the Rusty BACnet project: reduced to names, numbers and flags; every",
        "# comment and description of the original was removed.",
        f"# Regenerate: {REGEN}",
        "",
    ]


def strip_data_types(path):
    defs, rev, published, copyright_line = load(path)
    tables = []
    collect_tables(defs, [], tables)
    paths = [p for _, p, _, _ in tables]
    if len(set(paths)) != len(paths):
        fail(f"{path}: a table path repeats")
    lines = header("data types", ["bacnet-data-types.xml"], rev, published, copyright_line)
    lines.append("# <enum|bits> <path> [minimum=N] [maximum=N] [length=N], then one '<value> <name>' line per member.")
    for kind, tpath, attrs, members in sorted(tables, key=lambda t: t[1]):
        lines.append("")
        lines.append(" ".join([kind, tpath] + [f"{k}={v}" for k, v in attrs]))
        lines.extend(f"{v} {n}" for v, n in sorted(members))
    return lines, len(tables)


def strip_object_types(path):
    defs, rev, published, copyright_line = load(path)
    objects = {}
    for o in defs:
        if tag(o) != "Object" or set(o.attrib) != {"name"}:
            fail(f"{path}: unexpected definition {tag(o)} {sorted(o.attrib)}")
        oname = o.get("name")
        if oname in objects:
            fail(f"{path}: object {oname} repeats")
        props = {}
        for p in o:
            pt = tag(p)
            where = f"{oname}/{p.get('name')}"
            if pt not in DATATYPE_TAGS or len(p):
                fail(f"{where}: unexpected property element {pt}")
            unknown = set(p.attrib) - PROP_DROP - set(PROP_BOOL_FLAGS)
            if unknown:
                fail(f"{where}: unknown property attributes {sorted(unknown)}")
            pid = p.get("propertyIdentifier", "")
            if not pid.isdigit() or not p.get("name"):
                fail(f"{where}: property without an identifier number or name")
            if int(pid) in props:
                fail(f"{where}: property {pid} repeats")
            datatype = pt
            if p.get("type"):
                datatype += ":" + p.get("type")
            if p.get("memberType"):
                datatype += "<" + p.get("memberType") + ">"
            fields = [pid, p.get("name"), datatype]
            low, high = p.get("minimumSize"), p.get("maximumSize")
            if low is not None or high is not None:
                if pt != "Array" or low != high or not low.isdigit():
                    fail(f"{where}: sizes {low!r}..{high!r} on {pt} are not one fixed array size")
                fields.append(f"size={low}")
            for flag in PROP_BOOL_FLAGS:
                if flag in p.attrib:
                    check_bool(where, flag, p.get(flag))
                    fields.append(f"{flag}={p.get(flag)}")
            if p.get("requiredWith"):
                fields.append("required-with=" + ",".join(sorted(p.get("requiredWith").split(","))))
            if any(" " in f for f in fields):
                fail(f"{where}: a field holds a space")
            props[int(pid)] = " ".join(fields)
        objects[oname] = props
    lines = header("object types", ["bacnet-object-types.xml"], rev, published, copyright_line)
    lines.append(
        "# object <name>, then one line per property: <identifier> <name> <datatype> [size=N] [flag=value ...]."
    )
    for oname in sorted(objects):
        lines.append("")
        lines.append(f"object {oname}")
        lines.extend(objects[oname][pid] for pid in sorted(objects[oname]))
    return lines, len(objects)


def main(argv):
    if len(argv) != 4:
        sys.exit(__doc__)
    data_types, object_types, out_dir = argv[1], argv[2], Path(argv[3])
    dt_lines, n_tables = strip_data_types(data_types)
    ot_lines, n_objects = strip_object_types(object_types)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "xd-data-types.txt").write_text("\n".join(dt_lines) + "\n", encoding="utf-8")
    (out_dir / "xd-object-types.txt").write_text("\n".join(ot_lines) + "\n", encoding="utf-8")
    print(f"xd_strip: {n_tables} tables, {n_objects} object types written to {out_dir}")


if __name__ == "__main__":
    main(sys.argv)
