"""Exercise the compiled-module workspace runner without Cargo."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class WorkspaceRunner(unittest.TestCase):
    def run_runner(self, extra, *, build_artifact=True):
        (ROOT / "target").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="workspace-runner-", dir=ROOT / "target") as scratch:
            scratch = Path(scratch)
            (scratch / "cargo").write_text(
                '#!/bin/sh\n'
                'printf "CALL\\n" >> "$MOCK_LOG"\n'
                'for arg do printf "ARG:%s\\n" "$arg" >> "$MOCK_LOG"; done\n'
                'if [ "$1" = test ] && [ -f "$TINYMEMES_MODULE" ]; then printf "MODULE_EXISTS:yes\\n" >> "$MOCK_LOG"; fi\n'
                'if [ "$1" = build ] && [ "$MOCK_BUILD_ARTIFACT" = yes ]; then\n'
                '  mkdir -p "$MOCK_TARGET/debug"\n'
                '  : > "$MOCK_TARGET/debug/libtinymemes_module.so"\n'
                'fi\n'
                'printf "MODULE:%s\\n" "$TINYMEMES_MODULE" >> "$MOCK_LOG"\n'
                'printf \'{"target_directory":"unused"}\\n\'\n'
            )
            (scratch / "python3").write_text(
                '#!/bin/sh\ncat >/dev/null\nprintf "%s\\n" "$MOCK_TARGET"\n'
            )
            (scratch / "uname").write_text('#!/bin/sh\nprintf "Linux\\n"\n')
            for command in ("cargo", "python3", "uname"):
                (scratch / command).chmod(0o755)
            log = scratch / "calls.log"
            environment = dict(os.environ)
            environment.update(
                PATH=f"{scratch}:/usr/local/bin:/usr/bin:/bin",
                MOCK_LOG=str(log),
                MOCK_TARGET=str(scratch / "build"),
                MOCK_BUILD_ARTIFACT="yes" if build_artifact else "no",
            )
            shell = os.environ.get("WORKSPACE_TEST_SHELL", "bash")
            runner = os.environ.get("WORKSPACE_TEST_RUNNER", str(ROOT / "scripts/test-workspace.sh"))
            result = subprocess.run(
                [shell, runner, *extra],
                env=environment, text=True, capture_output=True, check=False,
            )
            calls = [block.splitlines() for block in log.read_text().split("CALL\n")[1:]]
            self.assertEqual(calls[0], ["ARG:metadata", "ARG:--locked", "ARG:--no-deps", "ARG:--format-version", "ARG:1", "MODULE:"])
            self.assertEqual(
                calls[1][:-1],
                ["ARG:build", "ARG:--locked", "ARG:--no-default-features", "ARG:-p", "ARG:tinymemes-module"],
            )
            self.assertEqual(calls[1][-1], "MODULE:")
            if build_artifact:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls[2][0:3], ["ARG:test", "ARG:--locked", "ARG:--workspace"])
                self.assertEqual(calls[2][3:-2], [f"ARG:{arg}" for arg in extra])
                module = Path(calls[2][-1][len("MODULE:"):])
                self.assertEqual(module.name, "libtinymemes_module.so")
                self.assertEqual(calls[2][-2], "MODULE_EXISTS:yes")
                self.assertEqual(module.parent.parent, scratch / "build" / "compiled-fixtures")
            else:
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("compiled TinyMemes module is missing", result.stderr)
                self.assertEqual(len(calls), 2, "tests must not run without the cdylib")

    def test_feature_args_reach_workspace_tests_without_changing_module_abi(self):
        self.run_runner(["--no-default-features"])

    def test_missing_build_artifact_stops_before_tests(self):
        self.run_runner([], build_artifact=False)


if __name__ == "__main__":
    unittest.main()
