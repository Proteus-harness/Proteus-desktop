"""Pure Python runner controls: subprocesses and source resolvers are all fakes."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import Mock, patch


RUNNER_PATH = Path(__file__).with_name("test.py")
ROOT = Path("/synthetic/desktop")
EXTENSIONS = Path("/synthetic/extensions")


def load_runner():
    sources = types.ModuleType("sources")
    sources.ROOT = ROOT
    sources.resolve = Mock(return_value=EXTENSIONS)
    limits = types.ModuleType("limits")
    limits.limit = Mock()
    licenses = types.ModuleType("check_licenses")
    licenses.check = Mock(return_value=[])
    subprocess = types.ModuleType("subprocess")
    subprocess.run = Mock()
    subprocess.call = Mock(return_value=0)
    runtime = types.ModuleType("runtime_paths")
    runtime.require_extensions = Mock()
    stubs = {"runtime_paths": runtime, "sources": sources, "limits": limits,
             "check_licenses": licenses, "subprocess": subprocess}
    spec = importlib.util.spec_from_file_location("candidate_runner", RUNNER_PATH)
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, stubs):
        spec.loader.exec_module(module)
    return module


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.runner = load_runner()

    def test_exact_headless_selectors(self):
        for args in (
            ["-p", "proteus-diff"], ["--package", "proteus-tabs"],
            ["--package=proteus-diff"], ["-pproteus-tabs"],
            ["-p", "proteus-diff", "-p", "proteus-tabs"],
            ["-p", "proteus-diff", "-p", "proteus-diff"],
            ["-p", "proteus-tabs", "--lib", "tests::history"],
            ["tests::history", "--package=proteus-tabs", "--offline"],
            ["-p", "proteus-diff", "--", "--nocapture", "--test-threads=2"],
            ["-p", "proteus-tabs", "--", "--package", "proteus-native"],
        ):
            with self.subTest(args=args):
                self.assertTrue(self.runner.headless_only(args))

    def test_known_non_selection_cargo_arguments(self):
        for extra in (
            ["--lib"], ["--doc"], ["--no-run"], ["--no-fail-fast"],
            ["--locked", "--frozen", "--offline"], ["-q"], ["-v"], ["-vv"],
            ["--quiet"], ["--verbose"], ["--jobs", "2"], ["-j", "2"],
            ["-j2"], ["--jobs=2"], ["--color", "never"],
            ["--message-format=json"], ["--message-format", "json"],
        ):
            with self.subTest(extra=extra):
                self.assertTrue(self.runner.headless_only(["-p", "proteus-diff", *extra]))

    def test_no_implicit_headless_selection(self):
        for args in ([], ["--lib"], ["tests::diff"], ["--help"], ["full"],
                     ["--", "-p", "proteus-diff"], ["--", "--package=proteus-tabs"]):
            with self.subTest(args=args):
                self.assertFalse(self.runner.headless_only(args))

    def test_unknown_and_mixed_packages_keep_full_path(self):
        for package in ("proteus-native", "proteus-client-common", "proteus-*",
                        "proteus-diff@0.1.0", "*", "", "proteus-diff,proteus-tabs"):
            for selector in (["-p", package], ["--package=" + package]):
                with self.subTest(selector=selector):
                    self.assertFalse(self.runner.headless_only(selector))
                    self.assertFalse(self.runner.headless_only(["-p", "proteus-tabs", *selector]))

    def test_broader_or_ambiguous_cargo_options_keep_full_path(self):
        for extra in (
            ["--workspace"], ["--all"], ["--exclude", "proteus-native"],
            ["--manifest-path", "/other/Cargo.toml"],
            ["--manifest-path=/other/Cargo.toml"], ["--config", "x=y"],
            ["--config=x=y"], ["-C", "/other"], ["-Z", "unstable-options"],
            ["--features", "proteus-native/foo"], ["--all-features"],
            ["--target", "triple"], ["--unknown"], ["-vp", "proteus-native"],
            ["--packageX=proteus-native"], ["--exclude=proteus-native"],
        ):
            with self.subTest(extra=extra):
                self.assertFalse(self.runner.headless_only(["-p", "proteus-diff", *extra]))

    def test_malformed_options_are_not_proven_headless(self):
        for extra in (["-p"], ["--package"], ["--package="], ["-p", "--lib"],
                      ["--jobs"], ["--color", ""], ["--color=--package"],
                      ["one", "two"], ["--jobs", "-p", "proteus-tabs"]):
            with self.subTest(extra=extra):
                self.assertFalse(self.runner.headless_only(["-p", "proteus-diff", *extra]))


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.runner = load_runner()
        self.env = patch.dict(os.environ, {"PATH": "/synthetic/bin"}, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)
        self.stderr = io.StringIO()
        redirect = contextlib.redirect_stderr(self.stderr)
        redirect.__enter__()
        self.addCleanup(redirect.__exit__, None, None, None)

    def test_import_has_no_execution_side_effects(self):
        self.runner.limit.assert_not_called()
        self.runner.resolve.assert_not_called()
        self.runner.check.assert_not_called()
        self.runner.subprocess.run.assert_not_called()
        self.runner.subprocess.call.assert_not_called()

    def test_default_gate_is_desktop_only(self):
        self.assertEqual(self.runner.main([]), 0)
        self.runner.limit.assert_called_once_with()
        self.runner.resolve.assert_not_called()
        self.runner.check.assert_called_once_with(ROOT)
        self.runner.require_extensions.assert_not_called()
        self.runner.subprocess.run.assert_not_called()
        cargo = self.runner.subprocess.call.call_args
        self.assertEqual(cargo.args[0], ["cargo", "test", "--locked", "--workspace"])
        self.assertEqual(cargo.kwargs["env"]["RUST_TEST_THREADS"], "4")
        self.assertNotIn("PROTEUS_TEST_EXTENSIONS_SOURCE", cargo.kwargs["env"])

    def test_extensions_are_ready_only_and_explicit(self):
        self.assertEqual(self.runner.main(["--extensions", "--release", "--", "--nocapture"]), 0)
        self.runner.resolve.assert_called_once_with("extensions")
        self.runner.subprocess.run.assert_not_called()
        self.runner.require_extensions.assert_called_once()
        cargo = self.runner.subprocess.call.call_args
        self.assertEqual(cargo.args[0], ["cargo", "test", "--locked", "--workspace", "--release", "--", "--nocapture", "--include-ignored"])
        self.assertEqual(cargo.kwargs["env"]["PROTEUS_TEST_EXTENSIONS_SOURCE"], str(EXTENSIONS))
        self.assertEqual(cargo.kwargs["env"]["PROTEUS_EXTENSIONS_DIR"], str(EXTENSIONS / "packages"))
        self.assertEqual([c.args[0] for c in self.runner.check.call_args_list], [ROOT, EXTENSIONS])

    def test_build_extensions_is_the_only_build_opt_in(self):
        self.runner.main(["--build-extensions", "--release"])
        env = self.runner.subprocess.call.call_args.kwargs["env"]
        self.runner.subprocess.run.assert_called_once_with(
            [sys.executable, str(EXTENSIONS / "package.py"), "--dev"], check=True, env=env)
        self.assertEqual(self.runner.subprocess.call.call_args.args[0][-2:], ["--", "--include-ignored"])

    def test_explicit_ignored_and_libtest_boundary(self):
        for flag in ("--ignored", "--include-ignored"):
            runner = load_runner()
            runner.main(["--extensions", "--", flag])
            self.assertEqual(runner.subprocess.call.call_args.args[0].count(flag), 1)
        runner = load_runner()
        runner.main(["--", "--extensions", "--build-extensions"])
        runner.resolve.assert_not_called()
        runner.subprocess.run.assert_not_called()
        self.assertEqual(runner.subprocess.call.call_args.args[0][-3:], ["--", "--extensions", "--build-extensions"])

    def test_missing_ready_extensions_fail_before_cargo(self):
        self.runner.require_extensions.side_effect = SystemExit("missing executable; --build-extensions")
        with self.assertRaisesRegex(SystemExit, "--build-extensions"):
            self.runner.main(["--extensions"])
        self.runner.subprocess.run.assert_not_called()
        self.runner.subprocess.call.assert_not_called()

    def test_headless_skips_only_external_setup(self):
        args = ["-p", "proteus-diff", "--lib", "tests::parse", "--", "--nocapture"]
        self.runner.limit.side_effect = lambda: os.environ.setdefault("CARGO_BUILD_JOBS", "2")
        self.assertEqual(self.runner.main(args), 0)
        self.runner.limit.assert_called_once_with()
        self.runner.resolve.assert_not_called()
        self.runner.check.assert_called_once_with(ROOT)
        self.runner.subprocess.run.assert_not_called()
        cargo = self.runner.subprocess.call.call_args
        self.assertEqual(cargo.args[0], ["cargo", "test", "--locked", *args])
        self.assertEqual(cargo.kwargs["cwd"], ROOT)
        self.assertEqual(cargo.kwargs["env"]["CARGO_BUILD_JOBS"], "2")
        self.assertEqual(cargo.kwargs["env"]["RUST_TEST_THREADS"], "4")
        self.assertNotIn("PROTEUS_TEST_EXTENSIONS_SOURCE", cargo.kwargs["env"])
        self.assertNotIn("PROTEUS_EXTENSIONS_DIR", cargo.kwargs["env"])
        self.assertIn("not the client workspace gate", self.stderr.getvalue())

    def test_explicit_package_and_manifest_requests_are_never_widened(self):
        for args in (["-p", "proteus-diff", "--workspace"],
                     ["-p", "proteus-tabs", "-p", "proteus-native"],
                     ["-p", "proteus-tabs", "--release"],
                     ["--package=proteus-client-common", "--profile", "release"],
                     ["--manifest-path", "/selected/Cargo.toml"],
                     ["-p", "proteus-diff", "--features", "chosen"],
                     ["-p", "proteus-diff", "--manifest-path", "elsewhere/Cargo.toml"]):
            with self.subTest(args=args):
                runner = load_runner()
                self.assertEqual(runner.main(args), 0)
                runner.resolve.assert_not_called()
                runner.subprocess.run.assert_not_called()
                self.assertEqual(runner.subprocess.call.call_args.args[0],
                                 ["cargo", "test", "--locked", *args])

    def test_libtest_filter_does_not_become_a_cargo_package_selection(self):
        runner = load_runner()
        runner.main(["--", "-p", "proteus-tabs"])
        self.assertEqual(runner.subprocess.call.call_args.args[0],
                         ["cargo", "test", "--locked", "--workspace", "--", "-p", "proteus-tabs"])

    def test_root_license_failure_blocks_both_paths(self):
        for args in ([], ["-p", "proteus-tabs"]):
            with self.subTest(args=args):
                runner = load_runner()
                runner.check.side_effect = lambda path: ["root license failure"] if path == ROOT else []
                with self.assertRaisesRegex(SystemExit, "root license failure"):
                    runner.main(args)
                runner.subprocess.run.assert_not_called()
                runner.subprocess.call.assert_not_called()

    def test_extension_license_failure_blocks_full_gate(self):
        self.runner.check.side_effect = lambda path: ["extension license failure"] if path == EXTENSIONS else []
        with self.assertRaisesRegex(SystemExit, "extension license failure"):
            self.runner.main(["--extensions"])
        self.runner.subprocess.run.assert_not_called()
        self.runner.subprocess.call.assert_not_called()

    def test_extension_build_failure_never_runs_cargo(self):
        self.runner.subprocess.run.side_effect = RuntimeError("fake build failed")
        with self.assertRaisesRegex(RuntimeError, "fake build failed"):
            self.runner.main(["--build-extensions"])
        self.runner.subprocess.call.assert_not_called()

    def test_nonzero_cargo_status_is_preserved(self):
        for args in ([], ["--package=proteus-tabs"]):
            with self.subTest(args=args):
                runner = load_runner()
                runner.subprocess.call.return_value = 101
                self.assertEqual(runner.main(args), 101)

    def test_environment_settings_are_not_overwritten(self):
        os.environ.update(RUST_TEST_THREADS="1", CARGO_BUILD_JOBS="1",
                          PROTEUS_TEST_EXTENSIONS_SOURCE="/prior/extensions")
        self.runner.main(["-pproteus-tabs"])
        env = self.runner.subprocess.call.call_args.kwargs["env"]
        self.assertEqual(env["RUST_TEST_THREADS"], "1")
        self.assertEqual(env["CARGO_BUILD_JOBS"], "1")
        self.assertEqual(env["PROTEUS_TEST_EXTENSIONS_SOURCE"], "/prior/extensions")
        self.assertNotIn("PROTEUS_EXTENSIONS_DIR", env)


if __name__ == "__main__":
    unittest.main()
