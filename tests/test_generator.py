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
import kconfig_removed  # noqa: E402


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

    def test_builtins_of_apps_cpp_in(self):
        tree = self.tree({
            "src/systemcmds/shutdown/CMakeLists.txt": "px4_add_module(MODULE systemcmds__shutdown MAIN shutdown SRCS s.cpp)\n",
            "platforms/common/apps.cpp.in": 'static void init(apps_map_type &apps)\n{\n\t${builtin_apps_string}\n'
                                            '\tapps["shutdown"] = shutdown_main;\n\tapps["sleep"] = sleep_main;\n'
                                            '\t// apps["not_this"] = x;\n}\n',
        })
        self.assertEqual(generate.commands_data(tree), {
            "shutdown": ["platforms/common/apps.cpp.in", "src/systemcmds/shutdown/CMakeLists.txt"],
            "sleep": ["platforms/common/apps.cpp.in"],
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


class RemovalRules(unittest.TestCase):
    def test_token_insert_rename(self):
        self.assertTrue(kconfig_removed.insert_rename("DRIVERS_IMU_L3GD20", "DRIVERS_IMU_ST_L3GD20"))
        self.assertFalse(kconfig_removed.insert_rename("DRIVERS_IMU_L3GD20", "DRIVERS_IMU_L3GD20_X"))

    def test_batch_prefix_rename_needs_three_pairs(self):
        rem = {"UAVCAN_V1_A", "UAVCAN_V1_B", "UAVCAN_V1_C", "OTHER"}
        add = {"CYPHAL_A", "CYPHAL_B", "CYPHAL_C", "NEW"}
        self.assertEqual(kconfig_removed.prefix_groups(rem, add),
                         {"UAVCAN_V1_A": "CYPHAL_A", "UAVCAN_V1_B": "CYPHAL_B", "UAVCAN_V1_C": "CYPHAL_C"})
        self.assertEqual(kconfig_removed.prefix_groups({"UAVCAN_V1_A", "UAVCAN_V1_B"}, {"CYPHAL_A", "CYPHAL_B"}), {})

    def test_zenoh_topic_name(self):
        self.assertEqual(kconfig_removed.topic_of("msg/RoverSpeedStatus.msg"), "ZENOH_PUBSUB_ROVER_SPEED_STATUS")


class Ordering(unittest.TestCase):
    def test_version_key(self):
        names = ["main", "v1.18.0", "v1.18.0-rc1", "v1.18.0-beta2", "v1.17.0", "v1.9.0", "v1.10.0"]
        self.assertEqual(sorted(names, key=generate.version_key),
                         ["v1.9.0", "v1.10.0", "v1.17.0", "v1.18.0-beta2", "v1.18.0-rc1", "v1.18.0", "main"])
        with self.assertRaises(generate.GenerateError):
            generate.version_key("release/1.18")

    def test_check_notes_only_a_git_version_difference(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = os.path.join(tmp.name, "r.json")
        data = {"tools": {"git": "2.56.0"}, "kconfig": {"symbols": ["A"]}}
        write(path, generate.dumps({**data, "tools": {"git": "2.43.0"}}))
        self.assertEqual(generate.compare(path, data, generate.dumps(data)),
                         (True, " (data made with git 2.43.0, this is git 2.56.0)"))
        changed = {**data, "kconfig": {"symbols": ["B"]}}
        self.assertEqual(generate.compare(path, changed, generate.dumps(changed)), (False, ""))


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

    def test_posix_builtins_in_every_release(self):
        # Scripts run them though no px4_add_module() declares them (DESIGN.md, P3 note).
        for release, d in self.data.items():
            for name in ["list_files", "list_tasks", "shutdown", "sleep"]:
                self.assertIn("platforms/common/apps.cpp.in", d["commands"].get(name, []), (release, name))

    def test_parameters(self):
        final = self.data["v1.18.0"]["parameters"]
        self.assertEqual(len(final["removed"]), 61)
        self.assertEqual(final["removed"]["SENS_TEMP_ID"], {"kind": "migrated", "successors": ["HEATER1_SENS_ID"]})
        self.assertEqual(set(self.data["v1.17.0"]["parameters"]["names"]) - set(final["names"]), set(final["removed"]))
        # param-lineage replaces a release candidate's snapshot with the final release's
        for release in ["v1.18.0-rc1", "main"]:
            self.assertIsNone(self.data[release]["parameters"])
            self.assertTrue(any(f"no snapshot of {release}" in n for n in self.data[release]["notes"]))

    def removal(self, release, symbol):
        return self.data[release]["kconfig"]["removed"]["symbols"][symbol]

    def test_kconfig_removed_matches_the_symbol_sets(self):
        releases = [e["release"] for e in self.manifest["releases"]]
        self.assertIsNone(self.data[releases[0]]["kconfig"]["removed"])
        for previous, release in zip(releases, releases[1:]):
            removed = self.data[release]["kconfig"]["removed"]
            self.assertEqual(removed["previous"], previous)
            self.assertEqual(set(removed["symbols"]), self.symbols(previous) - self.symbols(release), release)
            for symbol, entry in removed["symbols"].items():
                self.assertEqual(entry["successors"], sorted(set(entry["successors"])), symbol)
                self.assertTrue(set(entry["successors"]) <= self.symbols(release), symbol)
                self.assertEqual(entry["evidence"] is None, not entry["successors"], symbol)

    # Known answers read from each commit by hand (DESIGN.md "Kconfig rename hints").

    def test_kconfig_renamed(self):
        e = self.removal("main", "NUM_MISSION_ITMES_SUPPORTED")
        self.assertEqual((e["evidence"], e["successors"]), ("renamed", ["NUM_MISSION_ITEMS_SUPPORTED"]))
        self.assertEqual(e["removed_by"][0]["commit"][:10], "1476321315")
        e = self.removal("v1.18.0-rc1", "ZENOH_PUBSUB_ROVER_VELOCITY_STATUS")
        self.assertEqual(e["successors"], ["ZENOH_PUBSUB_ROVER_SPEED_STATUS"])

    def test_kconfig_late_rules(self):
        # K5: sibling RoverVelocityStatus -> RoverSpeedStatus, same commit adds RoverSpeedSetpoint.
        e = self.removal("v1.18.0-rc1", "ZENOH_PUBSUB_ROVER_VELOCITY_SETPOINT")
        self.assertEqual((e["evidence"], e["successors"]), ("renamed", ["ZENOH_PUBSUB_ROVER_SPEED_SETPOINT"]))
        # K7: Kconfig orphaned by 21daa0f398, which added the ICP101XX family driver.
        for symbol in ("DRIVERS_BAROMETER_INVENSENSE_ICP10100", "DRIVERS_BAROMETER_INVENSENSE_ICP10111"):
            e = self.removal("v1.15.0", symbol)
            self.assertEqual((e["evidence"], e["successors"]), ("renamed", ["DRIVERS_BAROMETER_INVENSENSE_ICP101XX"]))
        # K6: the commit message names the successor.
        for release, symbol, successor in (("main", "ZENOH_PUBSUB_SENSOR_GPS", "ZENOH_PUBSUB_SENSOR_GNSS"),
                                           ("v1.16.0", "BOARD_ROOTFSDIR", "BOARD_ROOT_PATH"),
                                           ("v1.16.0", "DRIVERS_MAGNETOMETER_LIS2MDL", "DRIVERS_MAGNETOMETER_ST_IIS2MDC")):
            e = self.removal(release, symbol)
            self.assertEqual((e["evidence"], e["successors"]), ("named", [successor]))

    def test_name_rules(self):
        self.assertTrue(kconfig_removed.extends_last("D_P_MS4525", "D_P_MS4525DO"))
        self.assertFalse(kconfig_removed.extends_last("D_OSD", "D_OSD_MSP_OSD"))
        self.assertTrue(kconfig_removed.family_match("B_ICP10100", "B_ICP101XX"))
        self.assertFalse(kconfig_removed.family_match("B_ICP10100", "B_ICP201XX"))
        self.assertEqual(kconfig_removed.token_change("ROVER_VELOCITY_STATUS", "ROVER_SPEED_STATUS"), ("VELOCITY", "SPEED"))
        self.assertEqual(kconfig_removed.substitute("DRIVERS_UAVCAN_V1", "UAVCAN_V1", "CYPHAL"), "DRIVERS_CYPHAL")
        self.assertIsNone(kconfig_removed.substitute("DRIVERS_UAVCAN_V10", "UAVCAN_V1", "CYPHAL"))

    def test_kconfig_boards(self):
        e = self.removal("v1.17.0", "MODULES_FW_POS_CONTROL")
        self.assertEqual(e["evidence"], "boards")
        self.assertEqual(e["boards"], {
            "MODULES_FW_LATERAL_LONGITUDINAL_CONTROL": {"boards": 61, "of": 64, "how": "net"},
            "MODULES_FW_MODE_MANAGER": {"boards": 79, "of": 79, "how": "same-commit"},
        })
        self.assertEqual(e["removed_by"][0]["subject"], "rename FWPositionController to FWModeManager")

    def test_kconfig_selected_and_none(self):
        e = self.removal("v1.18.0-rc1", "COMMON_GPIO")
        self.assertEqual((e["evidence"], e["successors"]), ("selected", ["DRIVERS_GPIO_MCP23009"]))
        e = self.removal("main", "BOARD_UAVCAN_TIMER_OVERRIDE")
        self.assertEqual((e["evidence"], e["successors"]), (None, []))
        self.assertEqual(e["removed_by"][0]["commit"][:10], "3f6fe0e40c")
        e = self.removal("v1.17.0", "MODULES_ROVER_POS_CONTROL")
        self.assertEqual((e["evidence"], e["removed_by"][0]["subject"]), (None, "rover: remove deprecated module"))

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
