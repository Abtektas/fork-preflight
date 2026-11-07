# SPDX-License-Identifier: BSD-3-Clause
"""Load the Kconfig tree of the current directory and print its symbols as JSON.

Run by generate.py in a clean environment, with the source tree as working
directory and the variables the tree's Kconfig files use (VENDOR, MODEL, ...)
set the way PX4's cmake/kconfig.cmake sets them.
"""

import json
import sys

import kconfiglib


def main() -> int:
    kconf = kconfiglib.Kconfig("Kconfig", warn_to_stderr=False)
    json.dump(
        {
            "symbols": sorted({sym.name for sym in kconf.unique_defined_syms}),
            "warnings": list(kconf.warnings),
        },
        sys.stdout,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
