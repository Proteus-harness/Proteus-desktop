"""Exercise proxy propagation through the real portable launcher and release gate."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ASSETS = Path(__file__).resolve().parent / "portable"
spec = importlib.util.spec_from_file_location("system_proxy", ASSETS / "system_proxy.py")
proxy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proxy)


class PortableProxyTests(unittest.TestCase):
    def settings(self, values):
        return patch.object(proxy, "setting", side_effect=lambda schema, key: values.get((schema, key)))

    def test_manual_proxy_auth_ipv6_and_loopback_bypass(self):
        values = {(proxy.SCHEMA, "mode"): "manual", (proxy.SCHEMA, "use-same-proxy"): True,
                  (proxy.SCHEMA, "ignore-hosts"): ["*.example.invalid", "localhost,::1"],
                  (proxy.SCHEMA + ".http", "host"): "::1", (proxy.SCHEMA + ".http", "port"): 10808,
                  (proxy.SCHEMA + ".http", "use-authentication"): True,
                  (proxy.SCHEMA + ".http", "authentication-user"): "user@name",
                  (proxy.SCHEMA + ".http", "authentication-password"): "a:/ b"}
        with self.settings(values):
            env = proxy.environment({})
        for name in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"):
            self.assertEqual(env[name], "http://user%40name:a%3A%2F%20b@[::1]:10808")
        self.assertEqual(set(env["NO_PROXY"].split(",")),
                         {".example.invalid", "localhost", "127.0.0.1", "::1"})

    def test_separate_https_and_socks_and_http_fallback(self):
        values = {(proxy.SCHEMA, "mode"): "manual", (proxy.SCHEMA, "use-same-proxy"): False}
        for kind, host, port in [("http", "http.invalid", 8080), ("https", "tls.invalid", 8443),
                                 ("socks", "socks.invalid", 1080)]:
            values[(proxy.SCHEMA + "." + kind, "host")] = host
            values[(proxy.SCHEMA + "." + kind, "port")] = port
        with self.settings(values):
            env = proxy.environment({})
            self.assertEqual(env["HTTPS_PROXY"], "http://tls.invalid:8443")
            self.assertEqual(env["ALL_PROXY"], "socks5h://socks.invalid:1080")
            values[(proxy.SCHEMA + ".https", "port")] = 0
            self.assertEqual(proxy.environment({})["HTTPS_PROXY"], "http://http.invalid:8080")

    def test_explicit_environment_takes_precedence(self):
        with patch.object(proxy, "setting") as setting:
            env = proxy.environment({"https_proxy": "http://explicit.invalid:8080", "NO_PROXY": "internal.invalid"})
        setting.assert_not_called()
        self.assertEqual(env["https_proxy"], "http://explicit.invalid:8080")
        self.assertNotIn("HTTP_PROXY", env)
        self.assertIn("internal.invalid", env["NO_PROXY"])
        self.assertIn("127.0.0.1", env["no_proxy"])

    def test_disabled_or_unavailable_settings_do_not_enable_saved_proxy(self):
        for mode in ("none", "auto", None):
            with self.settings({(proxy.SCHEMA, "mode"): mode,
                                (proxy.SCHEMA + ".http", "host"): "saved.invalid",
                                (proxy.SCHEMA + ".http", "port"): 8080}):
                env = proxy.environment({})
            self.assertFalse(any(key.lower() in proxy.PROXIES for key in env))

    def test_launcher_forwards_arguments_and_config_environment_to_child(self):
        with tempfile.TemporaryDirectory(prefix="proteus-launcher-test-") as temporary:
            root = Path(temporary)
            portable, config = root / "Portable with spaces", root / "config"
            for relative in ("bin", "share/configs"):
                (portable / relative).mkdir(parents=True)
            (portable / "share/configs/config.toml").write_text("# synthetic launcher profile\n")
            config.mkdir()
            for relative in ("proteus-native", "system_proxy.py"):
                target = portable / ("share" if relative.endswith(".py") else "") / relative
                shutil.copy2(ASSETS / relative, target)
            binary = portable / "bin/proteus-native"
            binary.write_text(f"#!{sys.executable}\nimport json,os,sys\n"
                              "print(json.dumps({'args':sys.argv[1:],'proxy':os.environ.get('HTTPS_PROXY'),"
                              "'bypass':os.environ['NO_PROXY'],'server':os.environ['PROTEUS_BIN']}))\n")
            binary.chmod(0o755)
            (config / "environment").write_text("export HTTPS_PROXY='http://configured.invalid:8080'\n")
            env = {key: value for key, value in os.environ.items()
                   if key.lower() not in (*proxy.PROXIES, "no_proxy")}
            env["PROTEUS_CONFIG_HOME"] = str(config)
            args = ["--choose-workspace", "папка с пробелами", "literal;$(value)"]
            result = subprocess.run([portable / "proteus-native", *args], cwd=root,
                                    env=env, capture_output=True, text=True, check=True, timeout=10)
            output = json.loads(result.stdout)
            self.assertEqual(output["args"], args)
            self.assertEqual(output["proxy"], "http://configured.invalid:8080")
            self.assertIn("127.0.0.1", output["bypass"])
            self.assertEqual(output["server"], str(portable / "bin/proteus"))
            # The same launcher must use current desktop settings on every launch.
            (config / "environment").unlink()
            settings = root / "settings.json"
            fake = portable / "bin/gsettings"
            fake.write_text(f"#!{sys.executable}\nimport json,os,sys\nfrom pathlib import Path\n"
                            "v=json.loads(Path(os.environ['TEST_PROXY_SETTINGS']).read_text()).get('|'.join(sys.argv[2:]))\n"
                            "print('true' if v is True else 'false' if v is False else '@as []' if v==[] else repr(v))\n")
            fake.chmod(0o755)
            env["TEST_PROXY_SETTINGS"] = str(settings)
            env["PATH"] = str(portable / "bin") + os.pathsep + env["PATH"]
            for mode, port in [("manual", 10808), ("none", 10808), ("manual", 10809)]:
                settings.write_text(json.dumps({proxy.SCHEMA + "|mode": mode,
                    proxy.SCHEMA + "|use-same-proxy": True, proxy.SCHEMA + "|ignore-hosts": [],
                    proxy.SCHEMA + ".http|host": "127.0.0.1", proxy.SCHEMA + ".http|port": port}))
                result = subprocess.run([portable / "proteus-native", *args], cwd=root,
                                        env=env, capture_output=True, text=True, check=True, timeout=10)
                output = json.loads(result.stdout)
                expected = f"http://127.0.0.1:{port}" if mode == "manual" else None
                self.assertEqual(output["proxy"], expected)
                self.assertEqual(output["args"], args)


if __name__ == "__main__":
    unittest.main()
