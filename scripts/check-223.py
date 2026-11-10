#!/usr/bin/env python3
"""Validate Standard 223 exports against the vendored Open223 shapes (#2436).

    python -m venv .venv-223 && .venv-223/bin/pip install pyshacl==0.40.2
    .venv-223/bin/python scripts/check-223.py [FILE.ttl ...] [--qudt FILE ...]

With no files, checks the golden files of `bacnet export 223-experimental`
(crates/bacnet-ops/tests/s223/*.ttl); a live export (`-o FILE`) can be
passed too. Each file is merged with testdata/open223/223p.ttl and run
through pySHACL in advanced mode, so the ontology's SHACL rules apply
before its shapes.

The ontology imports the QUDT vocabulary, which isn't vendored, and its
shapes require a property's unit to be a qudt:Unit and its quantity kind a
qudt:QuantityKind. Without --qudt the script types every IRI the graph uses
with qudt:hasUnit or qudt:hasQuantityKind as one, so the rest of the shapes
can be checked; whether those IRIs exist in QUDT is then unchecked. Give
--qudt the QUDT unit and quantity-kind vocabulary files to check them.

Only results about the export's own nodes count. Exits 1 on any
Violation, after listing every Violation and Warning; Info results are
counted. Needs no network.

The RDF libraries stay in this script's virtual environment: no crate
depends on one.
"""

import argparse
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ONTOLOGY = ROOT / "testdata" / "open223" / "223p.ttl"
GOLDEN = ROOT / "crates" / "bacnet-ops" / "tests" / "s223"

try:
    from pyshacl import validate
    from rdflib import Graph, Namespace, RDF
except ImportError:
    sys.exit("check-223.py needs pySHACL: pip install pyshacl==0.40.2 in a virtual environment")

SH = Namespace("http://www.w3.org/ns/shacl#")
QUDT = Namespace("http://qudt.org/schema/qudt/")


def stub_qudt(graph):
    """Type each unit and quantity kind IRI the graph uses, as QUDT would."""
    for predicate, kind in ((QUDT.hasUnit, QUDT.Unit), (QUDT.hasQuantityKind, QUDT.QuantityKind)):
        for term in set(graph.objects(None, predicate)):
            graph.add((term, RDF.type, kind))


def check(path, ontology, qudt):
    """Validate one file; return its results on the export's own nodes."""
    model = Graph().parse(path, format="turtle")
    ours = set(model.subjects())
    data = Graph()
    for graph in (model, ontology, *qudt):
        data += graph
    if not qudt:
        stub_qudt(data)
    _, results, _ = validate(
        data,
        shacl_graph=ontology,
        advanced=True,
        allow_warnings=True,
        inference="none",
    )
    found = []
    for result in results.subjects(RDF.type, SH.ValidationResult):
        focus = results.value(result, SH.focusNode)
        if focus not in ours:
            continue
        severity = str(results.value(result, SH.resultSeverity)).rsplit("#", 1)[-1]
        message = str(results.value(result, SH.resultMessage) or results.value(result, SH.sourceShape))
        found.append((severity, str(focus), message))
    return len(model), sorted(found)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("files", nargs="*", type=Path, help="Turtle files (default: the golden files)")
    parser.add_argument("--ontology", type=Path, default=ONTOLOGY, help="the 223 ontology (default: the vendored copy)")
    parser.add_argument("--qudt", type=Path, action="append", default=[], help="a QUDT vocabulary file to load")
    args = parser.parse_args()
    files = args.files or sorted(GOLDEN.glob("*.ttl"))
    if not files:
        sys.exit(f"no Turtle files in {GOLDEN}")
    ontology = Graph().parse(args.ontology, format="turtle")
    qudt = [Graph().parse(path) for path in args.qudt]
    if not qudt:
        print("QUDT not loaded: unit and quantity-kind IRIs are typed by a stub, not checked")
    failed = 0
    totals = Counter()
    for path in files:
        triples, found = check(path, ontology, qudt)
        counts = Counter(severity for severity, _, _ in found)
        totals.update(counts)
        summary = ", ".join(f"{counts[s]} {s}" for s in ("Violation", "Warning", "Info"))
        print(f"{path.name}: {triples} triples, {summary}")
        for severity, focus, message in found:
            if severity in ("Violation", "Warning"):
                print(f"  {severity}: {focus}: {message}")
        failed += counts["Violation"] > 0
    print(f"{len(files)} files: " + ", ".join(f"{totals[s]} {s}" for s in ("Violation", "Warning", "Info")))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
