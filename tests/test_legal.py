# SPDX-License-Identifier: BSD-3-Clause
import os
import re
import subprocess
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def read(name):
    with open(os.path.join(ROOT, name), encoding="utf-8") as f:
        return f.read()


class Legal(unittest.TestCase):
    def test_license_files(self):
        self.assertIn("Copyright (c) 2026, the fork-preflight contributors", read("LICENSE"))
        third = read("LICENSE-THIRD-PARTY")
        self.assertIn("PX4 Development Team", third)
        self.assertIn("not affiliated with or endorsed by the PX4 project", third)

    def test_disclaimer(self):
        text = " ".join(read("DISCLAIMER.md").split())
        for phrase in ['"as is", without warranty of any kind', "accept no liability and no responsibility",
                       "not affiliated with or endorsed by the PX4 project", "PX4 is a trademark of Dronecode Project, Inc."]:
            self.assertIn(phrase, text)
        self.assertIn("DISCLAIMER.md", read("README.md"))

    def test_no_trademark_in_file_names(self):
        files = subprocess.run(["git", "-C", ROOT, "ls-files", "--cached", "--others", "--exclude-standard"],
                               capture_output=True, text=True, check=True).stdout.split()
        self.assertTrue(files)
        self.assertEqual([f for f in files if re.search("px4", f, re.I)], [])
