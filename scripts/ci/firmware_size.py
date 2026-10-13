#!/usr/bin/env python3
"""Check a firmware image's sizes against its budget (#1626), and the stack
an emulation run reports against the stack budget (#1660).

  firmware_size.py <elf> <budget.toml> <table>
  firmware_size.py --suggest <elf> <table>
  firmware_size.py --stack [--all-boards] <run.log> <stack-budget.toml> <board>...
  firmware_size.py --stack-suggest <run.log>

<table> names the image's budget, `<target>.<image>`, such as
`thumbv7m-none-eabi.mstp`. The check prints:

- every section the image loads, and whether it counts toward flash, RAM
  or both;
- .text, .rodata, .data, .bss and the flash and RAM totals against the
  table's budget, with the counts of core::fmt functions, of 64-bit
  division routines and of memmove routines linked in;
- the largest objects in RAM, and .text by crate.

It exits 1 when a size or a count is over budget, when the budget lacks an
entry, or when the image has no symbol table to look for those functions
in. A size more than 20% under its budget, or a count under its limit, gets
a notice, not a failure. --suggest prints a budget table for the image
instead, each size plus 12%. Used by check-firmware-link.sh; see
docs/ci.md, "Firmware link test".

--stack reads the output of scripts/emu/emulate-firmware.sh's run, whose
images each print `PASS <image> on <board> (<target>): ...; stack peak N of
M octets`, and checks each peak against the stack budget's `[<board>]`
table, which has a key per image. It exits 1 when a peak is over its
budget, when an image the table lists printed no PASS line, or when a PASS
line has no budget. With --all-boards, which says the boards named are all
the emulation has, it also fails on a table for any other board, left from
a board since removed. A peak that needs a budget more than 20% under the
one it has gets a notice. --stack-suggest prints a table for each board
instead, each peak plus 25%, rounded as the sizes are. Used by
emulate-firmware.sh; see docs/ci.md, "Emulation (manual)".

Totals go by section flags, not by memory region: flash is every loaded
section that has contents (SHF_ALLOC, not NOBITS), .data's initial image
included; RAM is every loaded writable section, .data included. So code
placed in RAM would count toward flash only, and the alignment gaps
between sections count toward neither. riscv-rt's .stack is left out of
RAM: its link.x sizes it to fill whatever RAM is left, so it says nothing
about the image. Its .heap does count: that is `_heap_size` octets reserved
on purpose (0 by default).

The script reads the ELF's section headers and symbol table itself, so it
needs only Python 3.9 or later, no binutils. The budget file is the subset
of TOML that `read_budget` accepts: comments, `[name]` tables and
`key = integer`, with bare keys, such as `mstp-extended`.
"""

import re
import struct
import sys
from pathlib import Path

