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

/// Whether a path is a startup or airframe script the scanner reads: files in
/// `ROMFS/<dir>/init.d*/`, `boards/<vendor>/<model>/init/` and `target/`,
/// `platforms/nuttx/init/` and `posix-configs/`, except build files, templates
/// and documents.
pub fn is_script(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let in_script_dir = match parts.as_slice() {
        ["ROMFS", _, dir, _, ..] => dir.starts_with("init.d"),
        ["boards", _, _, "init" | "target", _, ..] => true,
        ["platforms", "nuttx", "init", _, ..] => true,
        ["posix-configs", _, ..] => true,
        _ => false,
    };
    let name = parts.last().copied().unwrap_or("");
    let excluded = name == "CMakeLists.txt"
        || name.starts_with('.')
        || name.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty()
                && matches!(
                    ext,
                    "txt"
                        | "in"
                        | "md"
                        | "params"
                        | "jinja"
                        | "json"
                        | "py"
                        | "cmake"
                        | "yaml"
                        | "sh"
                        | "bin"
                )
        });
    in_script_dir && !excluded
}

/// One simple command in a script line: the command word and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptCommand {
    /// 1-based line number.
    pub line: usize,
    pub command: String,
    pub args: Vec<String>,
    /// The line, trailing whitespace removed.
    pub text: String,
}

/// Words that start a compound command or a branch; the command follows them.
const LEADING_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "while", "until", "do", "!", "{", "time",
];
/// Segments starting with these hold no command the scanner checks.
const SKIPPED_SEGMENTS: &[&str] = &["fi", "done", "esac", "}", "for", "case", "in"];

/// The simple commands of a startup script (NuttShell or POSIX sh). Comments,
/// here-documents and quoted text are skipped; `;`, `&&`, `||`, `|` and `&`
/// separate commands; leading `if`/`then`/`!`... and variable assignments are
/// skipped. Only plain words (`[A-Za-z0-9_]`) count as command names.
pub fn script_commands(content: &str) -> Vec<ScriptCommand> {
    let mut commands = Vec::new();
    let mut heredoc: Option<String> = None;
    for (index, raw) in content.lines().enumerate() {
        let line = raw.trim_end();
        if let Some(end) = &heredoc {
            if line.trim_start_matches('\t') == end {
                heredoc = None;
            }
            continue;
        }
        let (segments, opened) = split_line(line);
        heredoc = opened;
        for words in segments {
            let mut words = words.into_iter().peekable();
            while words.peek().is_some_and(|w| {
                is_assignment(&w.text)
                    || (!w.quoted
                        && (LEADING_KEYWORDS.contains(&w.text.as_str())
                            || is_case_pattern(&w.text)))
            }) {
                words.next();
            }
            let Some(first) = words.next() else { continue };
            if first.quoted
                || SKIPPED_SEGMENTS.contains(&first.text.as_str())
                || !is_plain_word(&first.text)
            {
                continue;
            }
            commands.push(ScriptCommand {
                line: index + 1,
                command: first.text,
                args: words.map(|w| w.text).collect(),
                text: line.to_string(),
            });
        }
    }
    commands
}

struct Word {
    text: String,
    quoted: bool,
}

/// Split one line into command segments of words, and the delimiter of a here-document it opens.
fn split_line(line: &str) -> (Vec<Vec<Word>>, Option<String>) {
    let mut segments = vec![Vec::new()];
    let mut heredoc = None;
    let mut chars = line.chars().peekable();
    let mut current: Option<Word> = None;
    let finish = |current: &mut Option<Word>, segments: &mut Vec<Vec<Word>>| {
        if let Some(word) = current.take() {
            segments.last_mut().unwrap().push(word);
        }
    };
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => finish(&mut current, &mut segments),
            '#' if current.is_none() => break,
            '"' | '\'' => {
                let word = current.get_or_insert(Word {
                    text: String::new(),
                    quoted: false,
                });
                word.quoted = true;
                for q in chars.by_ref() {
                    if q == c {
                        break;
                    }
                    word.text.push(q);
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    current
                        .get_or_insert(Word {
                            text: String::new(),
                            quoted: false,
                        })
                        .text
                        .push(next);
                }
            }
            ';' | '&' | '|' => {
                finish(&mut current, &mut segments);
                while chars.peek().is_some_and(|n| matches!(n, ';' | '&' | '|')) {
                    chars.next();
                }
                segments.push(Vec::new());
            }
            '<' if chars.peek() == Some(&'<') => {
                finish(&mut current, &mut segments);
                chars.next();
                let rest: String = chars.by_ref().collect();
                let word = rest
                    .trim_start_matches('-')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                let delimiter = word.trim_matches(|q| q == '"' || q == '\'');
                if !delimiter.is_empty() {
                    heredoc = Some(delimiter.to_string());
                }
                break;
            }
            _ => current
                .get_or_insert(Word {
                    text: String::new(),
                    quoted: false,
                })
                .text
                .push(c),
        }
    }
    finish(&mut current, &mut segments);
    (segments, heredoc)
}

