"""Offline checks for source verification and archive extraction boundaries."""
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("plugin_build", ROOT / "tools/build-plugin.py")
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class BuildSafety(unittest.TestCase):
    def test_rejects_checksum_mismatch_before_extracting(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            archive = base / "bad.tar.gz"
            archive.write_bytes(b"not a trusted source archive")
            with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
                build.prepare_source(base / "build", archive)
            self.assertFalse((base / "build/pipewire-1.6.8").exists())

    def test_rejects_unsupported_pipewire_before_compiling(self):
        with patch.object(build, "pkg", return_value="0.3.65"), patch.object(build.subprocess, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "supports PipeWire"):
                build.build_plugin(Path("unused"), Path("unused"))
            run.assert_not_called()

    def test_rejects_escaping_paths_and_links(self):
        for name, kind in (("../escape", tarfile.REGTYPE),
                           ("/absolute", tarfile.REGTYPE),
                           ("another-source/file", tarfile.REGTYPE),
                           ("pipewire-1.6.8/link", tarfile.SYMTYPE)):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                base = Path(directory)
                archive = base / "source.tar"
                with tarfile.open(archive, "w") as output:
                    member = tarfile.TarInfo(name)
                    member.type = kind
                    member.linkname = "../../outside" if kind == tarfile.SYMTYPE else ""
                    output.addfile(member)
                with self.assertRaisesRegex(RuntimeError, "Unexpected archive"):
                    build.extract_archive(archive, base / "extracted")

    def test_extracts_only_expected_regular_content(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            archive = base / "source.tar"
            with tarfile.open(archive, "w") as output:
                member = tarfile.TarInfo("pipewire-1.6.8/example.c")
                member.size = 4
                output.addfile(member, io.BytesIO(b"test"))
            build.extract_archive(archive, base / "extracted")
            self.assertEqual((base / "extracted/pipewire-1.6.8/example.c").read_bytes(), b"test")


if __name__ == "__main__":
    unittest.main()
