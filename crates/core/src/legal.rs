// SPDX-License-Identifier: BSD-3-Clause
//! The license and the notices, so that every copy of the program carries them.

pub const LICENSE: &str = include_str!("../../../LICENSE");
pub const LICENSE_THIRD_PARTY: &str = include_str!("../../../LICENSE-THIRD-PARTY");

pub const NO_WARRANTY: &str = "This software and its data are provided \"as is\", without warranty of any kind. \
The authors and contributors accept no liability and no responsibility for any use of them or of their results.";

pub const NOT_A_SAFETY_TOOL: &str =
    "A report without findings does not mean that an upgrade is complete or safe; \
the tool only checks what its data covers.";

pub const NOT_AFFILIATED: &str =
    "This project is not affiliated with or endorsed by the PX4 project or the \
Dronecode Foundation. PX4 is a trademark of Dronecode Project, Inc.";

/// The short notice every report and `--version` end with.
pub fn notice() -> String {
    format!("{NO_WARRANTY} {NOT_A_SAFETY_TOOL} {NOT_AFFILIATED} See DISCLAIMER.md and `fork-preflight --license`.")
}
