import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

import relay_key  # noqa: E402
import sessions  # noqa: E402


class SessionsTest(unittest.TestCase):
    def setUp(self):
        self.base = tempfile.mkdtemp(prefix="herdr-talk-sessions-")

    def tearDown(self):
        shutil.rmtree(self.base, ignore_errors=True)

    def test_two_new_sessions_have_different_ids_and_keys(self):
        first_id, first = sessions.new_session(self.base)
        second_id, second = sessions.new_session(self.base)
        self.assertNotEqual(first_id, second_id)
        self.assertNotEqual(relay_key.load_or_create_short_id(first), relay_key.load_or_create_short_id(second))
        self.assertNotEqual(relay_key.load_or_create(first), relay_key.load_or_create(second))

    def test_the_default_folder_is_not_touched(self):
        sessions.new_session(self.base)
        self.assertFalse(os.path.exists(os.path.join(self.base, relay_key.KEY_FILE)))
        self.assertFalse(os.path.exists(os.path.join(self.base, "relay-signing.key")))

    def test_an_existing_session_must_exist_and_default_is_the_base_folder(self):
        session_id, folder = sessions.new_session(self.base)
        self.assertEqual(sessions.existing_session(self.base, session_id), (session_id, folder))
        self.assertEqual(sessions.existing_session(self.base, sessions.DEFAULT_SESSION), (sessions.DEFAULT_SESSION, self.base))
        with self.assertRaises(ValueError):
            sessions.existing_session(self.base, "ABCD2345")
        with self.assertRaises(ValueError):
            sessions.existing_session(self.base, "../x")

    def test_list_shows_only_session_folders(self):
        session_id, _ = sessions.new_session(self.base)
        os.makedirs(os.path.join(sessions.sessions_dir(self.base), "NotASession"))
        self.assertEqual([name for name, _ in sessions.list_sessions(self.base)], [session_id])

    def _node(self, session_id, pid=None):
        folder = os.path.join(sessions.sessions_dir(self.base), session_id)
        os.makedirs(folder)
        if pid is not None:
            open(os.path.join(folder, f"control-{pid}.json"), "w").close()
        return folder

    def test_dead_nodes_go_and_live_or_unknown_ones_stay(self):
        dead = self._node("ABCD2345", pid=4000001)
        live = self._node("ABCD2346", pid=4242)
        unknown = self._node("ABCD2347")
        failed = sessions.remove_dead_sessions(self.base, alive=lambda pid: pid == 4242)
        self.assertEqual(failed, [])
        self.assertFalse(os.path.exists(dead))
        self.assertTrue(os.path.exists(live))
        self.assertTrue(os.path.exists(unknown))

    def test_a_dead_node_with_keys_leaves_once_before_its_folder_goes(self):
        def keys(folder):
            relay_key.load_or_create(folder)
            relay_key.load_or_create_short_id(folder)
            return folder

        keyed = keys(self._node("ABCD2345", pid=4000001))
        keyless = self._node("ABCD2346", pid=4000001)
        live = keys(self._node("ABCD2347", pid=4242))
        left = []

        def leave(folder):
            self.assertTrue(os.path.isfile(os.path.join(folder, relay_key.KEY_FILE)), "the folder is still there when the row goes")
            left.append(folder)
            raise OSError("relay down")  # a failed delete never keeps the folder

        self.assertEqual(sessions.remove_dead_sessions(self.base, alive=lambda pid: pid == 4242, leave=leave), [])
        self.assertEqual(left, [keyed])
        self.assertFalse(os.path.exists(keyed))
        self.assertFalse(os.path.exists(keyless))
        self.assertTrue(os.path.exists(live))

    def test_the_own_session_is_kept_even_when_its_old_node_is_dead(self):
        mine = self._node("ABCD2345", pid=4000001)
        other = self._node("ABCD2346", pid=4000001)
        sessions._chosen = ("ABCD2345", mine)
        try:
            self.assertEqual(sessions.remove_dead_sessions(self.base, alive=lambda pid: False), [])
        finally:
            sessions._chosen = None
        self.assertTrue(os.path.exists(mine))
        self.assertFalse(os.path.exists(other))

    def test_the_default_folder_is_never_removed(self):
        control = os.path.join(self.base, "control-4000001.json")
        open(control, "w").close()
        self.assertEqual(sessions.remove_dead_sessions(self.base, alive=lambda pid: False), [])
        self.assertTrue(os.path.exists(control))

    def test_a_folder_that_cannot_be_removed_is_skipped_and_named(self):
        dead = self._node("ABCD2345", pid=4000001)

        def refuse(folder):
            raise PermissionError("locked")

        self.assertEqual(sessions.remove_dead_sessions(self.base, alive=lambda pid: False, remove=refuse), ["ABCD2345"])
        self.assertTrue(os.path.exists(dead))

    def test_an_own_folder_goes_when_no_other_live_node_is_in_it(self):
        mine = self._node("ABCD2345", pid=1)
        self.assertIsNone(sessions._remove_session_folder("ABCD2345", mine, 1, alive=lambda pid: False))
        self.assertFalse(os.path.exists(mine))
        shared = self._node("ABCD2346", pid=4242)
        self.assertIsNone(sessions._remove_session_folder("ABCD2346", shared, 1, alive=lambda pid: pid == 4242))
        self.assertTrue(os.path.exists(shared), "another live node still uses it")
        refused = sessions._remove_session_folder("ABCD2346", shared, 1, alive=lambda pid: False, remove=lambda folder: (_ for _ in ()).throw(PermissionError("locked")))
        self.assertEqual(refused, "ABCD2346")

    def test_the_default_folder_is_never_removed_on_stop(self):
        self.assertIsNone(sessions._remove_session_folder(sessions.DEFAULT_SESSION, self.base, 1, alive=lambda pid: False))
        self.assertTrue(os.path.isdir(self.base))

    def test_list_is_empty_before_any_session(self):
        self.assertEqual(sessions.list_sessions(self.base), [])


if __name__ == "__main__":
    unittest.main()