SECTIONS = (".text", ".rodata", ".data", ".bss")
# Each row of the budget: (label, budget key).
ROWS = tuple((name, name.lstrip(".")) for name in SECTIONS) + (
    ("flash", "flash"),
    ("ram", "ram"),
    ("core::fmt", "fmt_functions"),
    ("64-bit division", "div64_functions"),
    ("memmove", "memmove_functions"),
)
# Counts of functions, budgeted as counts rather than sizes: (key, what each
# listed function is called in the output, advice when over budget).
COUNTS = (
    ("fmt_functions", "core::fmt function",
     "find what formats, starting from the functions listed above"),
    ("div64_functions", "64-bit division function",
     "find the 64-bit division by a runtime value, starting from the functions listed"
     " above, and divide in 32 bits"),
    ("memmove_functions", "memmove function",
     "find the copy that calls memmove, starting from the functions listed above, and"
     " copy field by field or swap instead"),
)
# What --suggest and the over-budget error add to a measured size.
HEADROOM = 0.12
# What --stack-suggest and the stack check's error add to a stack peak, more
# than a size gets: one more buffer on the stack, or a change in what the
# compiler inlines, can move the peak by a whole frame.
STACK_HEADROOM = 0.25
# An emulation image's line when its run passed.
PASS_LINE = re.compile(
    r"PASS (?P<image>\S+) on (?P<board>\S+) \((?P<target>[^)]+)\): .*"
    r"; stack peak (?P<peak>\d+) of (?P<room>\d+) octets"
)
# A size this far under its budget gets a notice. Budgets of TOKEN octets or
# less are allowances for an empty section and get none.
TIGHTEN = 0.20
TOKEN = 64
# Sections the linker script sizes to fill free RAM.
RESERVATIONS = (".stack",)
# Section types and flags, and symbol types, from the ELF specification.
SHT_SYMTAB = 2
SHT_NOBITS = 8
SHF_WRITE = 0x1
SHF_ALLOC = 0x2
STT_OBJECT = 1
STT_FUNC = 2
# Objects in RAM and crates in .text to list, largest first.
TOP = 4
# A Rust symbol in core::fmt or below, in either mangling scheme.
FMT_MARK = "4core3fmt"
# 64-bit division on a 32-bit target, which has no instruction for it: the
# C-ABI routines compiler-builtins exports, signed and unsigned, quotient
# and remainder, and its Rust implementation behind them. A 64-bit division
# by a constant compiles to multiplications and links none of these.
DIV64_ROUTINES = frozenset({
    "__aeabi_uldivmod", "__aeabi_ldivmod",
    "__udivmoddi4", "__divmoddi4", "__udivdi3", "__divdi3", "__umoddi3", "__moddi3",
})
DIV64_RUST = re.compile(r"specialized_div_rem.*[iu]64_div_rem")
# memmove: compiler-builtins' Rust implementation, about 0.8-1.4 KB on these
# targets, and the C-ABI entry points in front of it. A whole-value copy
# the compiler can't prove doesn't overlap, or a slice rotation, calls it.
MEMMOVE_ROUTINES = frozenset({
    "memmove", "__aeabi_memmove", "__aeabi_memmove4", "__aeabi_memmove8",
    "compiler_builtins::mem::memmove",
})


class InputError(Exception):
    pass


def cstring(data, start):
    end = data.find(b"\0", start)
    if end < 0:
        raise InputError("unterminated string in the ELF")
    return data[start:end].decode(errors="replace")


def read_sections(data):
    """(name, type, flags, offset, size, link, entsize) for each section of
    a 32- or 64-bit little-endian ELF."""
    if data[:4] != b"\x7fELF" or len(data) < 64 or data[5] != 1 or data[4] not in (1, 2):
        raise InputError("not a little-endian ELF")
    if data[4] == 1:
        shoff, shentsize, shnum, shstrndx = struct.unpack_from("<32xI10xHHH", data)
        layout = "<IIIIIIIIII"
    else:
        shoff, shentsize, shnum, shstrndx = struct.unpack_from("<40xQ10xHHH", data)
        layout = "<IIQQQQIIQQ"
    raw = [struct.unpack_from(layout, data, shoff + i * shentsize) for i in range(shnum)]
    names = raw[shstrndx][4]
    return [
        (cstring(data, names + name), kind, flags, offset, size, link, entsize)
        for name, kind, flags, _addr, offset, size, link, _info, _align, entsize in raw
    ]


def read_symbols(data, sections):
    """(name, size, type, section name) for each entry of .symtab, or None
    when the image has no symbol table."""
    symtab = next((s for s in sections if s[1] == SHT_SYMTAB), None)
    if symtab is None:
        return None
    _, _, _, offset, size, link, entsize = symtab
    if entsize == 0:
        raise InputError("the symbol table's entry size is 0")
    strings = sections[link][3]
    wide = data[4] == 2
    symbols = []
    for at in range(offset, offset + size, entsize):
        if wide:
            name, info, _other, shndx, _value, length = struct.unpack_from("<IBBHQQ", data, at)
        else:
            name, _value, length, info, _other, shndx = struct.unpack_from("<IIIBBH", data, at)
        where = sections[shndx][0] if 0 < shndx < len(sections) else ""
        symbols.append((cstring(data, strings + name), length, info & 0xF, where))
    return symbols


def counted_in(name, kind, flags):
    """Which totals a section counts toward: 'flash', 'ram', both, or none."""
    if not flags & SHF_ALLOC:
        return ()
    totals = []
    if kind != SHT_NOBITS:
        totals.append("flash")
    if flags & SHF_WRITE and name not in RESERVATIONS:
        totals.append("ram")
    return tuple(totals)


def v0_identifier(name, i):
    """An identifier at name[i] in the v0 scheme: optional disambiguator,
    decimal length, optional `_` separator, the bytes."""
    if name.startswith("s", i):
        i = name.index("_", i) + 1
    digits = re.match(r"\d+", name[i:])
    if not digits:
        raise ValueError(name)
    i += digits.end()
    if name.startswith("_", i):
        i += 1
    length = int(digits.group())
    return name[i : i + length], i + length


