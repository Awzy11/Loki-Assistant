//! the user's standing instructions (the vault `CLAUDE.md`) and persistent memory, as grounding
//! for whatever model Loki is running. Read once at startup (restart to pick up changes);
//! best-effort — a missing file is an empty block, never a startup failure.
//!
//! **Condensed, not dumped.** The whole instructions file is ~14k tokens and is re-read on every
//! model call, but a given turn needs a small part of it. So the system prompt carries only a
//! small always-on core plus a one-line index of the remaining sections; the model pulls a
//! section with `read_guidance(section)` and a memory file with `read_memory(file)` when the
//! task touches it. Routines (`/bod`, `/eod`, …) get the sections they depend on preloaded
//! (`routine_block`) so that path never depends on the model remembering to look.
//! `[context] condense = false` restores the old full dump.

use crate::config::ContextConfig;
use std::path::PathBuf;

#[derive(Debug, Clone)]
struct Section {
    title: String,
    body: String,
}

#[derive(Debug, Default)]
pub struct LocalContext {
    /// Appended to the system prompt.
    pub prompt_block: String,
    sections: Vec<Section>,
    memory_dir: Option<PathBuf>,
    routine_sections: Vec<String>,
}

/// Claude Code's own project-memory directory naming convention: the absolute working
/// directory with `/` replaced by `-`, prefixed with `-` (e.g. `/home/you/Desktop` ->
/// `-home-user-Desktop`). Internal to Claude Code, observed from this machine's own memory
/// path; fragile if that ever changes, but this targets one fixed personal setup.
fn memory_dir(routine_working_dir: &str) -> PathBuf {
    let slug = format!("-{}", routine_working_dir.trim_start_matches('/').replace('/', "-"));
    dirs::home_dir().unwrap_or_default().join(".claude/projects").join(slug).join("memory")
}

/// Splits markdown on `## ` headings. Text before the first `##` is the preamble (not a
/// section). Each section keeps its `###` subsections.
fn split_sections(md: &str) -> (String, Vec<Section>) {
    let mut preamble = String::new();
    let mut sections: Vec<Section> = Vec::new();
    for line in md.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            sections.push(Section { title: title.trim().to_string(), body: format!("{line}\n") });
        } else if let Some(last) = sections.last_mut() {
            last.body.push_str(line);
            last.body.push('\n');
        } else {
            preamble.push_str(line);
            preamble.push('\n');
        }
    }
    (preamble, sections)
}

fn first_sentence(body: &str) -> String {
    let text = body
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('|') && !l.starts_with('#') && !l.starts_with("```") && !l.starts_with("---"))
        .unwrap_or("");
    let cut = text.find(". ").map(|i| i + 1).unwrap_or(text.len());
    let s: String = text[..cut].chars().take(110).collect();
    s
}

fn matches_title(section: &Section, query: &str) -> bool {
    let (t, q) = (section.title.to_lowercase(), query.trim().to_lowercase());
    !q.is_empty() && (t == q || t.starts_with(&q) || t.contains(&q))
}

pub fn build(routine_working_dir: Option<&str>, cfg: &ContextConfig) -> LocalContext {
    let Some(dir) = routine_working_dir else { return LocalContext::default() };
    let claude_md = std::fs::read_to_string(PathBuf::from(dir).join("CLAUDE.md")).ok();
    let mem_dir = memory_dir(dir);
    let memory_index = std::fs::read_to_string(mem_dir.join("MEMORY.md")).ok();

    let mut block = String::new();
    let mut sections = Vec::new();
    if let Some(content) = claude_md.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !cfg.condense {
            block.push_str("\n\n--- Project instructions (CLAUDE.md) ---\n");
            block.push_str(content);
        } else {
            let (preamble, secs) = split_sections(content);
            block.push_str("\n\n--- the user's standing instructions (CLAUDE.md), condensed ---\n");
            block.push_str(preamble.trim());
            block.push('\n');
            let mut other: Vec<String> = Vec::new();
            for s in &secs {
                if cfg.always_sections.iter().any(|a| matches_title(s, a)) {
                    block.push_str(&format!("\n{}", s.body));
                } else {
                    other.push(format!("- {} — {}", s.title, first_sentence(&s.body)));
                }
            }
            if !other.is_empty() {
                block.push_str(
                    "\n### Other sections (not loaded — call `read_guidance` with the section name before working in that area)\n",
                );
                block.push_str(&other.join("\n"));
                block.push('\n');
            }
            sections = secs;
        }
    }
    if let Some(content) = memory_index.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        block.push_str("\n\n--- Persistent memory index ---\n");
        if cfg.condense {
            // One line per memory, hook text shortened; the full file is a `read_memory` away.
            for line in content.lines() {
                let l: String = line.chars().take(150).collect();
                block.push_str(&l);
                block.push('\n');
            }
            block.push_str("\nThese are one-line pointers. Call `read_memory` with a file name to load the full memory when it looks relevant.");
        } else {
            block.push_str(content);
            block.push_str("\n\nThis is the index only — individual memory files aren't loaded here.");
        }
    }
    LocalContext { prompt_block: block, sections, memory_dir: Some(mem_dir), routine_sections: cfg.routine_sections.clone() }
}

