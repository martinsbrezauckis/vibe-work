"""Protocol formatter batching under Windows command limits, using a fake subprocess runner.

Run with the pinned Vibe Python; imports the real generator without writing generated files.
"""
import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("protocol_generate", ROOT / "tools/public/codegen/generate.py")
generate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generate)


class FormatBatches(unittest.TestCase):
    def test_long_paths_are_formatted_once_in_bounded_batches_with_the_same_flags(self):
        base = Path("C:/gate temp") / ("long directory " * 12)
        paths = [Path(f"generated/file {i}.{suffix}") for suffix in ("rs", "ts") for i in range(100)]
        with patch.object(generate, "_tool", side_effect=lambda name: f"C:/Program Files/tools/{name}"), \
                patch.object(generate.subprocess, "run") as run:
            generate.format_tree(base, paths)
        prefixes = {
            "rs": ["C:/Program Files/tools/rustfmt", "--edition", "2024", "--config-path", str(ROOT / "rustfmt.toml")],
            "ts": ["C:/Program Files/tools/pnpm", "--silent", "--dir", str(ROOT / "ui"), "exec", "prettier",
                   "--config", str(ROOT / "ui/.prettierrc.json"), "--write", "--log-level", "warn"],
        }
        for suffix, prefix in prefixes.items():
            with self.subTest(suffix=suffix):
                batches = [call for call in run.call_args_list if call.args[0][0] == prefix[0]]
                self.assertGreater(len(batches), 1)
                formatted = []
                for call in batches:
                    command = call.args[0]
                    self.assertEqual(command[:len(prefix)], prefix)
                    self.assertEqual(call.kwargs, {"check": True})
                    self.assertLess(len(subprocess.list2cmdline(command)), 6000)
                    formatted.extend(command[len(prefix):])
                self.assertEqual(formatted, [str(base / path) for path in paths if path.suffix == f".{suffix}"])


if __name__ == "__main__":
    unittest.main()
