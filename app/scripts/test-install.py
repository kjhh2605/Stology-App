"""Keep installation blocked by its target, but not by separate demo bundles."""
import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallProcessTests(unittest.TestCase):
    def check_paths(self, paths):
        result = subprocess.CompletedProcess([], 0, "\n".join(str(i) for i in paths), "")
        with patch.object(installer.subprocess, "run", return_value=result), patch.object(
            installer, "executable_path", side_effect=paths.get
        ):
            return installer.destination_running()

    def test_target_blocks_even_with_another_demo(self):
        self.assertTrue(self.check_paths({
            1: Path("/Users/example/Applications/Stology Demo.app/Contents/MacOS/stology-local"),
            2: Path("/Applications/Stology Local.app/Contents/MacOS/stology-local"),
        }))

    def test_separate_bundle_and_similar_prefix_are_not_targets(self):
        self.assertFalse(self.check_paths({
            1: Path("/Users/example/Applications/Stology Demo.app/Contents/MacOS/stology-local"),
            2: Path("/Applications/Stology Local.app QA/Contents/MacOS/stology-local"),
        }))

    def test_unknown_executable_remains_blocking(self):
        self.assertTrue(self.check_paths({1: None}))

    def test_no_running_processes(self):
        with patch.object(installer.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "", "")):
            self.assertFalse(installer.destination_running())

    def test_process_query_error_does_not_allow_install(self):
        with patch.object(installer.subprocess, "run", return_value=subprocess.CompletedProcess([], 2, "", "error")):
            with self.assertRaises(RuntimeError):
                installer.destination_running()


if __name__ == "__main__":
    unittest.main()
