// SPDX-License-Identifier: BSD-3-Clause
//! The git queries the scanner needs, run with the `git` command.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
            .args(["-c", "core.quotePath=false"])
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
        self.is_ancestor(commit, "HEAD")
    }

    /// Whether `commit` is `of` or an ancestor of it.
    pub fn is_ancestor(&self, commit: &str, of: &str) -> bool {
        self.run(&["merge-base", "--is-ancestor", commit, of])
            .is_ok()
    }

    /// The full commit id of a revision, or an error when the repository does not have it.
    pub fn resolve_commit(&self, rev: &str) -> Result<String, String> {
        if rev.starts_with('-') {
            return Err(format!("not a revision: {rev}"));
        }
        self.run(&[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ])
        .map(|out| out.trim().to_string())
        .map_err(|_| format!("the repository has no commit {rev}"))
    }

    /// The best common ancestor of HEAD and `commit`, if they have one.
    pub fn merge_base_with_head(&self, commit: &str) -> Option<String> {
        self.run(&["merge-base", "HEAD", commit])
            .ok()
            .map(|out| out.trim().to_string())
            .filter(|out| !out.is_empty())
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

    /// Line numbers (1-based, in HEAD) of the lines that HEAD added or changed since `base`, per file.
    pub fn added_lines(&self, base: &str) -> Result<BTreeMap<String, Vec<usize>>, String> {
        let out = self.run(&[
            "diff",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "-U0",
            "--no-color",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            base,
            "HEAD",
        ])?;
        added_lines_by_file(&out)
    }

    /// The contents of several files in HEAD, read with one `git cat-file --batch`.
    pub fn show_head_many(&self, paths: &[&str]) -> Result<Vec<String>, String> {
        self.show_many("HEAD", paths)
    }

    /// Every blob (no submodule or symlink entry) in the tree of a commit.
    pub fn blob_files(&self, commit: &str) -> Result<Vec<String>, String> {
        let out = self.run(&["ls-tree", "-r", "-z", commit])?;
        let mut files: Vec<String> = out
            .split('\0')
            .filter_map(|entry| {
                let (meta, path) = entry.split_once('\t')?;
                let mut meta = meta.split(' ');
                let mode = meta.next()?;
                (meta.next()? == "blob" && mode != "120000").then(|| path.to_string())
            })
            .collect();
        files.sort();
        Ok(files)
    }

    /// The contents of several files in the tree of `rev`, read with one `git cat-file --batch`.
    pub fn show_many(&self, rev: &str, paths: &[&str]) -> Result<Vec<String>, String> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run git: {e}"))?;
        let mut request = String::new();
        for path in paths {
            if path.contains('\n') {
                return Err(format!("file name with a newline: {path:?}"));
            }
            request.push_str(&format!("{rev}:{path}\n"));
        }
        let mut stdin = child.stdin.take().expect("piped stdin");
        let writer = std::thread::spawn(move || stdin.write_all(request.as_bytes()));
        let mut out = Vec::new();
        child
            .stdout
            .take()
            .expect("piped stdout")
            .read_to_end(&mut out)
            .map_err(|e| format!("git cat-file: {e}"))?;
        writer
            .join()
            .map_err(|_| "git cat-file: writer failed".to_string())?
            .map_err(|e| format!("git cat-file: {e}"))?;
        let status = child.wait().map_err(|e| format!("git cat-file: {e}"))?;
        if !status.success() {
            return Err("git cat-file --batch failed".into());
        }
        let mut contents = Vec::with_capacity(paths.len());
        let mut rest = out.as_slice();
        for path in paths {
            let end = rest
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(|| "git cat-file: truncated output".to_string())?;
            let header = String::from_utf8_lossy(&rest[..end]).to_string();
            rest = &rest[end + 1..];
            let size: usize = match header.rsplit(' ').next().map(str::parse) {
                Some(Ok(size)) if header.split(' ').nth(1) == Some("blob") => size,
                _ => return Err(format!("git cat-file {rev}:{path}: {header}")),
            };
            if rest.len() < size + 1 {
                return Err("git cat-file: truncated output".into());
            }
            contents.push(String::from_utf8_lossy(&rest[..size]).into_owned());
            rest = &rest[size + 1..];
        }
        Ok(contents)
    }
}

/// The new-side line numbers of `+` lines in a unified diff with zero context, per file
/// (the `b/` path of each `diff --git` header).
pub fn added_lines_by_file(diff: &str) -> Result<BTreeMap<String, Vec<usize>>, String> {
    let mut files = BTreeMap::new();
    let mut chunks: Vec<(String, String)> = Vec::new();
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            chunks.push((String::new(), String::new()));
        }
        let Some((path, text)) = chunks.last_mut() else {
            continue;
        };
        if let Some(name) = line.strip_prefix("+++ b/") {
            if path.is_empty() {
                *path = name.trim_end_matches('\n').to_string();
            }
        }
        text.push_str(line);
    }
    for (path, text) in chunks {
        if !path.is_empty() {
            files.insert(path, added_lines_from_diff(&text)?);
        }
    }
    Ok(files)
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
        let two =
            format!("{diff}diff --git a/g b/g\n--- a/g\n+++ b/g\n@@ -1 +1,2 @@\n-x\n+y\n+z\n");
        let by_file = added_lines_by_file(&two).unwrap();
        assert_eq!(by_file["f"], vec![3, 11, 12]);
        assert_eq!(by_file["g"], vec![1, 2]);
    }
}
