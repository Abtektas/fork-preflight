// SPDX-License-Identifier: BSD-3-Clause
//! The git queries the scanner needs, run with the `git` command.

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Repo {
    path: PathBuf,
}

/// How a file changed between the base and HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
}

/// A file the fork added or modified since the base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub change: Change,
}

impl Repo {
    pub fn open(path: &Path) -> Result<Repo, String> {
        let repo = Repo {
            path: path.to_path_buf(),
        };
        let top = repo.run(&["rev-parse", "--show-toplevel"])?;
        Ok(Repo {
            path: PathBuf::from(top.trim()),
        })
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(args)
            .output()
            .map_err(|e| format!("cannot run git: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        String::from_utf8(out.stdout)
            .map_err(|_| format!("git {}: output is not UTF-8", args.join(" ")))
    }

    pub fn has_commit(&self, commit: &str) -> bool {
        self.run(&["cat-file", "-e", &format!("{commit}^{{commit}}")])
            .is_ok()
    }

    pub fn head(&self) -> Result<String, String> {
        Ok(self.run(&["rev-parse", "HEAD"])?.trim().to_string())
    }

    /// Whether `commit` is HEAD or an ancestor of it.
    pub fn is_ancestor_of_head(&self, commit: &str) -> bool {
        self.run(&["merge-base", "--is-ancestor", commit, "HEAD"])
            .is_ok()
    }

    /// Files added or modified between `base` and HEAD (renames count as added).
    pub fn changed_files(&self, base: &str) -> Result<Vec<ChangedFile>, String> {
        let out = self.run(&["diff", "--no-renames", "--name-status", "-z", base, "HEAD"])?;
        let mut fields = out.split('\0').filter(|f| !f.is_empty());
        let mut files = Vec::new();
        while let Some(status) = fields.next() {
            let path = fields
                .next()
                .ok_or_else(|| "git diff --name-status: truncated output".to_string())?;
            let change = match status {
                "A" => Change::Added,
                "M" | "T" => Change::Modified,
                _ => continue,
            };
            files.push(ChangedFile {
                path: path.to_string(),
                change,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    /// Every file in HEAD.
    pub fn head_files(&self) -> Result<Vec<String>, String> {
        let out = self.run(&["ls-tree", "-r", "-z", "--name-only", "HEAD"])?;
        let mut files: Vec<String> = out
            .split('\0')
            .filter(|f| !f.is_empty())
            .map(String::from)
            .collect();
        files.sort();
        Ok(files)
    }

    /// The content of a file in HEAD.
    pub fn show_head(&self, path: &str) -> Result<String, String> {
        self.run(&["show", &format!("HEAD:{path}")])
    }

    /// Line numbers (1-based, in HEAD) of the lines that HEAD added or changed in `path` since `base`.
    pub fn added_lines(&self, base: &str, path: &str) -> Result<Vec<usize>, String> {
        let out = self.run(&[
            "diff",
            "--no-renames",
            "-U0",
            "--no-color",
            base,
            "HEAD",
            "--",
            path,
        ])?;
        added_lines_from_diff(&out)
    }
}

/// Parse the new-side line numbers of `+` lines in a unified diff with zero context.
pub fn added_lines_from_diff(diff: &str) -> Result<Vec<usize>, String> {
    let mut lines = Vec::new();
    let mut next: Option<usize> = None;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            // File headers (---/+++) follow; they are not content lines.
            next = None;
        } else if let Some(rest) = line.strip_prefix("@@ ") {
            let plus = rest
                .split_whitespace()
                .find(|part| part.starts_with('+'))
                .ok_or_else(|| format!("bad hunk header: {line}"))?;
            let start = plus[1..].split(',').next().unwrap_or("");
            next = Some(
                start
                    .parse()
                    .map_err(|_| format!("bad hunk header: {line}"))?,
            );
        } else if let Some(n) = next.as_mut() {
            if line.starts_with('+') {
                lines.push(*n);
                *n += 1;
            }
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_parsing() {
        let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -3 +3 @@\n-old\n+new\n@@ -10,0 +11,2 @@\n+a\n+++b\n@@ -20,2 +22,0 @@\n-gone\n-gone\n";
        assert_eq!(added_lines_from_diff(diff).unwrap(), vec![3, 11, 12]);
    }
}
