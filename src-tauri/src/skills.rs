//! Skills: expert how-to notes (`<dir>/<name>/SKILL.md` with `name`/`description` front matter,
//! plus reference files next to it) — the same format Claude Code uses, so a skill written for it
//! works in Loki on any engine. The system prompt carries a one-line index; the model loads a
//! skill with `read_skill(name)` before doing that kind of work, and reference files with
//! `read_skill(name, file)`. Read-only, and confined to the skill's own folder.

use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: usize = 80_000;
const INDEX_DESCRIPTION_CHARS: usize = 200;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
}

#[derive(Debug, Default)]
pub struct Skills {
    skills: Vec<Skill>,
}

/// Splits `---\nkey: value\n---\nbody`. Understands plain values and folded/literal blocks
/// (`key: >` / `key: |` followed by indented lines), which is how skill descriptions are written.
fn parse_frontmatter(text: &str) -> (Vec<(String, String)>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else { return (Vec::new(), text) };
    let Some(end) = rest.find("\n---") else { return (Vec::new(), text) };
    let (fm, after) = rest.split_at(end);
    let body = after.trim_start_matches("\n---").trim_start_matches('\n');
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut block: Option<(String, Vec<String>)> = None;
    let flush = |block: &mut Option<(String, Vec<String>)>, pairs: &mut Vec<(String, String)>| {
        if let Some((k, lines)) = block.take() {
            pairs.push((k, lines.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")));
        }
    };
    for line in fm.lines() {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented {
            if let Some((_, lines)) = block.as_mut() {
                lines.push(line.trim().to_string());
            }
            continue;
        }
        flush(&mut block, &mut pairs);
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            if v == ">" || v == "|" || v == ">-" || v == "|-" {
                block = Some((k.trim().to_string(), Vec::new()));
            } else {
                pairs.push((k.trim().to_string(), v.trim_matches('"').trim_matches('\'').to_string()));
            }
        }
    }
    flush(&mut block, &mut pairs);
    (pairs, body)
}

fn expand_home(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(p),
    }
}

impl Skills {
    /// Scans each directory for `<dir>/<skill>/SKILL.md` (symlinked skill folders are followed).
    pub fn load(dirs: &[String]) -> Self {
        let mut skills: Vec<Skill> = Vec::new();
        for d in dirs {
            let Ok(entries) = std::fs::read_dir(expand_home(d)) else { continue };
            for entry in entries.flatten() {
                let dir = entry.path();
                let Ok(text) = std::fs::read_to_string(dir.join("SKILL.md")) else { continue };
                let (fm, _) = parse_frontmatter(&text);
                let get = |k: &str| fm.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
                let name = get("name").unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
                if skills.iter().any(|s| s.name == name) {
                    continue; // earlier directories win
                }
                skills.push(Skill { name, description: get("description").unwrap_or_default(), dir });
            }
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Self { skills }
    }

    pub fn names(&self) -> Vec<String> {
        self.skills.iter().map(|s| s.name.clone()).collect()
    }

    /// The system-prompt index ("" when there are no skills).
    pub fn prompt_block(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }
        let mut out = String::from("\n\n--- Skills (expert how-to notes: call `read_skill` with the name BEFORE doing that kind of task) ---\n");
        for s in &self.skills {
            let d: String = s.description.chars().take(INDEX_DESCRIPTION_CHARS).collect();
            out.push_str(&format!("- {} — {}\n", s.name, d));
        }
        out
    }

