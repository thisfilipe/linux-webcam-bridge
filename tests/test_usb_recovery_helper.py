#!/usr/bin/python3
"""Safety checks with fake device descriptors; never reset real hardware."""
import importlib.machinery
import importlib.util
from contextlib import redirect_stdout
import io
import os
from pathlib import Path
import stat
import struct
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "src/jieli-usb-reset"
loader = importlib.machinery.SourceFileLoader("jieli_reset_helper", str(SOURCE))
spec = importlib.util.spec_from_loader(loader.name, loader)
helper = importlib.util.module_from_spec(spec)
loader.exec_module(helper)


class HelperSafety(unittest.TestCase):
    def descriptor(self, vid=0x1224, pid=0x2a25):
        data = bytearray(18)
        data[:2] = b"\x12\x01"
        struct.pack_into("<HH", data, 8, vid, pid)
        return bytes(data)

    def info(self, *, major=189, mode=stat.S_IFCHR, uid=0):
        return SimpleNamespace(st_mode=mode, st_rdev=os.makedev(major, 910), st_uid=uid, st_nlink=1, st_ino=123)

    def test_rejects_arguments(self):
        with patch.object(helper.sys, "argv", ["helper", "8/15"]), patch.object(helper.os, "open") as opened:
            with self.assertRaises(SystemExit):
                helper.main()
            opened.assert_not_called()

    def test_rejects_unprivileged_execution(self):
        with patch.object(helper.sys, "argv", ["helper"]), patch.object(helper.os, "geteuid", return_value=1000):
            with self.assertRaises(SystemExit):
                helper.main()

    def verify_fd(self, info, descriptor, success):
        device = ("8-1", 8, 15, Path("/dev/bus/usb/008/015"))
        with patch.object(helper.os, "open", return_value=17), patch.object(helper.os, "fstat", return_value=info), patch.object(helper.os, "read", return_value=descriptor), patch.object(helper.os, "close") as closed:
            if success:
                fd, _ = helper.open_verified(device)
                self.assertEqual(fd, 17)
                closed.assert_not_called()
            else:
                with self.assertRaises(RuntimeError):
                    helper.open_verified(device)
                closed.assert_called_once_with(17)

    def test_valid_target(self):
        self.verify_fd(self.info(), self.descriptor(), True)

    def test_rejects_other_vendor(self):
        self.verify_fd(self.info(), self.descriptor(vid=0x3277), False)

    def test_rejects_other_product(self):
        self.verify_fd(self.info(), self.descriptor(pid=0x0029), False)

    def test_rejects_non_usb_major(self):
        self.verify_fd(self.info(major=81), self.descriptor(), False)

    def test_rejects_regular_file(self):
        self.verify_fd(self.info(mode=stat.S_IFREG), self.descriptor(), False)

    def test_rejects_truncated_descriptor(self):
        self.verify_fd(self.info(), b"\x12\x01", False)

    def test_rejects_address_change(self):
        info = self.info()
        info.st_rdev = os.makedev(189, 911)
        self.verify_fd(info, self.descriptor(), False)

    def test_rejects_untrusted_or_writable_lock(self):
        for info in (self.info(mode=stat.S_IFREG, uid=1000),
                     self.info(mode=stat.S_IFREG | 0o622)):
            with self.subTest(info=info), patch.object(helper.os, "open", return_value=10), patch.object(helper.os, "fstat", return_value=info), patch.object(helper.os, "close"), patch.object(helper.fcntl, "ioctl") as ioctl:
                with self.assertRaises(RuntimeError):
                    helper.reset()
                ioctl.assert_not_called()

    def test_discovery_requires_unique_matching_device(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            with patch.object(helper, "Path", lambda value: base):
                with self.assertRaises(RuntimeError):
                    helper.discover()
                for name in ("8-1", "8-2"):
                    device = base / name
                    device.mkdir()
                    for key, value in {"idVendor":"1224", "idProduct":"2a25", "busnum":"8", "devnum":"15"}.items():
                        (device / key).write_text(value)
                    if name == "8-1":
                        self.assertEqual(helper.discover()[1:3], (8, 15))
                with self.assertRaises(RuntimeError):
                    helper.discover()

    def test_only_reset_ioctl_and_cooldown(self):
        device = ("8-1", 8, 15, Path("/dev/bus/usb/008/015"))
        with patch.object(helper.os, "open", return_value=10), patch.object(helper.os, "fstat", return_value=self.info(mode=stat.S_IFREG)), patch.object(helper.os, "read", return_value=b""), patch.object(helper.os, "write"), patch.object(helper.os, "lseek"), patch.object(helper.os, "ftruncate"), patch.object(helper.os, "close"), patch.object(helper.fcntl, "flock"), patch.object(helper.fcntl, "ioctl") as ioctl, patch.object(helper, "discover", return_value=device), patch.object(helper, "open_verified", return_value=(20, self.info())), patch.object(helper.time, "monotonic", return_value=20):
            with redirect_stdout(io.StringIO()):
                helper.reset()
            ioctl.assert_called_once_with(20, 0x5514, 0)
            ioctl.reset_mock()
            with patch.object(helper.os, "read", return_value=b"19"):
                with self.assertRaises(RuntimeError):
                    helper.reset()
                ioctl.assert_not_called()


if __name__ == "__main__":
    unittest.main()