fn is_plain_word(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        is_plain_word(name) && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// A `case` branch pattern such as `4001)` or `*)`.
fn is_case_pattern(word: &str) -> bool {
    word.len() > 1 && word.ends_with(')') && !word.contains('(')
}

/// A parameter a script names, from a `param` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamUse {
    pub name: String,
    pub subcommand: String,
    /// The command has `-s`, which silences PX4's error for an unknown name.
    pub silent: bool,
}

/// The parameters a `param` command names: the name of `set`, `set-default`,
/// `compare`, `greater`, `bitset`, `bitclear` and `find`, every name of `touch`
/// and `reset` (not those with a wildcard). Names with `$` or other non-name
/// characters are skipped.
pub fn param_uses(command: &ScriptCommand) -> Vec<ParamUse> {
    if command.command != "param" {
        return Vec::new();
    }
    let Some((sub, rest)) = command.args.split_first() else {
        return Vec::new();
    };
    let (silent, rest) = match rest.split_first() {
        Some((flag, rest))
            if flag == "-s" && matches!(sub.as_str(), "set-default" | "compare" | "greater") =>
        {
            (true, rest)
        }
        _ => (false, rest),
    };
    let names: &[String] = match sub.as_str() {
        "set" | "set-default" | "compare" | "greater" | "bitset" | "bitclear" | "find" => {
            &rest[..rest.len().min(1)]
        }
        "touch" | "reset" => rest,
        _ => &[],
    };
    names
        .iter()
        .filter(|name| is_param_name(name))
        .map(|name| ParamUse {
            name: name.clone(),
            subcommand: sub.clone(),
            silent,
        })
        .collect()
}

/// PX4's parameter name form: upper case, digits and `_`, at most 16 characters.
pub fn is_param_name(name: &str) -> bool {
    name.len() <= 16
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Parameters a fork's own source defines, so that scripts naming them are not reported.
#[derive(Debug, Default)]
pub struct ParamDefinitions {
    names: std::collections::BTreeSet<String>,
    /// Names with `${i}` in a module config, as (prefix, suffix) around the instance number.
    instanced: Vec<(String, String)>,
    /// `param_prefix` of a module config's actuator output, which generates `<prefix>_*`.
    prefixes: Vec<String>,
}

impl ParamDefinitions {
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
            || self.instanced.iter().any(|(prefix, suffix)| {
                name.strip_prefix(prefix.as_str())
                    .and_then(|rest| rest.strip_suffix(suffix.as_str()))
                    .is_some_and(|digits| {
                        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
                    })
            })
            || self.prefixes.iter().any(|prefix| {
                name.strip_prefix(prefix.as_str())
                    .is_some_and(|rest| rest.starts_with('_'))
            })
    }

    /// Read the definitions in one file, limited to the given lines when there are any:
    /// `PARAM_DEFINE_INT32/FLOAT(NAME` in C and C++ files, and the names under
    /// `definitions:` and `param_prefix:` in YAML module configs.
    pub fn add_file(&mut self, path: &str, content: &str, lines: Option<&[usize]>) {
        let name = path.rsplit('/').next().unwrap_or(path);
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        let wanted = |index: usize| lines.is_none_or(|lines| lines.contains(&(index + 1)));
        match ext {
            "c" | "cpp" | "h" | "hpp" => {
                for (_, line) in content.lines().enumerate().filter(|(i, _)| wanted(*i)) {
                    for macro_name in ["PARAM_DEFINE_INT32(", "PARAM_DEFINE_FLOAT("] {
                        for (pos, _) in line.match_indices(macro_name) {
                            let rest = &line[pos + macro_name.len()..];
                            let param: String = rest
                                .trim_start()
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                                .collect();
                            if !param.is_empty() {
                                self.names.insert(param);
                            }
                        }
                    }
                }
            }
            "yaml" | "yml" => {
                for (_, line) in content.lines().enumerate().filter(|(i, _)| wanted(*i)) {
                    let trimmed = line.trim();
                    if let Some(value) = trimmed
                        .strip_prefix("- ")
                        .unwrap_or(trimmed)
                        .strip_prefix("param_prefix:")
                    {
                        let value = value.trim().trim_matches(|q| q == '"' || q == '\'');
                        if !value.is_empty() && !value.contains('$') {
                            self.prefixes.push(value.to_string());
                        }
                        continue;
                    }
                    if !line.starts_with([' ', '\t']) {
                        continue;
                    }
                    let Some(key) = trimmed.strip_suffix(':') else {
                        continue;
                    };
                    if let Some((prefix, suffix)) = key.split_once("${i}") {
                        if (prefix.is_empty() || is_param_name(prefix)) && !suffix.contains('$') {
                            self.instanced
                                .push((prefix.to_string(), suffix.to_string()));
                        }
                    } else if is_param_name(key) {
                        self.names.insert(key.to_string());
                    }
                }
            }
            _ => {}
        }
    }
}