    /// `file = None`: the skill's main text plus the list of its reference files. `file = Some(..)`:
    /// one of those files (plain text only, inside the skill's folder).
    pub fn read(&self, name: &str, file: Option<&str>) -> Result<String, String> {
        let skill = self
            .skills
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("no skill named '{name}'. Available: {}", self.names().join(", ")))?;
        let root = skill.dir.canonicalize().map_err(|e| e.to_string())?;
        match file {
            None => {
                let text = std::fs::read_to_string(root.join("SKILL.md")).map_err(|e| e.to_string())?;
                let (_, body) = parse_frontmatter(&text);
                let mut extra: Vec<String> = Vec::new();
                collect_files(&root, &root, &mut extra);
                extra.retain(|f| f != "SKILL.md");
                extra.sort();
                let mut out = body.trim().to_string();
                if !extra.is_empty() {
                    out.push_str(&format!("\n\n[Reference files in this skill — load one with read_skill(name, file): {}]", extra.join(", ")));
                }
                Ok(out)
            }
            Some(f) => {
                let f = f.trim();
                let plain = !f.is_empty() && !f.starts_with('/') && !f.split('/').any(|c| c == ".." || c.is_empty());
                let ext_ok = f.ends_with(".md") || f.ends_with(".txt");
                if !plain || !ext_ok {
                    return Err("file must be a relative .md or .txt path inside the skill".to_string());
                }
                let target = root.join(f).canonicalize().map_err(|_| format!("no file '{f}' in skill '{name}'"))?;
                if !target.starts_with(&root) {
                    return Err("that path leaves the skill's folder".to_string());
                }
                let bytes = std::fs::read(&target).map_err(|e| e.to_string())?;
                let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_FILE_BYTES)]).to_string();
                Ok(if bytes.len() > MAX_FILE_BYTES { text + "\n… [truncated]" } else { text })
            }
        }
    }
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_files(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            let r = rel.to_string_lossy().to_string();
            if r.ends_with(".md") || r.ends_with(".txt") {
                out.push(r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skills_dir() -> String {
        // One folder per call: tests run in parallel and must not delete each other's fixtures.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("loki-skills-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("omarchy")).unwrap();
        std::fs::write(
            d.join("omarchy/SKILL.md"),
            "---\nname: omarchy\ndescription: >\n  REQUIRED for desktop customization.\n  Triggers: Hyprland, themes.\n---\n# Omarchy\nMain text.\n",
        )
        .unwrap();
        std::fs::write(d.join("omarchy/hyprland.md"), "hypr notes").unwrap();
        std::fs::write(d.join("omarchy/secret.sh"), "no").unwrap();
        std::fs::create_dir_all(d.join("other")).unwrap();
        std::fs::write(d.join("other/SKILL.md"), "---\nname: other\ndescription: plain one-liner\n---\nbody").unwrap();
        std::fs::write(d.join("README.md"), "not a skill").unwrap();
        // A file outside any skill that a traversal might try to reach.
        std::fs::write(d.join("outside.md"), "TOP SECRET").unwrap();
        d.display().to_string()
    }

    #[test]
    fn parses_folded_and_plain_front_matter() {
        let (fm, body) = parse_frontmatter("---\nname: x\ndescription: >\n  line one\n  line two\nother: 'q'\n---\nBODY");
        assert_eq!(fm[0], ("name".into(), "x".into()));
        assert_eq!(fm[1], ("description".into(), "line one line two".into()));
        assert_eq!(fm[2], ("other".into(), "q".into()));
        assert_eq!(body, "BODY");
        assert_eq!(parse_frontmatter("no front matter").1, "no front matter");
    }

    #[test]
    fn indexes_skills_and_reads_them() {
        let s = Skills::load(&[skills_dir()]);
        assert_eq!(s.names(), vec!["omarchy", "other"]);
        let block = s.prompt_block();
        assert!(block.contains("- omarchy — REQUIRED for desktop customization. Triggers: Hyprland, themes."));
        let main = s.read("omarchy", None).unwrap();
        assert!(main.starts_with("# Omarchy") && main.contains("hyprland.md"));
        assert!(!main.contains("secret.sh"), "only .md/.txt reference files are offered");
        assert_eq!(s.read("omarchy", Some("hyprland.md")).unwrap(), "hypr notes");
        assert!(s.read("nope", None).unwrap_err().contains("Available: omarchy, other"));
    }

    #[test]
    fn reads_cannot_leave_the_skill_folder() {
        let s = Skills::load(&[skills_dir()]);
        for bad in ["../outside.md", "/etc/passwd", "secret.sh", "", "a//b.md", "..", "../other/SKILL.md"] {
            assert!(s.read("omarchy", Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn symlinked_skill_folders_are_followed() {
        let d = std::path::PathBuf::from(skills_dir());
        let link_dir = std::env::temp_dir().join(format!("loki-skills-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&link_dir);
        std::fs::create_dir_all(&link_dir).unwrap();
        std::os::unix::fs::symlink(d.join("omarchy"), link_dir.join("omarchy")).unwrap();
        let s = Skills::load(&[link_dir.display().to_string()]);
        assert_eq!(s.names(), vec!["omarchy"]);
        assert!(s.read("omarchy", None).unwrap().contains("Main text."));
    }

    #[test]
    fn no_skills_means_no_prompt_block() {
        assert_eq!(Skills::load(&["/nonexistent/skills".to_string()]).prompt_block(), "");
    }
}
