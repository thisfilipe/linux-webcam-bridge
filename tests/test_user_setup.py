"""Exercise only temporary files; service/link operations are mocked."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("user_setup", ROOT / "tools/setup-user.py")
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


class UserSetup(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.home = Path(self.directory.name)
        state = self.home / ".local/state/jieli-camera"
        for name, value in {"HOME": self.home, "CONFIG": self.home / ".config", "STATE": state,
                            "MANIFEST": state / "installed-files.json",
                            "PLUGIN": self.home / ".local/lib/jieli-gstreamer/libgstpipewire.so"}.items():
            item = patch.object(setup, name, value)
            item.start()
            self.addCleanup(item.stop)
        self.plugin = self.home / "built.so"
        self.plugin.write_bytes(b"mock plugin")
        self.calls = patch.object(setup.subprocess, "run", side_effect=lambda command, **kw:
            SimpleNamespace(returncode=3 if command[0] == "systemctl" else 0, stdout="", stderr=""))
        self.run_mock = self.calls.start()
        self.addCleanup(self.calls.stop)
        self.uid = patch.object(setup.os, "geteuid", return_value=1000)
        self.uid.start()
        self.addCleanup(self.uid.stop)

    def invoke(self, *args):
        with patch("sys.argv", ["setup-user.py", "--plugin", str(self.plugin), *args]), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            setup.main()

    def test_default_plan_has_no_service_calls_or_writes(self):
        self.invoke()
        self.run_mock.assert_not_called()
        self.assertFalse(setup.MANIFEST.exists())

    def test_install_and_remove_exact_manifest(self):
        self.invoke("--install")
        paths = list(setup.files(self.plugin))
        self.assertTrue(all(path.is_file() for path in paths))
        manifest = json.loads(setup.MANIFEST.read_text())
        self.assertEqual(set(manifest), {str(path) for path in paths})
        self.invoke("--remove")
        self.assertFalse(any(path.exists() for path in paths))
        self.assertTrue(list((setup.STATE / "backups").iterdir()))

    def test_refuses_running_service(self):
        self.run_mock.side_effect = None
        self.run_mock.return_value = SimpleNamespace(returncode=0)
        with self.assertRaises(SystemExit):
            self.invoke("--install")
        self.assertFalse(setup.MANIFEST.exists())

    def test_refuses_conflict_then_replace_archives_it(self):
        target = next(iter(setup.files(self.plugin)))
        target.parent.mkdir(parents=True)
        target.write_text("local customization")
        with self.assertRaises(SystemExit):
            self.invoke("--install")
        self.assertEqual(target.read_text(), "local customization")
        self.invoke("--install", "--replace")
        self.assertTrue(any(path.read_text() == "local customization" for path in (setup.STATE / "backups").rglob(target.name)))

    def test_remove_preserves_modified_file(self):
        self.invoke("--install")
        target = next(iter(setup.files(self.plugin)))
        target.write_text("modified")
        with self.assertRaises(SystemExit):
            self.invoke("--remove")
        self.assertTrue(setup.MANIFEST.exists())
        self.assertEqual(target.read_text(), "modified")

    def test_refuses_symlink_destination(self):
        target = next(iter(setup.files(self.plugin)))
        target.parent.mkdir(parents=True)
        target.symlink_to(self.plugin)
        with self.assertRaises(SystemExit):
            self.invoke("--install", "--replace")
        self.assertEqual(self.plugin.read_bytes(), b"mock plugin")


if __name__ == "__main__":
    unittest.main()
