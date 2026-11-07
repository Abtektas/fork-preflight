# SPDX-License-Identifier: BSD-3-Clause
import json
import os
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "generator"))

import generate  # noqa: E402


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


class CommandParsing(unittest.TestCase):
    def tree(self, files):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        for rel, text in files.items():
            write(os.path.join(tmp.name, rel), text)
        return tmp.name

    def test_main_of_every_module_call(self):
        tree = self.tree({
            "src/modules/a/CMakeLists.txt": "px4_add_module(\n\tMODULE modules__a\n\tMAIN a_cmd\n\tCOMPILE_FLAGS $<$<CONFIG:Debug>:-O0>\n\tSRCS a.cpp\n)\n",
            "src/drivers/b/CMakeLists.txt": "# px4_add_module(MAIN commented_out)\npx4_add_module(MODULE drivers__b MAIN b_cmd SRCS b.cpp)\n",
            "src/lib/c/CMakeLists.txt": "px4_add_library(c c.cpp)\n",
            "boards/x/y/src/CMakeLists.txt": "px4_add_module(MODULE z MAIN a_cmd SRCS z.cpp) # MAIN not_this\n",
        })
        self.assertEqual(generate.commands_data(tree), {
            "a_cmd": ["boards/x/y/src/CMakeLists.txt", "src/modules/a/CMakeLists.txt"],
            "b_cmd": ["src/drivers/b/CMakeLists.txt"],
        })

    def test_non_literal_main_stops(self):
        tree = self.tree({"src/a/CMakeLists.txt": "px4_add_module(MODULE a MAIN ${NAME})\n"})
        with self.assertRaisesRegex(generate.GenerateError, "not a literal"):
            generate.commands_data(tree)

    def test_unbalanced_call_stops(self):
        tree = self.tree({"src/a/CMakeLists.txt": "px4_add_module(MODULE a MAIN a\n"})
        with self.assertRaisesRegex(generate.GenerateError, "unbalanced"):
            generate.commands_data(tree)

    def test_hash_inside_quotes_is_not_a_comment(self):
        self.assertEqual(generate.strip_cmake_comments('set(X "a#b") # c'), 'set(X "a#b") ')


class KconfigLoading(unittest.TestCase):
    def tree(self, files):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        for rel, text in files.items():
            write(os.path.join(tmp.name, "tree", rel), text)
        return os.path.join(tmp.name, "tree"), os.path.join(tmp.name, "generated")

    def test_global_and_board_symbols(self):
        tree, generated = self.tree({
            "Kconfig": 'comment "Vendor: $(VENDOR)"\nconfig A\n\tbool "a"\nmenuconfig B\n\tbool "b"\nchoice\n\tprompt "c"\nconfig C1\n\tbool "c1"\nconfig C2\n\tbool "c2"\nendchoice\n'
                       + generate.BOARD_SOURCE + "\nsource \"src/*/Kconfig\"\n",
            "src/m/Kconfig": 'config M\n\tbool "m"\n\tselect UNDEFINED_BUT_SELECTED\n',
            "boards/v/one/Kconfig": 'config BOARD_ONE_X\n\tbool "x"\n',
            "boards/v/two/Kconfig": 'config A\n\tbool "a again"\n',
        })
        data = generate.kconfig_data(tree, generated)
        self.assertEqual(data["symbols"], ["A", "B", "C1", "C2", "M"])
        self.assertEqual(data["board_symbols"], {"v/one": ["BOARD_ONE_X"]})
        self.assertEqual(data["generated"], [])

    def test_board_kconfig_without_osource_is_not_loaded(self):
        tree, generated = self.tree({
            "Kconfig": 'config A\n\tbool "a"\n',
            "boards/v/one/Kconfig": 'config BOARD_ONE_X\n\tbool "x"\n',
        })
        data = generate.kconfig_data(tree, generated)
        self.assertEqual((data["symbols"], data["board_symbols"]), (["A"], {}))

    def test_unknown_variable_stops(self):
        tree, generated = self.tree({"Kconfig": 'source "$(SOMETHING_NEW)/Kconfig"\n'})
        with self.assertRaisesRegex(generate.GenerateError, "SOMETHING_NEW"):
            generate.kconfig_data(tree, generated)


