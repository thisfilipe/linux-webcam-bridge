"""Portable root installer safety tests: temporary files and mocked subprocesses.

No sudo, modprobe, USB ioctl, service operation, package install, or root-owned
file is ever touched. File owner metadata is simulated for transaction tests.
"""
import contextlib
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import stat
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'tools' / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


usb = load('usb_installer', 'setup-usb-recovery.py')
v4l2 = load('v4l2_installer', 'setup-v4l2.py')
ORIGINAL_LSTAT = Path.lstat
ORIGINAL_FLAGS = usb.sys.flags


class Flags:
    def __init__(self, isolated):
        self.isolated = isolated

    def __getattr__(self, name):
        return getattr(ORIGINAL_FLAGS, name)


def root_metadata(path):
    info = list(ORIGINAL_LSTAT(path))
    info[4] = info[5] = 0
    return os.stat_result(info)


class USBInstaller(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.helper = self.root / 'usr/local/libexec/jieli-usb-reset'
        self.rule = self.root / 'etc/sudoers.d/90-jieli-camera-reset'
        self.source = self.root / 'source'
        self.source.write_bytes(usb.SOURCE.read_bytes())
        for name, value in (('HELPER', self.helper), ('RULE', self.rule), ('SOURCE', self.source)):
            self.enterContext(patch.object(usb, name, value))
        self.run = self.enterContext(patch.object(usb.subprocess, 'run', return_value=SimpleNamespace(returncode=0)))
        self.enterContext(patch.object(usb.pwd, 'getpwnam', return_value=SimpleNamespace(pw_name='alice', pw_uid=1000)))

    def invoke(self, *args, uid=0, isolated=True):
        with contextlib.ExitStack() as stack:
            stack.enter_context(patch('sys.argv', ['setup-usb-recovery.py', '--user', 'alice', *args]))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            stack.enter_context(contextlib.redirect_stderr(io.StringIO()))
            stack.enter_context(patch.object(usb.os, 'geteuid', return_value=uid))
            stack.enter_context(patch.object(usb.sys, 'flags', Flags(isolated)))
            stack.enter_context(patch.object(usb, 'check_directory_tree'))
            stack.enter_context(patch.object(Path, 'lstat', root_metadata))
            stack.enter_context(patch.object(usb.os, 'chown'))
            stack.enter_context(patch.object(usb.os, 'fchown'))
            usb.main()

    def test_hash_is_current_repository_helper(self):
        self.assertEqual(hashlib.sha256(self.source.read_bytes()).hexdigest(), usb.HELPER_SHA256)

    def test_plan_has_no_writes_or_commands_and_needs_no_root(self):
        self.source.unlink()
        self.invoke(uid=1000, isolated=False)
        self.run.assert_not_called()
        self.assertFalse(self.helper.parent.exists())
        self.assertFalse(self.rule.parent.exists())

    def test_user_argument_required(self):
        with patch('sys.argv', ['setup-usb-recovery.py']), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            usb.main()
        self.run.assert_not_called()

    def test_conservative_names_reject_sudoers_injection(self):
        for name in ('root:ALL', 'alice ALL', '#alice', 'alice\nroot', 'user.name', '-alice', 'alice$', 'Álice'):
            with self.subTest(name=name), self.assertRaises(SystemExit):
                usb.validated_user(name)

    def test_root_unknown_and_alias_accounts_rejected(self):
        for account in (SimpleNamespace(pw_name='alice', pw_uid=0), SimpleNamespace(pw_name='alias', pw_uid=1000)):
            with patch.object(usb.pwd, 'getpwnam', return_value=account), self.assertRaises(SystemExit):
                usb.validated_user('alice')
        with patch.object(usb.pwd, 'getpwnam', side_effect=KeyError), self.assertRaises(SystemExit):
            usb.validated_user('unknown')

    def test_mutations_require_root_and_isolation(self):
        for args in ({'uid': 1000}, {'isolated': False}):
            with self.subTest(args=args), self.assertRaises(SystemExit):
                self.invoke('--install', **args)
        self.run.assert_not_called()

    def test_install_reinstall_remove_exact_content(self):
        self.invoke('--install')
        self.assertEqual(self.helper.read_bytes(), self.source.read_bytes())
        self.assertEqual(stat.S_IMODE(self.helper.stat().st_mode), 0o755)
        self.assertEqual(self.rule.read_text(), usb.rule_text('alice'))
        self.assertIn('jieli-usb-reset ""\n', self.rule.read_text())
        self.assertEqual(stat.S_IMODE(self.rule.stat().st_mode), 0o440)
        self.invoke('--install')
        self.invoke('--remove')
        self.assertFalse(self.helper.exists())
        self.assertFalse(self.rule.exists())
        self.assertTrue(all(call.args[0][0] == '/usr/sbin/visudo' for call in self.run.call_args_list))

    def test_modified_source_cannot_install_or_remove(self):
        self.source.write_bytes(b'unreviewed')
        for mode in ('--install', '--remove'):
            with self.subTest(mode=mode), self.assertRaises(SystemExit):
                self.invoke(mode)
        self.assertFalse(self.helper.exists())

    def test_modified_rule_preserves_both_files_on_remove(self):
        self.invoke('--install')
        self.rule.chmod(0o600)
        self.rule.write_text('local customization\n')
        self.rule.chmod(0o440)
        with self.assertRaises(SystemExit):
            self.invoke('--remove')
        self.assertTrue(self.helper.exists())
        self.assertEqual(self.rule.read_text(), 'local customization\n')

    def test_modified_helper_preserved_on_update(self):
        self.invoke('--install')
        self.helper.write_bytes(b'previous release')
        with self.assertRaises(SystemExit):
            self.invoke('--install')
        self.assertEqual(self.helper.read_bytes(), b'previous release')

    def test_helper_symlink_and_hardlink_refused(self):
        self.helper.parent.mkdir(parents=True)
        self.helper.symlink_to(self.source)
        with self.assertRaises(SystemExit):
            self.invoke('--install')
        self.helper.unlink()
        os.link(self.source, self.helper)
        self.source.chmod(0o755)
        with self.assertRaises(SystemExit):
            self.invoke('--install')

    def test_visudo_failure_does_not_publish_helper_or_rule(self):
        self.run.side_effect = RuntimeError('mock visudo rejection')
        with self.assertRaises(RuntimeError):
            self.invoke('--install')
        self.assertFalse(self.helper.exists())
        self.assertFalse(self.rule.exists())


class PathSafety(unittest.TestCase):
    def directory(self, mode, uid=0):
        path = Mock()
        path.parents = ()
        path.lstat.return_value = SimpleNamespace(st_mode=mode, st_uid=uid)
        return path

    def test_directory_symlink_nondirectory_nonroot_or_writable_refused(self):
        for module in (usb, v4l2):
            for mode, uid in ((stat.S_IFLNK | 0o777, 0), (stat.S_IFREG | 0o644, 0),
                              (stat.S_IFDIR | 0o775, 0), (stat.S_IFDIR | 0o755, 1000)):
                with self.subTest(module=module.__name__, mode=mode, uid=uid), self.assertRaises(SystemExit):
                    module.check_directory_tree(self.directory(mode, uid))
            module.check_directory_tree(self.directory(stat.S_IFDIR | 0o755))

    def test_writable_ancestor_is_checked_even_with_safe_leaf(self):
        for module in (usb, v4l2):
            child = self.directory(stat.S_IFDIR | 0o755)
            parent = self.directory(stat.S_IFDIR | 0o777)
            child.parents = (parent,)
            with self.subTest(module=module.__name__), self.assertRaises(SystemExit):
                module.check_directory_tree(child)

    def test_regular_file_ownership_mode_and_link_count_required(self):
        for module, mode in ((usb, 0o755), (v4l2, 0o644)):
            for changed in ({'st_uid': 1000}, {'st_gid': 1000}, {'st_nlink': 2},
                            {'st_mode': stat.S_IFREG | 0o777}, {'st_mode': stat.S_IFLNK | 0o777}):
                info = {'st_uid': 0, 'st_gid': 0, 'st_nlink': 1, 'st_mode': stat.S_IFREG | mode}
                info.update(changed)
                path = Mock()
                path.lstat.return_value = SimpleNamespace(**info)
                with self.subTest(module=module.__name__, changed=changed), self.assertRaises(SystemExit):
                    if module is usb:
                        module.check_existing(path, b'exact', mode)
                    else:
                        module.check_existing(path, 'exact')


class V4L2Installer(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.config = {self.root / 'etc/modprobe.d/90-jieli-loopback.conf': v4l2.CONFIG[next(iter(v4l2.CONFIG))],
                       self.root / 'etc/modules-load.d/90-jieli-loopback.conf': v4l2.MARKER + 'v4l2loopback\n'}
        values = {'CONFIG': self.config, 'CONFIG_DIRS': tuple(path.parent for path in self.config),
                  'MODULE': self.root / 'sys/module/v4l2loopback', 'NODE': self.root / 'dev/video10',
                  'CLASS': self.root / 'sys/class/video4linux/video10',
                  'VIRTUAL': self.root / 'sys/devices/virtual/video4linux'}
        for name, value in values.items():
            self.enterContext(patch.object(v4l2, name, value))
        self.run = self.enterContext(patch.object(v4l2.subprocess, 'run', return_value=SimpleNamespace(stdout='v4l2loopback\n', returncode=0)))
        self.runtime = self.enterContext(patch.object(v4l2, 'verify_runtime'))

    def invoke(self, *args, uid=0, isolated=True):
        with contextlib.ExitStack() as stack:
            stack.enter_context(patch('sys.argv', ['setup-v4l2.py', *args]))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            stack.enter_context(patch.object(v4l2.os, 'geteuid', return_value=uid))
            stack.enter_context(patch.object(v4l2.sys, 'flags', Flags(isolated)))
            stack.enter_context(patch.object(v4l2, 'check_directory_tree'))
            stack.enter_context(patch.object(Path, 'lstat', root_metadata))
            stack.enter_context(patch.object(v4l2.os, 'fchown'))
            v4l2.main()

    def test_default_plan_has_no_commands_or_writes(self):
        self.invoke(uid=1000, isolated=False)
        self.run.assert_not_called()
        self.assertFalse(any(path.parent.exists() for path in self.config))

    def test_mutations_require_root_isolated(self):
        for mode in ('--install', '--remove'):
            for args in ({'uid': 1000}, {'isolated': False}):
                with self.subTest(mode=mode, args=args), self.assertRaises(SystemExit):
                    self.invoke(mode, **args)
        self.run.assert_not_called()

    def test_install_only_modinfo_modprobe_and_exact_configs(self):
        self.invoke('--install')
        self.assertEqual(self.run.call_args_list[0].args[0], ['/usr/sbin/modinfo', '-F', 'name', 'v4l2loopback'])
        self.assertEqual(self.run.call_args_list[1].args[0], ['/usr/sbin/modprobe', '--ignore-install', 'v4l2loopback', *v4l2.OPTIONS])
        self.runtime.assert_called_once()
        for path, text in self.config.items():
            self.assertEqual(path.read_text(), text)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o644)

    def test_remove_does_not_unload_or_remove_packages(self):
        self.invoke('--install')
        self.run.reset_mock()
        v4l2.MODULE.mkdir(parents=True)
        self.invoke('--remove')
        self.assertFalse(any(path.exists() for path in self.config))
        self.assertTrue(v4l2.MODULE.exists())
        self.run.assert_not_called()

    def test_modified_config_prevents_partial_removal(self):
        self.invoke('--install')
        second = list(self.config)[1]
        second.write_text('customized\n')
        with self.assertRaises(SystemExit):
            self.invoke('--remove')
        self.assertTrue(all(path.exists() for path in self.config))

    def test_occupied_video10_refused(self):
        v4l2.NODE.parent.mkdir(parents=True)
        v4l2.NODE.touch()
        with self.assertRaises(SystemExit):
            self.invoke('--install')
        self.run.assert_not_called()

    def test_broken_video10_symlink_refused(self):
        v4l2.NODE.parent.mkdir(parents=True)
        v4l2.NODE.symlink_to('/nonexistent-fixture-target')
        with self.assertRaises(SystemExit):
            self.invoke('--install')
        self.run.assert_not_called()

    def test_unrelated_loaded_module_refused(self):
        v4l2.MODULE.mkdir(parents=True)
        with self.assertRaises(SystemExit):
            self.invoke('--install')
        self.run.assert_not_called()

    def test_loaded_owned_module_idempotent_without_modprobe(self):
        self.invoke('--install')
        self.run.reset_mock()
        v4l2.MODULE.mkdir(parents=True)
        self.invoke('--install')
        self.run.assert_not_called()
        self.assertEqual(self.runtime.call_count, 2)

    def test_foreign_config_refused_but_comments_ignored(self):
        directory = next(iter(self.config)).parent
        directory.mkdir(parents=True)
        foreign = directory / 'other.conf'
        foreign.write_text('# v4l2loopback is documented here\n')
        v4l2.check_other_configs()
        foreign.write_text('install v4l2loopback /bin/false\n')
        with self.assertRaises(SystemExit):
            self.invoke('--install')
        self.run.assert_not_called()

    def test_foreign_config_continuation_cannot_hide_module_name(self):
        directory = next(iter(self.config)).parent
        directory.mkdir(parents=True)
        foreign = directory / 'continued.conf'
        foreign.write_text('options v4l2' + chr(92) + chr(10) + 'loopback devices=2' + chr(10))
        with self.assertRaises(SystemExit):
            self.invoke('--install')
        self.run.assert_not_called()

    def test_module_load_failure_writes_no_config(self):
        self.run.side_effect = [SimpleNamespace(stdout='v4l2loopback\n'), RuntimeError('mock load failure')]
        with self.assertRaises(RuntimeError):
            self.invoke('--install')
        self.assertFalse(any(path.exists() for path in self.config))

    def test_wrong_installed_module_identity_refused(self):
        self.run.return_value.stdout = 'different_module\n'
        with self.assertRaises(SystemExit):
            self.invoke('--install')
        self.assertEqual(self.run.call_count, 1)
        self.assertFalse(any(path.exists() for path in self.config))


class RuntimeIdentity(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        for name, value in {'MODULE': root / 'module', 'NODE': root / 'video10',
                            'CLASS': root / 'class/video10', 'VIRTUAL': root / 'virtual'}.items():
            self.enterContext(patch.object(v4l2, name, value))
        (v4l2.VIRTUAL / 'video10').mkdir(parents=True)
        (v4l2.VIRTUAL / 'video10/name').write_text(v4l2.LABEL)
        (v4l2.VIRTUAL / 'video10/dev').write_text('81:4\n')
        v4l2.CLASS.parent.mkdir()
        v4l2.CLASS.symlink_to(v4l2.VIRTUAL / 'video10')
        (v4l2.MODULE / 'parameters').mkdir(parents=True)
        for name, value in {'video_nr': '10,-1,-1', 'exclusive_caps': 'Y,N,N',
                            'max_buffers': '8', 'max_width': '1280', 'max_height': '720'}.items():
            (v4l2.MODULE / 'parameters' / name).write_text(value)
        original = Path.stat
        def metadata(path, *args, **kwargs):
            if path == v4l2.NODE:
                return SimpleNamespace(st_mode=stat.S_IFCHR | 0o660, st_rdev=os.makedev(81, 4))
            return original(path, *args, **kwargs)
        self.enterContext(patch.object(Path, 'stat', metadata))

    def test_exact_runtime_accepted(self):
        v4l2.verify_runtime()

    def test_kernel_minor_need_not_match_video_basename(self):
        self.assertEqual((v4l2.CLASS / 'dev').read_text().strip(), '81:4')
        v4l2.verify_runtime()

    def test_mismatched_and_invalid_sysfs_device_numbers_refused(self):
        for number in ('81:10', '1:4', 'garbage', '81:-4', '81:4:1'):
            with self.subTest(number=number):
                (v4l2.CLASS / 'dev').write_text(number)
                with self.assertRaises(SystemExit):
                    v4l2.verify_runtime()

    def test_foreign_label_extra_device_and_wrong_parameters_refused(self):
        name = v4l2.CLASS / 'name'
        name.write_text('unrelated camera')
        with self.assertRaises(SystemExit):
            v4l2.verify_runtime()
        name.write_text(v4l2.LABEL)
        (v4l2.VIRTUAL / 'video11').mkdir()
        with self.assertRaises(SystemExit):
            v4l2.verify_runtime()
        (v4l2.VIRTUAL / 'video11').rmdir()
        (v4l2.MODULE / 'parameters/max_width').write_text('1920')
        with self.assertRaises(SystemExit):
            v4l2.verify_runtime()


if __name__ == '__main__':
    unittest.main(verbosity=2)
