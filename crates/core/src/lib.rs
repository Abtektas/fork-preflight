// SPDX-License-Identifier: BSD-3-Clause
//! fork-preflight: find lines in a PX4 fork that name what a newer PX4 release no longer defines.

pub mod check;
pub mod data;
pub mod git;
pub mod legal;
pub mod parse;