def v0_path(name, i):
    """A path made of crate roots (C) and nested names (N); anything else,
    such as an impl or generic arguments, raises ValueError."""
    if name.startswith("C", i):
        return v0_identifier(name, i + 1)
    if name.startswith("N", i):
        parent, i = v0_path(name, i + 2)
        ident, i = v0_identifier(name, i)
        return (f"{parent}::{ident}" if ident else parent), i
    raise ValueError(name)


def demangle(name):
    """A readable path for a Rust symbol where that is easy, else the name."""
    if name.startswith("_R"):
        try:
            path, end = v0_path(name, 2)
            if end == len(name):
                return path
        except (ValueError, IndexError):
            pass
        return name
    legacy = re.fullmatch(r"_ZN(.*)E", name)
    if not legacy:
        return name
    rest, parts = legacy.group(1), []
    while rest:
        digits = re.match(r"\d+", rest)
        if not digits:
            return name
        start, length = digits.end(), int(digits.group())
        parts.append(rest[start : start + length])
        rest = rest[start + length :]
    if parts and re.fullmatch(r"h[0-9a-f]{16}", parts[-1]):
        parts.pop()
    return "::".join(parts)


def crate_of(name):
    """The crate that defines a Rust symbol: the first crate root in a v0
    name, or the first path segment in a legacy one. A generic is credited
    to the crate that defines it, wherever it was instantiated. Symbols that
    aren't Rust, such as memcpy, come back as '(unmangled)'."""
    if name.startswith("_R"):
        root = re.search(r"C(?:s[0-9A-Za-z]*_)?(\d+)_?", name)
        if root:
            start = root.end()
            return name[start : start + int(root.group(1))]
    elif name.startswith("_ZN"):
        first = re.match(r"_ZN(\d+)", name)
        if first:
            segment = name[first.end() : first.end() + int(first.group(1))]
            # An impl, `<crate::Type as Trait>`, mangles as `_$LT$crate..Type...`.
            return segment.removeprefix("_$LT$").split("..")[0]
    return "(unmangled)"


