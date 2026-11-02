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
