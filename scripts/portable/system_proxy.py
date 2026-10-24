"""Pass desktop proxy settings to the client and its child processes at launch."""
import ast
import os
import subprocess
import sys
from urllib.parse import quote


SCHEMA = "org.gnome.system.proxy"
PROXIES = ("http_proxy", "https_proxy", "all_proxy")


def setting(schema, key):
    try:
        result = subprocess.run(["gsettings", "get", schema, key],
                                capture_output=True, text=True, timeout=2, check=True)
        value = result.stdout.strip().removeprefix("@as ")
        if value in ("true", "false"):
            return value == "true"
        return ast.literal_eval(value)
    except (OSError, subprocess.SubprocessError, ValueError, SyntaxError):
        return None


def endpoint(kind):
    schema = SCHEMA + "." + kind
    host, port = setting(schema, "host"), setting(schema, "port")
    if not host or not isinstance(port, int) or not 0 < port <= 65535:
        return None
    if ":" in host and not host.startswith("["):
        host = "[" + host + "]"
    authority = f"{host}:{port}"
    if kind == "http" and setting(schema, "use-authentication"):
        user = quote(setting(schema, "authentication-user") or "", safe="")
        password = quote(setting(schema, "authentication-password") or "", safe="")
        authority = f"{user}:{password}@{authority}"
    return "http://" + authority


def environment(inherited):
    result = dict(inherited)
    explicit = any(result.get(key) or result.get(key.upper()) for key in PROXIES)
    ignored = []
    if not explicit and setting(SCHEMA, "mode") == "manual":
        http = endpoint("http")
        same = setting(SCHEMA, "use-same-proxy")
        https = http if same and http else endpoint("https") or http
        fallback = http if same and http else None
        for name, value in zip(PROXIES, (http, https, fallback)):
            if value:
                result[name] = result[name.upper()] = value
        ignored = setting(SCHEMA, "ignore-hosts") or []
    # The owned app-server and stdio processes must still reach loopback directly.
    bypass = [result.get("no_proxy", ""), result.get("NO_PROXY", ""), *ignored,
              "localhost", "127.0.0.1", "::1"]
    hosts = dict.fromkeys(host.strip().removeprefix("*")
                          for group in bypass for host in group.split(",") if host.strip())
    result["no_proxy"] = result["NO_PROXY"] = ",".join(hosts)
    return result


if __name__ == "__main__":
    os.execvpe(sys.argv[1], sys.argv[1:], environment(os.environ))