impl LocalContext {
    pub fn section_titles(&self) -> Vec<String> {
        self.sections.iter().map(|s| s.title.clone()).collect()
    }

    /// The full text of the best-matching section, or a list of what exists.
    pub fn guidance(&self, query: &str) -> String {
        match self.sections.iter().find(|s| matches_title(s, query)) {
            Some(s) => s.body.clone(),
            None => format!("No section matches '{query}'. Sections: {}", self.section_titles().join("; ")),
        }
    }

    /// The sections routines depend on, as a block to put before a slash command's text.
    pub fn routine_block(&self) -> Option<String> {
        let bodies: Vec<&str> = self
            .routine_sections
            .iter()
            .filter_map(|q| self.sections.iter().find(|s| matches_title(s, q)))
            .map(|s| s.body.as_str())
            .collect();
        (!bodies.is_empty()).then(|| format!("[Guidance loaded for this routine]\n{}\n[End of guidance]\n\n", bodies.join("\n")))
    }

    /// One memory file by name (`feedback_x.md`). Only plain `.md` names inside the memory
    /// directory are served.
    pub fn read_memory(&self, file: &str) -> Result<String, String> {
        let name = file.trim();
        let ok = !name.is_empty()
            && name.ends_with(".md")
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
            && !name.contains("..");
        if !ok {
            return Err("memory file names look like 'feedback_example.md'".to_string());
        }
        let dir = self.memory_dir.as_ref().ok_or("no memory directory is configured")?;
        std::fs::read_to_string(dir.join(name)).map_err(|_| format!("no memory file '{name}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "# CLAUDE.md\n\nIntro line.\n\n## Context\nThis is the context. More.\n\n## Conventions\nNaming rules apply. Details follow.\n### Sub\nsub text\n\n## Machines\n| a | b |\nHosts live here.\n";

    #[test]
    fn splits_sections_and_keeps_subsections() {
        let (pre, secs) = split_sections(MD);
        assert!(pre.contains("Intro line"));
        assert_eq!(secs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), vec!["Context", "Conventions", "Machines"]);
        assert!(secs[1].body.contains("sub text"));
    }

    #[test]
    fn index_line_is_the_first_prose_sentence() {
        let (_, secs) = split_sections(MD);
        assert_eq!(first_sentence(&secs[1].body), "Naming rules apply.");
        assert_eq!(first_sentence(&secs[2].body), "Hosts live here."); // table row skipped
    }

    fn ctx_from(md: &str, cfg: &ContextConfig) -> LocalContext {
        let dir = std::env::temp_dir().join(format!("loki-ctx-test-{}-{}", std::process::id(), md.len()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CLAUDE.md"), md).unwrap();
        build(Some(dir.to_str().unwrap()), cfg)
    }

    #[test]
    fn condensed_prompt_has_core_and_index_but_not_the_rest() {
        let cfg = ContextConfig { condense: true, always_sections: vec!["Context".into()], routine_sections: vec!["Conventions".into()], num_ctx: 24576, history_fraction: 0.5 };
        let c = ctx_from(MD, &cfg);
        assert!(c.prompt_block.contains("This is the context."));
        assert!(!c.prompt_block.contains("sub text"));
        assert!(c.prompt_block.contains("- Conventions — Naming rules apply."));
        assert!(c.prompt_block.contains("read_guidance"));
        assert!(c.guidance("conventions").contains("sub text")); // case-insensitive, full body
        assert!(c.guidance("nope").contains("Sections:"));
        assert!(c.routine_block().unwrap().contains("Naming rules apply."));
    }

    #[test]
    fn uncondensed_restores_the_full_dump() {
        let cfg = ContextConfig { condense: false, ..Default::default() };
        assert!(ctx_from(MD, &cfg).prompt_block.contains("sub text"));
    }

    #[test]
    fn memory_reads_are_confined_to_plain_md_names() {
        let c = LocalContext { memory_dir: Some(std::env::temp_dir()), ..Default::default() };
        for bad in ["../etc/passwd", "a/b.md", "x.txt", "", "..md"] {
            assert!(c.read_memory(bad).is_err(), "{bad}");
        }
        assert!(LocalContext::default().read_memory("x.md").is_err());
    }

    #[test]
    fn nothing_configured_is_empty() {
        assert_eq!(build(None, &ContextConfig::default()).prompt_block, "");
        assert_eq!(build(Some("/nonexistent/path/for/this/test"), &ContextConfig::default()).prompt_block, "");
    }
}