class ReleaseData(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with open(os.path.join(ROOT, "generator", "releases.json"), encoding="utf-8") as f:
            cls.manifest = json.load(f)
        cls.data = {}
        for entry in cls.manifest["releases"]:
            with open(os.path.join(ROOT, "data", "releases", f"{entry['release']}.json"), encoding="utf-8") as f:
                cls.data[entry["release"]] = json.load(f)

    def test_one_file_per_release_and_nothing_else(self):
        files = sorted(os.listdir(os.path.join(ROOT, "data", "releases")))
        self.assertEqual(files, sorted(f"{e['release']}.json" for e in self.manifest["releases"]))
        for entry in self.manifest["releases"]:
            d = self.data[entry["release"]]
            self.assertEqual((d["release"], d["source"]["commit"]), (entry["release"], entry["commit"]))

    def test_files_are_canonical_and_sorted(self):
        for release, d in self.data.items():
            with open(os.path.join(ROOT, "data", "releases", f"{release}.json"), encoding="utf-8") as f:
                self.assertEqual(f.read(), generate.dumps(d), release)
            for values in [d["kconfig"]["symbols"], *d["kconfig"]["board_symbols"].values(), *d["commands"].values()]:
                self.assertEqual(values, sorted(set(values)), release)
            if d["parameters"] is not None:
                self.assertEqual(d["parameters"]["names"], sorted(set(d["parameters"]["names"])), release)

    def test_schema(self):
        try:
            import jsonschema
        except ImportError:
            self.skipTest("jsonschema is not installed (generator/requirements-test.txt)")
        with open(os.path.join(ROOT, "data", "schema", "release.schema.json"), encoding="utf-8") as f:
            schema = json.load(f)
        jsonschema.Draft202012Validator.check_schema(schema)
        validator = jsonschema.Draft202012Validator(schema)
        for release, d in self.data.items():
            errors = [e.message for e in validator.iter_errors(d)]
            self.assertEqual(errors, [], release)

    # Known answers, measured independently on 2026-11-06 (contrib notes,
    # DESIGN.md "Evidence") with PX4's own loadconfig.py and git history.

    def symbols(self, release):
        return set(self.data[release]["kconfig"]["symbols"])

    def test_kconfig_removed_v1_17_to_v1_18_rc1(self):
        removed = self.symbols("v1.17.0") - self.symbols("v1.18.0-rc1")
        self.assertEqual({s for s in removed if not s.startswith("ZENOH_PUBSUB_")},
                         {"COMMON_GPIO", "DRIVERS_POWER_MONITOR_PM_SELECTOR_AUTERION"})

    def test_kconfig_removed_v1_18_rc1_to_main(self):
        removed = self.symbols("v1.18.0-rc1") - self.symbols("main")
        self.assertEqual({s for s in removed if not s.startswith("ZENOH_PUBSUB_")},
                         {"BOARD_UAVCAN_PERIPHERALS", "BOARD_UAVCAN_TIMER_OVERRIDE", "NUM_MISSION_ITMES_SUPPORTED"})

    def test_generated_zenoh_catalog_is_loaded_on_main(self):
        main = self.data["main"]["kconfig"]
        self.assertEqual(main["generated"], ["src/modules/zenoh/Kconfig.topics"])
        zenoh = {s for s in main["symbols"] if s.startswith("ZENOH_PUBSUB_")}
        self.assertGreater(len(zenoh), 250)
        self.assertIn("ZENOH_PUBSUB_VEHICLE_STATUS", zenoh)
        self.assertEqual(self.data["main"]["kconfig"]["board_symbols"]["nxp/mr-navq95"][0], "BOARD_NAVQ95_IO23_BUZZER")

    def test_symbols_already_undefined_in_releases(self):
        for release in ["v1.17.0", "v1.18.0-rc1", "main"]:
            for symbol in ["MODULES_ROVER_POS_CONTROL", "MODULES_FW_POS_CONTROL", "UXRCE_DDS_CLIENT_SERIAL", "BOARD_VENDOR"]:
                self.assertNotIn(symbol, self.symbols(release), (release, symbol))

    def test_commands_removed_v1_17_to_v1_18_rc1(self):
        a, b = set(self.data["v1.17.0"]["commands"]), set(self.data["v1.18.0-rc1"]["commands"])
        self.assertEqual(a - b, {"core_heater", "pm_selector_auterion"})
        self.assertIn("src/modules/commander/CMakeLists.txt", self.data["main"]["commands"]["commander"])

    def test_parameters(self):
        rc1 = self.data["v1.18.0-rc1"]["parameters"]
        self.assertEqual(len(rc1["removed"]), 60)
        self.assertEqual(rc1["removed"]["SENS_TEMP_ID"], {"kind": "renamed", "successors": ["HEATER1_SENS_ID"]})
        self.assertEqual(set(self.data["v1.17.0"]["parameters"]["names"]) - set(rc1["names"]), set(rc1["removed"]))
        self.assertIsNone(self.data["main"]["parameters"])
        self.assertTrue(any("no snapshot of main" in n for n in self.data["main"]["notes"]))

    def test_license_line_per_release(self):
        self.assertEqual(self.data["v1.17.0"]["license"]["copyright"], "Copyright (c) 2012 - 2023, PX4 Development Team")
        self.assertEqual(self.data["main"]["license"]["copyright"], "Copyright (c) 2012 - 2025, PX4 Development Team")


@unittest.skipUnless(os.environ.get("FORK_PREFLIGHT_PX4") and os.environ.get("FORK_PREFLIGHT_PARAM_LINEAGE"),
                     "set FORK_PREFLIGHT_PX4 and FORK_PREFLIGHT_PARAM_LINEAGE to the two clones")
class Regeneration(unittest.TestCase):
    def test_check_is_byte_identical(self):
        result = subprocess.run(
            [sys.executable, "-I", os.path.join(ROOT, "generator", "generate.py"),
             "--px4", os.environ["FORK_PREFLIGHT_PX4"], "--param-lineage", os.environ["FORK_PREFLIGHT_PARAM_LINEAGE"], "--check"],
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
