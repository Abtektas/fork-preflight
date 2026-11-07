// SPDX-License-Identifier: BSD-3-Clause
//! Parsers for the fork files the rules read.

/// One symbol assignment in a `*.px4board` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub symbol: String,
    /// 1-based line number.
    pub line: usize,
    pub text: String,
}

/// The assignments of a `*.px4board` file, recognized the way kconfiglib's
/// `load_config` does: `CONFIG_<name>=<value>` or `# CONFIG_<name> is not set`
/// at the start of the line, trailing whitespace removed.
pub fn board_assignments(content: &str) -> Vec<Assignment> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, raw)| {
            let line = raw.trim_end();
            assignment_symbol(line).map(|symbol| Assignment {
                symbol: symbol.to_string(),
                line: index + 1,
                text: line.to_string(),
            })
        })
        .collect()
}

/// The symbol a single config line assigns, if any.
pub fn assignment_symbol(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("CONFIG_") {
        let (name, _) = rest.split_once('=')?;
        return (!name.is_empty()).then_some(name);
    }
    let rest = line.strip_prefix("# CONFIG_")?;
    let (name, tail) = rest.split_once(' ')?;
    (!name.is_empty() && tail.starts_with("is not set")).then_some(name)
}

/// Symbols a Kconfig file defines (`config NAME` or `menuconfig NAME`).
pub fn kconfig_definitions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            match (words.next(), words.next(), words.next()) {
                (Some("config" | "menuconfig"), Some(name), None) => Some(name.to_string()),
                _ => None,
            }
        })
        .collect()
}

/// `vendor/model` of a board file path `boards/<vendor>/<model>/<label>.px4board`.
pub fn board_of(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["boards", vendor, model, file] if file.ends_with(".px4board") => {
            Some(format!("{vendor}/{model}"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignments_like_kconfiglib() {
        let text = "CONFIG_A=y\n# CONFIG_B is not set\n  CONFIG_INDENTED=y\n#CONFIG_C is not set\n# CONFIG_D\nCONFIG_E=\"x=y\"  \nCONFIG_=y\nCONFIG_F\n# comment\n";
        let got: Vec<(String, usize)> = board_assignments(text)
            .into_iter()
            .map(|a| (a.symbol, a.line))
            .collect();
        assert_eq!(got, vec![("A".into(), 1), ("B".into(), 2), ("E".into(), 6)]);
        assert_eq!(
            board_assignments("CONFIG_E=\"v\"  \n")[0].text,
            "CONFIG_E=\"v\""
        );
    }

    #[test]
    fn kconfig_definitions_only_named_entries() {
        let text = "menuconfig MODULES_X\n\tbool \"x\"\nif MODULES_X\nconfig MODULES_X_OPT\n\tint \"o\"\n  config INDENTED\nendif\n# config COMMENTED\nconfig\nchoice\n";
        assert_eq!(
            kconfig_definitions(text),
            vec!["MODULES_X", "MODULES_X_OPT", "INDENTED"]
        );
    }

    #[test]
    fn board_paths() {
        assert_eq!(
            board_of("boards/px4/fmu-v6x/default.px4board").as_deref(),
            Some("px4/fmu-v6x")
        );
        assert_eq!(board_of("boards/px4/fmu-v6x/src/x.px4board"), None);
        assert_eq!(board_of("other/a/b/c.px4board"), None);
    }
}
