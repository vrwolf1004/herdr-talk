import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

SRC = Path(__file__).resolve().parents[1] / "src"
SCREEN_AND_APP = ("i18n", "extend.settings_form", "extend.screen", "extend.tui", "talk", "herdr_api", "dock_layout")


class CoreBoundaryTest(unittest.TestCase):
    def test_importing_the_core_pulls_in_no_screen_or_locale_module(self):
        code = ("import sys; sys.path.insert(0, sys.argv[1]); "
                "from core import noise, ed25519, names, store, relay_key, relay_rest, events, chat, net, control, phone, relay_chat, relay_settings; "
                "print(','.join(name for name in sys.argv[2:] if name in sys.modules))")
        result = subprocess.run([sys.executable, "-c", code, str(SRC), *SCREEN_AND_APP],
                                capture_output=True, text=True, check=True, env=dict(os.environ))
        self.assertEqual(result.stdout.strip(), "", "the core must not import: " + result.stdout)


class FingerprintVectorTest(unittest.TestCase):
    """The fingerprint must match the vector file shared with mobile and Rust (relay_vectors_test.rs)."""

    def test_python_fingerprint_matches_the_shared_vector(self):
        from core import noise  # noqa: F401  (the core is importable from the test path)
        from core import store
        vectors = json.loads((Path(__file__).resolve().parents[2] / "mobile" / "test-vectors" / "relay.json")
                             .read_text(encoding="utf-8"))["outputs"]
        a, b = bytes.fromhex(vectors["initiatorPublic"]), bytes.fromhex(vectors["responderPublic"])
        self.assertEqual(store.fingerprint(a, b).replace("-", ""), vectors["fingerprint"].replace("-", ""))
        self.assertEqual(store.fingerprint(a, b), store.fingerprint(b, a))


if __name__ == "__main__":
    unittest.main()
