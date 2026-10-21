"""ScPemInspection.direct_issuer over an independent OpenSSL CA chain (#1846)."""
import ast
import subprocess
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import inspect_sc_pem, inspect_sc_pem_files
from test_endpoint_stub_parity import installed_stub, stub_classes


class ScDirectIssuerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="bacnet-direct-issuer-")
        cls.addClassCleanup(cls.temp.cleanup)
        root = cls.root = Path(cls.temp.name)

        def run(*args):
            subprocess.run(["openssl", *args], cwd=root, check=True,
                           capture_output=True, timeout=15)
        ec = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"]
        (root / "ca.ext").write_text(
            "basicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n")
        (root / "leaf.ext").write_text(
            "keyUsage=critical,digitalSignature\n"
            "extendedKeyUsage=serverAuth,clientAuth\n")
        run("req", "-x509", *ec, "-days", "25", "-subj", "/CN=Test root",
            "-keyout", "root.key", "-out", "root.pem",
            "-addext", "basicConstraints=critical,CA:TRUE",
            "-addext", "keyUsage=critical,keyCertSign,cRLSign")

        def signed(name, issuer, ext):
            run("req", "-new", *ec, "-subj", f"/CN=Test {name}",
                "-keyout", f"{name}.key", "-out", f"{name}.csr")
            run("x509", "-req", "-in", f"{name}.csr", "-CA", f"{issuer}.pem",
                "-CAkey", f"{issuer}.key", "-set_serial", str(len(name)), "-days", "25",
                "-extfile", ext, "-out", f"{name}.pem")
        signed("issuing", "root", "ca.ext")
        signed("leaf", "issuing", "leaf.ext")
        signed("direct", "root", "leaf.ext")
        cls.pem = {name: (root / f"{name}.pem").read_bytes()
                   for name in ("root", "issuing", "leaf", "direct")}

    def test_intermediate_signed_leaf_is_not_direct_against_the_root_alone(self):
        report = inspect_sc_pem(self.pem["leaf"], peer_trust_pem=self.pem["root"])
        self.assertEqual(report.direct_issuer, "not_direct")
        self.assertIsNone(report.direct_issuer_index)

    def test_directly_issued_leaf_names_its_ca_index(self):
        # Root first, issuing CA second: the leaf's issuer is at index 1.
        both = self.pem["root"] + self.pem["issuing"]
        report = inspect_sc_pem(self.pem["leaf"], peer_trust_pem=both)
        self.assertEqual(report.direct_issuer, "direct")
        self.assertEqual(report.direct_issuer_index, 1)
        # A leaf the root signed itself names index 0 of the same bundle.
        direct = inspect_sc_pem(self.pem["direct"], peer_trust_pem=both)
        self.assertEqual((direct.direct_issuer, direct.direct_issuer_index), ("direct", 0))
        on_disk = self.root / "bundle.pem"
        on_disk.write_bytes(both)
        files = inspect_sc_pem_files(self.root / "leaf.pem", peer_trust_path=on_disk)
        self.assertEqual((files.direct_issuer, files.direct_issuer_index), ("direct", 1))

    def test_without_peer_trust_it_is_not_assessed_and_the_getters_are_frozen(self):
        report = inspect_sc_pem(self.pem["leaf"])
        self.assertEqual(report.direct_issuer, "not_assessed")
        self.assertIsNone(report.direct_issuer_index)
        for name, value in (("direct_issuer", "direct"), ("direct_issuer_index", 0)):
            with self.assertRaises(AttributeError):
                setattr(report, name, value)

    def test_stub_declares_both_getters(self):
        shape = stub_classes(installed_stub())["ScPemInspection"]
        props = {node.name: ast.unparse(node.returns) for node in shape.body
                 if isinstance(node, ast.FunctionDef)}
        self.assertEqual(props["direct_issuer"],
                         "Literal['not_assessed', 'direct', 'not_direct', 'unknown']")
        self.assertEqual(props["direct_issuer_index"], "int | None")


if __name__ == "__main__":
    unittest.main()