def read_budget(path):
    """{table: {key: int}} from the TOML subset described in the module docs."""
    tables, table = {}, None
    for number, line in enumerate(Path(path).read_text().splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        header = re.fullmatch(r"\[([A-Za-z0-9_.-]+)\]", line)
        entry = re.fullmatch(r"([A-Za-z0-9_-]+)\s*=\s*(\d+)", line)
        if header:
            table = tables.setdefault(header.group(1), {})
        elif entry and table is not None:
            table[entry.group(1)] = int(entry.group(2))
        else:
            raise InputError(f"{path}:{number}: expected [table] or key = integer")
    return tables


def suggested(measured, headroom=HEADROOM):
    """The measured size plus `headroom`, rounded up to 16 octets."""
    return -(-int(measured * (1 + headroom)) // 16) * 16


def measure(elf):
    """The image's loaded sections, its measured sizes by budget key, its
    symbols, and the functions each count lists. Fails when the image has
    no usable symbol table."""
    data = Path(elf).read_bytes()
    sections = read_sections(data)
    symbols = read_symbols(data, sections)
    if not symbols or not any(s[2] == STT_FUNC for s in symbols):
        raise InputError(
            f"{elf} has no function symbols, so core::fmt, 64-bit division and memmove"
            " can't be checked: the image must not be stripped"
        )
    loaded = [(s[0], s[4], counted_in(s[0], s[1], s[2])) for s in sections if s[2] & SHF_ALLOC]
    sizes = {name.lstrip("."): 0 for name in SECTIONS}
    for name, size, _ in loaded:
        if name in SECTIONS:
            sizes[name.lstrip(".")] = size
    sizes["flash"] = sum(size for _, size, totals in loaded if "flash" in totals)
    sizes["ram"] = sum(size for _, size, totals in loaded if "ram" in totals)
    functions = [s[0] for s in symbols if s[2] == STT_FUNC]
    listed = {
        "fmt_functions": sorted({demangle(name) for name in functions if FMT_MARK in name}),
        "div64_functions": sorted({demangle(name) for name in functions if is_div64(name)}),
        "memmove_functions": sorted({
            demangle(name) for name in functions if demangle(name) in MEMMOVE_ROUTINES
        }),
    }
    for key, names in listed.items():
        sizes[key] = len(names)
    return loaded, sizes, symbols, listed


def is_div64(name):
    """Whether a function symbol is a 64-bit division routine."""
    return name in DIV64_ROUTINES or DIV64_RUST.search(name) is not None


def check(elf, budget_path, table):
    loaded, sizes, symbols, listed = measure(elf)
    budget = read_budget(budget_path).get(table)
    if budget is None:
        raise InputError(f"{budget_path} has no [{table}] table")
    missing = [key for _, key in ROWS if key not in budget]
    if missing:
        raise InputError(f"{budget_path} [{table}] has no {', '.join(missing)}")

    print(f"  {'loaded section':<16}{'size':>8}  counted in")
    for name, size, totals in loaded:
        if size:
            print(f"  {name:<16}{size:>8}  {' and '.join(totals) or '-'}")
    print(f"  {'':<16}{'size':>8}{'budget':>8}")
    counts = {key for key, _, _ in COUNTS}
    failed, loose = [], []
    for label, key in ROWS:
        measured, limit = sizes[key], budget[key]
        mark = ""
        if measured > limit:
            failed.append(key)
            mark = "  over budget"
        elif key in counts:
            # A count has no headroom: any room under its limit is room a
            # new function could take unnoticed.
            if measured < limit:
                loose.append(key)
                mark = "  under its limit"
        elif limit > TOKEN and measured < limit * (1 - TIGHTEN):
            loose.append(key)
            mark = "  well under budget"
        print(f"  {label:<16}{measured:>8}{limit:>8}{mark}")

    ram = [s for s in symbols if s[2] == STT_OBJECT and s[3] in (".data", ".bss")]
    print("  largest objects in RAM:")
    for name, size, _, where in sorted(ram, key=lambda s: -s[1])[:TOP]:
        print(f"  {size:>10}  {where:<6} {demangle(name)}")
    by_crate = {}
    for name, size, kind, where in symbols:
        if kind == STT_FUNC and where == ".text":
            by_crate[crate_of(name)] = by_crate.get(crate_of(name), 0) + size
    print("  .text by crate:")
    for crate, size in sorted(by_crate.items(), key=lambda item: -item[1])[:TOP + 2]:
        print(f"  {size:>10}  {crate}")
    for key, label, _ in COUNTS:
        for name in listed[key]:
            print(f"  {label}: {name}")

    # The table first, then the messages under it, when both go to one log.
    sys.stdout.flush()
    for key in loose:
        if key in counts:
            print(
                f"notice: {table}: {key} is {sizes[key]}, under its limit of {budget[key]};"
                f" lower the limit to {sizes[key]}, so a new function can't take its place.",
                file=sys.stderr,
            )
        else:
            print(
                f"notice: {table}: {key} is {sizes[key]}, more than {TIGHTEN:.0%} under"
                f" its budget of {budget[key]}; consider tightening it to"
                f" {suggested(sizes[key])}.",
                file=sys.stderr,
            )
    for key in failed:
        counted = [advice for counted, _, advice in COUNTS if counted == key]
        if counted:
            advice = counted[0]
        else:
            advice = (
                "if the growth is intended, raise the budget"
                f" ({suggested(sizes[key])} leaves {HEADROOM:.0%} headroom)"
            )
        print(
            f"error: {table}: {key} is {sizes[key]}, over its budget of"
            f" {budget[key]} in {budget_path}; {advice}. See docs/ci.md.",
            file=sys.stderr,
        )
    return 1 if failed else 0


def suggest(elf, table):
    _, sizes, _, _ = measure(elf)
    print(f"[{table}]")
    for _, key in ROWS:
        if any(key == counted for counted, _, _ in COUNTS):
            value = sizes[key]
        elif sizes[key] == 0:
            value = TOKEN
        else:
            value = suggested(sizes[key])
        print(f"{key} = {value}")
    return 0


def read_peaks(log):
    """{board: {image: peak}} from the PASS lines in an emulation run's
    output, in the order they come."""
    peaks = {}
    for line in Path(log).read_text(errors="replace").splitlines():
        match = PASS_LINE.fullmatch(line.strip())
        if match:
            peaks.setdefault(match["board"], {})[match["image"]] = int(match["peak"])
    return peaks


def check_stack(log, budget_path, boards, all_boards=False):
    """Check each named board's stack peaks in `log` against its table in
    `budget_path`: every image the table lists must have passed, with a
    peak within its budget, and every image that passed must have one.
    With `all_boards`, `boards` is every board there is, and a table for
    any other is stale."""
    peaks = read_peaks(log)
    budgets = read_budget(budget_path)
    for board in boards:
        if board not in budgets:
            raise InputError(f"{budget_path} has no [{board}] table")
    print(f"Stack peaks against {budget_path}, in octets:")
    print(f"  {'board':<16}{'image':<14}{'peak':>8}{'budget':>8}")
    failed, loose = [], []
    for board in boards:
        budget, found = budgets[board], peaks.get(board, {})
        # The table's images, then any image that passed without a budget.
        for image in dict.fromkeys([*budget, *found]):
            name, peak, limit = f"{board}.{image}", found.get(image), budget.get(image)
            mark = ""
            if peak is None:
                failed.append((
                    name,
                    "no PASS line for it: its run failed, or no such image ran;"
                    " its stack went unchecked",
                ))
                mark = "  no PASS line"
            elif limit is None:
                failed.append((name, f"no budget for its stack peak of {peak} in {budget_path}"))
                mark = "  no budget"
            elif peak > limit:
                failed.append((
                    name,
                    f"stack peak {peak} is over its budget of {limit} in {budget_path};"
                    " if the growth is intended, raise the budget"
                    f" ({suggested(peak, STACK_HEADROOM)} leaves {STACK_HEADROOM:.0%} headroom)",
                ))
                mark = "  over budget"
            elif suggested(peak, STACK_HEADROOM) < limit * (1 - TIGHTEN):
                loose.append((name, peak, limit))
                mark = "  well under budget"
            print(f"  {board:<16}{image:<14}{'-' if peak is None else peak:>8}"
                  f"{'-' if limit is None else limit:>8}{mark}")
    stale = [board for board in budgets if board not in boards] if all_boards else []
    for board in stale:
        failed.append((
            board,
            f"{budget_path} has a table for it, but no board of that name ran;"
            " remove the table, or add the board to emulate-firmware.sh",
        ))
        for image, limit in budgets[board].items():
            print(f"  {board:<16}{image:<14}{'-':>8}{limit:>8}  no such board")
    # The table first, then the messages under it, when both go to one log.
    sys.stdout.flush()
    for name, peak, limit in loose:
        print(
            f"notice: {name}: stack peak {peak} needs a budget of only"
            f" {suggested(peak, STACK_HEADROOM)}, more than {TIGHTEN:.0%} under its"
            f" budget of {limit}; consider tightening it.",
            file=sys.stderr,
        )
    for name, problem in failed:
        print(f"error: {name}: {problem}. See docs/ci.md.", file=sys.stderr)
    return 1 if failed else 0


def suggest_stack(log):
    """A stack budget table for each board in `log`: each peak plus
    STACK_HEADROOM, rounded as sizes are."""
    peaks = read_peaks(log)
    if not peaks:
        raise InputError(f"{log} has no PASS line with a stack peak")
    for number, (board, images) in enumerate(peaks.items()):
        if number:
            print()
        print(f"[{board}]")
        for image, peak in images.items():
            print(f"{image} = {suggested(peak, STACK_HEADROOM)}")
    return 0


def main(argv):
    try:
        if len(argv) == 4 and argv[1] == "--suggest":
            return suggest(argv[2], argv[3])
        if len(argv) >= 5 and argv[1] == "--stack":
            all_boards = argv[2] == "--all-boards"
            rest = argv[3:] if all_boards else argv[2:]
            if len(rest) >= 3 and not rest[0].startswith("-"):
                return check_stack(rest[0], rest[1], rest[2:], all_boards)
        if len(argv) == 3 and argv[1] == "--stack-suggest":
            return suggest_stack(argv[2])
        if len(argv) == 4 and not argv[1].startswith("-"):
            return check(*argv[1:])
    except (OSError, InputError, struct.error, IndexError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(
        "usage: firmware_size.py <elf> <budget.toml> <table>\n"
        "       firmware_size.py --suggest <elf> <table>\n"
        "       firmware_size.py --stack [--all-boards] <run.log> <stack-budget.toml>"
        " <board>...\n"
        "       firmware_size.py --stack-suggest <run.log>",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