/// Command names a `CMakeLists.txt` declares with `MAIN <name>` (in `px4_add_module`).
pub fn cmake_main_commands(content: &str, lines: Option<&[usize]>) -> Vec<String> {
    let mut commands = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if lines.is_some_and(|lines| !lines.contains(&(index + 1))) {
            continue;
        }
        let code = line.split('#').next().unwrap_or("");
        let mut words = code.split_whitespace();
        while let Some(word) = words.next() {
            if word == "MAIN" {
                if let Some(name) = words.next() {
                    let name = name.trim_end_matches(')');
                    if is_plain_word(name) {
                        commands.push(name.to_string());
                    }
                }
            }
        }
    }
    commands
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

    #[test]
    fn script_paths() {
        for path in [
            "ROMFS/px4fmu_common/init.d/rcS",
            "ROMFS/px4fmu_common/init.d/rc.vtol_defaults",
            "ROMFS/px4fmu_common/init.d/airframes/4001_quad_x",
            "ROMFS/px4fmu_common/init.d-posix/airframes/10016_none_iris",
            "ROMFS/px4fmu_common/init.d-posix/px4-rc.simulator",
            "ROMFS/cannode/init.d/rcS",
            "boards/px4/fmu-v6x/init/rc.board_defaults",
            "boards/modalai/voxl2/target/voxl-px4-start",
            "boards/modalai/voxl2/target/voxl-px4-hitl.config",
            "platforms/nuttx/init/stm32h7/rc.board_arch_defaults",
            "posix-configs/rpi/px4.config",
        ] {
            assert!(is_script(path), "{path}");
        }
        for path in [
            "ROMFS/px4fmu_common/init.d/CMakeLists.txt",
            "ROMFS/px4fmu_common/init.d/airframes/CMakeLists.txt",
            "ROMFS/px4fmu_common/mixers/quad_x.main.mix",
            "ROMFS/px4fmu_common/init.d/rc.board_bootloader_upgrade.in",
            "boards/px4/fmu-v6x/default.px4board",
            "boards/px4/fmu-v6x/src/init.c",
            "docs/assets/airframes/fw/x/params.params",
            "Tools/serial/rc.serial_port.jinja",
            "ROMFS/px4fmu_common/init.d/airframes/README.md",
        ] {
            assert!(!is_script(path), "{path}");
        }
    }

    fn commands_of(text: &str) -> Vec<(usize, String, Vec<String>)> {
        script_commands(text)
            .into_iter()
            .map(|c| (c.line, c.command, c.args))
            .collect()
    }

    #[test]
    fn script_command_words() {
        let text = "#!/bin/sh\n\
            # param set-default COMMENTED 1\n\
            param set-default MC_ROLL_P 6.5   # trailing comment\n\
            if param compare -s SYS_AUTOSTART 4001\n\
            then\n\
            \tcore_heater start\n\
            fi\n\
            if ! ver hwcmp PX4_FMU_V5X; then rgbled start; fi\n\
            A=1 B=\"x y\" safety_button start && echo \"param set X 1\" || tone_alarm error\n\
            echo 'core_heater start'\n\
            cat <<EOF > rules\n\
            gone_cmd start\n\
            EOF\n\
            for i in 1 2; do pwm_out start; done\n\
            case \"$x\" in\n\
            \t4001) param set-default CASE_P 1 ;;\n\
            \t*) ;;\n\
            esac\n\
            $CMD start\n\
            ./script.sh\n\
            [ -f x ] && px4-param show\n\
            param set NAME_\\\n";
        let got = commands_of(text);
        let words: Vec<(usize, &str)> = got.iter().map(|(l, c, _)| (*l, c.as_str())).collect();
        assert_eq!(
            words,
            vec![
                (3, "param"),
                (4, "param"),
                (6, "core_heater"),
                (8, "ver"),
                (8, "rgbled"),
                (9, "safety_button"),
                (9, "echo"),
                (9, "tone_alarm"),
                (10, "echo"),
                (11, "cat"),
                (14, "pwm_out"),
                (16, "param"),
                (22, "param"),
            ]
        );
        assert_eq!(got[0].2, vec!["set-default", "MC_ROLL_P", "6.5"]);
        assert_eq!(got[6].2, vec!["param set X 1"]);
    }

    #[test]
    fn param_names_of_commands() {
        let uses = |line: &str| -> Vec<(String, bool)> {
            script_commands(line)
                .iter()
                .flat_map(param_uses)
                .map(|u| (u.name, u.silent))
                .collect()
        };
        assert_eq!(
            uses("param set-default MC_ROLL_P 6.5"),
            vec![("MC_ROLL_P".into(), false)]
        );
        assert_eq!(
            uses("param set-default -s SENS_EN_X 1"),
            vec![("SENS_EN_X".into(), true)]
        );
        assert_eq!(
            uses("if param greater -s UAVCAN_ENABLE 0"),
            vec![("UAVCAN_ENABLE".into(), true)]
        );
        assert_eq!(
            uses("param set CA_ROTOR_COUNT 4 fail"),
            vec![("CA_ROTOR_COUNT".into(), false)]
        );
        assert_eq!(
            uses("param touch PWM_MAIN_FUNC1 PWM_MAIN_FUNC2"),
            vec![
                ("PWM_MAIN_FUNC1".into(), false),
                ("PWM_MAIN_FUNC2".into(), false)
            ]
        );
        assert_eq!(
            uses("param reset SYS_* CAL_ACC0_ID"),
            vec![("CAL_ACC0_ID".into(), false)]
        );
        assert!(uses("param set-default PWM_MAIN_FUNC${i} 101").is_empty());
        assert!(uses("param set-default $NAME 1").is_empty());
        assert!(uses("param show -q SYS_AUTOSTART").is_empty());
        assert!(uses("param select $PARAM_FILE").is_empty());
        assert!(uses("param reset_all SYS_AUTOSTART").is_empty());
        assert!(uses("param set-default THIS_NAME_IS_TOO_LONG 1").is_empty());
        assert!(uses("param set-default -s").is_empty());
    }

    #[test]
    fn fork_parameter_definitions() {
        let mut defs = ParamDefinitions::default();
        defs.add_file(
            "src/drivers/acme/acme_params.c",
            "PARAM_DEFINE_INT32(ACME_EN, 0);\nPARAM_DEFINE_FLOAT( ACME_GAIN, 1.0f);\n// PARAM_DEFINE_INT32(\n",
            None,
        );
        defs.add_file(
            "src/drivers/acme/module.yaml",
            "module_name: acme\nparameters:\n  - group: Acme\n    definitions:\n      ACME_RATE:\n        type: int32\n      ACME_${i}_ID:\n        num_instances: 3\n      ACME_OUT${i}:\n        type: int32\nactuator_output:\n  output_groups:\n    - param_prefix: ACME_PWM\n",
            None,
        );
        defs.add_file("src/drivers/acme/other.yaml", "  NOT_ADDED:\n", Some(&[2]));
        for name in [
            "ACME_EN",
            "ACME_GAIN",
            "ACME_RATE",
            "ACME_2_ID",
            "ACME_OUT12",
            "ACME_PWM_FUNC1",
            "ACME_PWM_REV",
        ] {
            assert!(defs.contains(name), "{name}");
        }
        for name in [
            "ACME_X_ID",
            "ACME__ID",
            "ACME_OUT",
            "ACME_PWMX",
            "NOT_ADDED",
            "module_name",
        ] {
            assert!(!defs.contains(name), "{name}");
        }
    }

    #[test]
    fn cmake_commands() {
        let text = "px4_add_module(\n\tMODULE drivers__acme\n\tMAIN acme_heater\n\tSRCS x.cpp\n)\n# MAIN commented\npx4_add_module(MODULE m MAIN short)\n";
        assert_eq!(
            cmake_main_commands(text, None),
            vec!["acme_heater", "short"]
        );
        assert_eq!(cmake_main_commands(text, Some(&[7])), vec!["short"]);
    }
}
