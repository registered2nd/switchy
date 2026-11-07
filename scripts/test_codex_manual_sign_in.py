"""Exercise Switchy's generated Codex login wrappers without real authentication."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
CASES = [
    ["login"],
    ["login", "status"],
    ["login", "help"],
    ["login", "--help"],
    ["login", "--with-api-key"],
    ["login", "--with-access-token"],
    ["login", "--device-auth"],
    ["login", "-c", "example=true"],
    ["--help"],
]


def set_marker(path, enabled):
    if enabled:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("")
    else:
        path.unlink(missing_ok=True)


def verify_posix():
    text = (ROOT / "src-tauri/src/proxy/codex_engine.rs").read_text()
    template = re.search(r'const CODEX_SHELL: &str = r#"(.*?)"#;', text, re.S).group(1)
    shells = [name for name in ["bash", "zsh"] if shutil.which(name)]
    assert "bash" in shells, "Bash is required for the POSIX verifier"
    count = 0
    with tempfile.TemporaryDirectory(prefix="switchy_codex_login_") as temporary:
        home = Path(temporary)
        config = home / "custom codex's config"
        launcher = config / "switchy/codex.sh"
        launcher.parent.mkdir(parents=True)
        quoted = "'" + str(config).replace("'", "'\"'\"'") + "'"
        launcher.write_text(template.replace("__SWITCHY_CODEX_CONFIG_DIR__", quoted))
        binary_dir = home / "bin"
        binary_dir.mkdir()
        stub = binary_dir / "codex"
        # Exit 23 simulates an unsupported option: the wrapper must never retry.
        stub.write_text("#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$SWITCHY_TEST_CAPTURE\"\nexit 23\n")
        stub.chmod(0o700)
        capture = home / "capture"
        env = os.environ.copy()
        env.update(HOME=str(home), PATH=str(binary_dir) + os.pathsep + env["PATH"],
                   SWITCHY_TEST_CAPTURE=str(capture))
        env.pop("CODEX_HOME", None)
        for shell in shells:
            for manual in [False, True]:
                set_marker(config / "switchy/manual_sign_in", manual)
                for status_sync in [False, True]:
                    set_marker(config / "switchy-sessions/enabled", status_sync)
                    for args in CASES:
                        capture.unlink(missing_ok=True)
                        result = subprocess.run([shell, "-c", '. "$1"; shift; codex "$@"',
                                                 "switchy-test", str(launcher), *args],
                                                env=env, capture_output=True, text=True, timeout=5)
                        expected = ["login", "--device-auth"] if manual and args == ["login"] else args
                        assert capture.read_text().splitlines() == expected, (shell, manual, status_sync, args, result.stderr)
                        assert result.returncode == 23, (shell, manual, status_sync, args, result.returncode)
                        count += 1
    print(f"PASS POSIX ({', '.join(shells)}): {count} isolated invocations; exact login rewrite, pass-through, no server or retry")


def verify_windows():
    count = 0
    with tempfile.TemporaryDirectory(prefix="switchy_codex_login_") as temporary:
        home = Path(temporary)
        config = home / "custom codex's config"
        launcher = config / "switchy/codex.ps1"
        launcher.parent.mkdir(parents=True)
        shutil.copyfile(ROOT / "src-tauri/src/proxy/codex_windows.ps1", launcher)
        binary_dir = home / "bin"
        binary_dir.mkdir()
        (binary_dir / "codex.ps1").write_text(
            "ConvertTo-Json -Compress -InputObject @($args) | Add-Content -LiteralPath $env:SWITCHY_TEST_CAPTURE\nexit 23\n")
        capture = home / "capture"
        runner = home / "run.ps1"
        runner.write_text(". $env:SWITCHY_TEST_LAUNCHER\n$swTestArgs = ($env:SWITCHY_TEST_ARGS | ConvertFrom-Json).args\ncodex @swTestArgs\nexit $LASTEXITCODE\n")
        env = os.environ.copy()
        env.update(PATH=str(binary_dir) + os.pathsep + env["PATH"],
                   SWITCHY_TEST_CAPTURE=str(capture), SWITCHY_TEST_LAUNCHER=str(launcher))
        env.pop("CODEX_HOME", None)
        for manual in [False, True]:
            set_marker(config / "switchy/manual_sign_in", manual)
            for status_sync in [False, True]:
                set_marker(config / "switchy-sessions/enabled", status_sync)
                for args in CASES:
                    capture.unlink(missing_ok=True)
                    env["SWITCHY_TEST_ARGS"] = json.dumps({"args": args})
                    result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(runner)],
                                            env=env, capture_output=True, text=True, timeout=5)
                    expected = ["login", "--device-auth"] if manual and args == ["login"] else args
                    records = capture.read_text(encoding="utf-8-sig").splitlines()
                    assert len(records) == 1 and json.loads(records[0]) == expected, (manual, status_sync, args, records, result.stderr)
                    assert result.returncode == 23, (manual, status_sync, args, result.returncode)
                    count += 1
    print(f"PASS Windows PowerShell: {count} isolated invocations; exact login rewrite, pass-through, no server or retry", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--posix", action="store_true")
    parser.add_argument("--wsl-distro", help="Optional WSL distribution for the POSIX checks")
    options = parser.parse_args()
    if os.name == "nt" and not options.posix:
        verify_windows()
        posix_path = "/mnt/" + str(Path(__file__).resolve()).replace("\\", "/").replace(":", "", 1)
        posix_path = posix_path[:5] + posix_path[5].lower() + posix_path[6:]
        wsl = ["wsl.exe"] + (["-d", options.wsl_distro] if options.wsl_distro else [])
        subprocess.run([*wsl, "--", "python3", posix_path, "--posix"], check=True, timeout=30)
    else:
        verify_posix()
