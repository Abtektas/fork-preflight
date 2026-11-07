// SPDX-License-Identifier: BSD-3-Clause
//! The release data generated into data/releases/ and embedded at build time.

use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

include!(concat!(env!("OUT_DIR"), "/releases.rs"));

/// The data layout this build reads (`format` in the files).
pub const FORMAT: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct Release {
    pub format: u32,
    pub release: String,
    pub source: Source,
    pub kconfig: Kconfig,
    pub commands: BTreeMap<String, Vec<String>>,
    /// Null when param-lineage has no snapshot of the release.
    #[serde(default)]
    pub parameters: Option<Parameters>,
}

#[derive(Debug, Deserialize)]
pub struct Parameters {
    pub names: BTreeSet<String>,
    /// Parameters this release removed, with param-lineage's curated kind and successors.
    pub removed: BTreeMap<String, Removal>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Removal {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub successors: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Source {
    pub commit: String,
}

#[derive(Debug, Deserialize)]
pub struct Kconfig {
    pub symbols: BTreeSet<String>,
    pub board_symbols: BTreeMap<String, BTreeSet<String>>,
}

impl Release {
    /// Whether a symbol is defined for a board (`vendor/model`), or globally when no board is given.
    pub fn defines_symbol(&self, symbol: &str, board: Option<&str>) -> bool {
        self.kconfig.symbols.contains(symbol)
            || board
                .and_then(|board| self.kconfig.board_symbols.get(board))
                .is_some_and(|symbols| symbols.contains(symbol))
    }
}

/// All embedded releases, keyed by release name.
pub fn releases() -> Result<BTreeMap<String, Release>, String> {
    let mut map = BTreeMap::new();
    for (name, text) in RELEASE_FILES {
        let release: Release =
            serde_json::from_str(text).map_err(|e| format!("embedded data {name}.json: {e}"))?;
        if release.format != FORMAT {
            return Err(format!(
                "embedded data {name}.json has format {}, this build reads {FORMAT}",
                release.format
            ));
        }
        if release.release != *name {
            return Err(format!(
                "embedded data {name}.json names release {}",
                release.release
            ));
        }
        map.insert(release.release.clone(), release);
    }
    Ok(map)
}

/// A release name as an orderable version. `main` is newer than every tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Version {
    Tag {
        numbers: [u32; 3],
        /// (kind rank, number); a final release has none and sorts last.
        pre: Option<(u8, u32)>,
    },
    Main,
}

impl Version {
    pub fn parse(name: &str) -> Option<Version> {
        if name == "main" {
            return Some(Version::Main);
        }
        let rest = name.strip_prefix('v')?;
        let (numbers, pre) = match rest.split_once('-') {
            Some((numbers, pre)) => (numbers, Some(pre)),
            None => (rest, None),
        };
        let parts: Vec<u32> = numbers
            .split('.')
            .map(|p| p.parse().ok())
            .collect::<Option<_>>()?;
        let numbers: [u32; 3] = parts.try_into().ok()?;
        let pre = match pre {
            None => None,
            Some(pre) => {
                let (rank, digits) = [("alpha", 0), ("beta", 1), ("rc", 2)]
                    .iter()
                    .find_map(|(kind, rank)| pre.strip_prefix(kind).map(|d| (*rank, d)))?;
                Some((rank, digits.parse().ok()?))
            }
        };
        Some(Version::Tag { numbers, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Version::Main, Version::Main) => Ordering::Equal,
            (Version::Main, _) => Ordering::Greater,
            (_, Version::Main) => Ordering::Less,
            (
                Version::Tag {
                    numbers: a,
                    pre: pa,
                },
                Version::Tag {
                    numbers: b,
                    pre: pb,
                },
            ) => a.cmp(b).then_with(|| match (pa, pb) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(x), Some(y)) => x.cmp(y),
            }),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_data_loads() {
        let releases = releases().unwrap();
        assert!(releases.contains_key("v1.17.0") && releases.contains_key("main"));
        let rc1 = &releases["v1.18.0-rc1"];
        assert!(rc1.defines_symbol("BOARD_UAVCAN_TIMER_OVERRIDE", None));
        let main = &releases["main"];
        assert!(!main.defines_symbol("BOARD_NAVQ95_IO23_PWM", None));
        assert!(main.defines_symbol("BOARD_NAVQ95_IO23_PWM", Some("nxp/mr-navq95")));
    }

    #[test]
    fn version_order() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("v1.18.0-alpha1") < v("v1.18.0-beta1"));
        assert!(v("v1.18.0-beta2") < v("v1.18.0-rc1"));
        assert!(v("v1.18.0-rc1") < v("v1.18.0"));
        assert!(v("v1.17.0") < v("v1.18.0-alpha1"));
        assert!(v("v1.9.0") < v("v1.10.0"));
        assert!(v("v1.18.0") < v("main"));
        assert_eq!(Version::parse("v1.18"), None);
        assert_eq!(Version::parse("v1.18.0-preview1"), None);
        assert_eq!(Version::parse("release/1.18"), None);
    }
}
