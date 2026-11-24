"""Toolchain preparation tests without network or a native compiler."""
import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import terminal_toolchain as tools


class TerminalToolchainTests(unittest.TestCase):
    def test_primary_archive_loses_one_root_and_preserves_content(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.tar.gz"
            output = Path(directory) / "flat.tar.gz"
            with tarfile.open(source, "w:gz") as archive:
                member = tarfile.TarInfo("package/build.zig.zon")
                contents = b".{ .name = .fixture }"
                member.size = len(contents)
                archive.addfile(member, io.BytesIO(contents))
            tools.flatten_archive(source, output)
            with tarfile.open(output) as archive:
                self.assertEqual(archive.getnames(), ["build.zig.zon"])
                self.assertEqual(archive.extractfile("build.zig.zon").read(), contents)

    def test_flatten_rejects_parent_path(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.tar.gz"
            with tarfile.open(source, "w:gz") as archive:
                archive.addfile(tarfile.TarInfo("package/../outside"))
            with self.assertRaisesRegex(RuntimeError, "unsafe source archive"):
                tools.flatten_archive(source, Path(directory) / "flat.tar.gz")

    def test_download_hash_failure_never_installs_an_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source"
            source.write_bytes(b"wrong contents")
            destination = Path(directory) / "downloaded"
            with self.assertRaisesRegex(RuntimeError, "SHA256 mismatch"):
                tools.download(source.as_uri(), destination, hashlib.sha256(b"expected").hexdigest())
            self.assertFalse(destination.exists())
            self.assertFalse(destination.with_name("downloaded.download").exists())

    def test_download_returns_actual_archive_digest_on_cold_and_warm_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            content = bytes(range(256)) * 4096
            source.write_bytes(content)
            expected = hashlib.sha256(content).hexdigest()
            url = source.as_uri()
            for pin in (expected, None):
                with self.subTest(pinned=pin is not None):
                    destination = root / f"archive-{pin is not None}"
                    self.assertEqual(tools.download(url, destination, pin), expected)
                    self.assertEqual(destination.read_bytes(), content)
                    self.assertFalse(destination.with_suffix(".download").exists())
                    with patch.object(tools.urllib.request, "urlopen") as open_url:
                        self.assertEqual(tools.download(url, destination, pin), expected)
                    open_url.assert_not_called()

    def test_corrupt_cached_archive_fails_without_fetching_again(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "cached"
            content = b"existing unexpected bytes"
            destination.write_bytes(content)
            expected = hashlib.sha256(b"pinned bytes").hexdigest()
            actual = hashlib.sha256(content).hexdigest()
            with patch.object(tools.urllib.request, "urlopen") as open_url:
                with self.assertRaises(RuntimeError) as raised:
                    tools.download("file:///unused", destination, expected)
            open_url.assert_not_called()
            self.assertEqual(str(raised.exception), f"{destination}: SHA256 mismatch: {actual}")
            self.assertEqual(destination.read_bytes(), content)

    def test_download_still_verifies_destination_after_rename(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.write_bytes(b"pinned contents")
            destination = root / "archive"
            temporary = root / "archive.download"
            expected = hashlib.sha256(source.read_bytes()).hexdigest()
            changed = b"synthetic replacement contents"
            actual = hashlib.sha256(changed).hexdigest()
            replace = Path.replace

            def change_destination(path, target):
                self.assertEqual(path, temporary)
                self.assertEqual(target, destination)
                output = replace(path, target)
                output.write_bytes(changed)
                return output

            with patch.object(Path, "replace", change_destination):
                with self.assertRaises(RuntimeError) as raised:
                    tools.download(source.as_uri(), destination, expected)
            self.assertEqual(str(raised.exception), f"{destination}: SHA256 mismatch: {actual}")
            self.assertFalse(temporary.exists())
            self.assertEqual(destination.read_bytes(), changed)

    def test_receipt_invalidates_when_a_verified_cache_file_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            cached = Path(directory) / "cached-package"
            cached.write_bytes(b"verified content")
            specification = {"version": "fixture"}
            receipt = {"specification": specification, "files": {str(cached): tools.fingerprint(cached)}}
            self.assertTrue(tools.receipt_valid(receipt, specification))
            cached.write_bytes(b"changed content length")
            self.assertFalse(tools.receipt_valid(receipt, specification))
            cached.unlink()
            self.assertFalse(tools.receipt_valid(receipt, specification))

    def test_explicit_toolchain_is_authoritative(self):
        with patch.object(tools, "_version", return_value="0.16.0"), patch.object(tools, "download") as download:
            with self.assertRaisesRegex(RuntimeError, "ZIG must be version 0.17.0"):
                tools.prepare(Path.cwd(), {"ZIG": "/custom/zig"})
            download.assert_not_called()


if __name__ == "__main__":
    unittest.main()
