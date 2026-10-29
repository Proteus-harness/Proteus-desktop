"""Проверки лицензий на локальных синтетических крейтах, без Cargo."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "check_licenses", Path(__file__).with_name("check_licenses.py"),
)
check_licenses = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_licenses)


MIT_LICENSE = '''MIT License

Copyright (c) 2026 Example contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
'''


class CheckLicensesTests(unittest.TestCase):
    def setUp(self):
        fixtures = Path(__file__).resolve().parents[1] / "build"
        fixtures.mkdir(exist_ok=True)
        temporary = tempfile.TemporaryDirectory(
            prefix="license-tests-", dir=fixtures,
        )
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "Cargo.toml").write_text("[workspace]\n")
        for name in check_licenses.LICENSES.values():
            (self.root / name).write_text("Synthetic root license\n")

    def crate(self, path, license="MIT"):
        crate = self.root / path
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_text(
            f'[package]\nname = "fixture"\nversion = "0.1.0"\nlicense = "{license}"\n',
        )
        return crate

    def link_license(self, crate, license):
        name = check_licenses.LICENSES[license]
        (crate / name).symlink_to(
            Path(*[".."] * len(crate.relative_to(self.root).parts), name),
        )

    def test_mit_vendor_with_upstream_notice(self):
        crate = self.crate("experiments/example/vendor/third-party")
        (crate / "LICENSE").write_text(MIT_LICENSE)
        self.assertEqual(check_licenses.check(self.root), [])

    def test_mit_vendor_requires_complete_notice(self):
        crate = self.crate("vendor/third-party")
        invalid_notices = (
            None, "", "MIT License\nCopyright (c) 2026 Example\n",
            MIT_LICENSE.replace("Copyright (c) 2026 Example contributors", ""),
            MIT_LICENSE.replace(
                "to use, copy, modify, merge, publish, distribute, sublicense, and/or sell", "",
            ),
        )
        for text in invalid_notices:
            with self.subTest(text=text):
                if text is not None:
                    (crate / "LICENSE").write_text(text)
                errors = check_licenses.check(self.root)
                self.assertEqual(len(errors), 1, errors)
                self.assertIn(str(crate / "LICENSE"), errors[0])

    def test_mit_vendor_rejects_symlink_notice(self):
        crate = self.crate("vendor/third-party")
        (self.root / "upstream-license").write_text(MIT_LICENSE)
        (crate / "LICENSE").symlink_to(self.root / "upstream-license")
        self.assertIn("text itself", check_licenses.check(self.root)[0])

    def test_mit_app_crate_is_rejected(self):
        crate = self.crate("crates/vendor-adapter")
        (crate / "LICENSE").write_text(MIT_LICENSE)
        self.assertIn("license must be one of", check_licenses.check(self.root)[0])

    def test_vendor_is_not_skipped_and_requires_literal_mit(self):
        crate = self.crate("vendor/third-party", "MIT OR Apache-2.0")
        (crate / "LICENSE").write_text(MIT_LICENSE)
        self.assertIn("license must be one of", check_licenses.check(self.root)[0])
        (self.root / "Cargo.toml").write_text('[workspace.package]\nlicense = "MIT"\n')
        (crate / "Cargo.toml").write_text(
            '[package]\nname = "fixture"\nversion = "0.1.0"\nlicense.workspace = true\n',
        )
        self.assertIn("license must be one of", check_licenses.check(self.root)[0])

    def test_gpl_and_apache_links_still_required_including_vendor(self):
        for path, license in (("crates/app", "GPL-3.0-or-later"),
                              ("vendor/apache-library", "Apache-2.0")):
            crate = self.crate(path, license)
            name = check_licenses.LICENSES[license]
            self.link_license(crate, license)
            self.assertEqual(check_licenses.check(self.root), [])
            (crate / name).unlink()
            (crate / name).write_text("Copied license, not a symlink\n")
            self.assertIn("must be a symlink", check_licenses.check(self.root)[0])
            (crate / name).unlink()
            self.link_license(crate, license)

    def test_apache_crate_rejects_gpl_zed_source(self):
        crate = self.crate("vendor/apache-library", "Apache-2.0")
        self.link_license(crate, "Apache-2.0")
        source = crate / "lib.rs"
        source.write_text("// Адаптировано из Zed: crates/gpui/src/lib.rs\n")
        self.assertEqual(check_licenses.check(self.root), [])
        source.write_text("// Адаптировано из Zed: crates/editor/src/lib.rs\n")
        self.assertIn("GPL code of Zed", check_licenses.check(self.root)[0])


if __name__ == "__main__":
    unittest.main()
