//! The user's profile: where things live in their vault and how their backlog is organised. This is the one place that knows
//! folder names, so Loki works with any vault layout, not just the one it grew up with.
//!
//! Source of truth is `~/.config/loki/profile.toml` (or `$LOKI_PROFILE`). Every field is optional; what is missing falls back to
//! a generic blank-vault layout. Ready-made layouts ship under `packs/` (see `packs/scrum-zettelkasten/profile.toml`).
//! A profile file that exists but is wrong stops the server from starting instead of quietly writing to the wrong folders.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Vault {
    pub inbox: String,
    pub daily: String,
    pub weeks: String,
    pub months: String,
    pub quarters: String,
    pub backlog: String,
    pub projects: String,
    pub notes: String,
    pub mocs: String,
    pub sources: String,
    pub reference: String,
    pub templates: String,
    pub archive: String,
    /// Where AI routines and findings live; edits there always need confirmation.
    pub agent: String,
    /// Raw journal entries, one file each. Default `Journal`.
    pub journal: String,
    /// Where Loki writes a readable log of each conversation. Default `Loki Conversations`.
    pub conversations: String,
    /// Your dated-events file (vault-relative, ends in .md). Default: `<mocs>/Upcoming Events.md`.
    pub events: Option<String>,
    /// Where the morning news digests are written. Default: `<daily>/News`.
    pub news: Option<String>,
    /// Where the lead scans are written. Default: `<backlog>/Leads`.
    pub leads: Option<String>,
    /// Your note of current priorities by life area. Default: `<mocs>/Current Priorities.md`.
    pub priorities: Option<String>,
    /// The log of estimates against actuals. Default: `<reference>/Estimation Log.md`.
    pub estimation_log: Option<String>,
    /// The templates new daily and weekly notes are made from. Default: `<templates>/Daily Note.md` and `<templates>/Weekly Note.md`.
    pub daily_template: Option<String>,
    pub week_template: Option<String>,
    /// A note collecting what you have found you actually want. Default: `<mocs>/Found Wants.md`.
    pub values: Option<String>,
    /// Your 5- and 10-year plan by life area. Default: `<mocs>/Long-Term Vision.md`.
    pub long_term_vision: Option<String>,
}

impl Default for Vault {
    /// A blank vault: plain folder names, nothing numbered.
    fn default() -> Self {
        let s = |x: &str| x.to_string();
        Vault {
            inbox: s("Inbox"),
            daily: s("Daily"),
            weeks: s("Weeks"),
            months: s("Months"),
            quarters: s("Quarters"),
            backlog: s("Backlog"),
            projects: s("Projects"),
            notes: s("Notes"),
            mocs: s("MOCs"),
            sources: s("Sources"),
            reference: s("Reference"),
            templates: s("Templates"),
            archive: s("Archive"),
            agent: s("Loki"),
            journal: s("Journal"),
            conversations: s("Loki Conversations"),
            events: None,
            news: None,
            leads: None,
            priorities: None,
            estimation_log: None,
            daily_template: None,
            week_template: None,
            values: None,
            long_term_vision: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Backlog {
    /// Subfolders of the backlog (and of the archive). Empty = a flat backlog.
    pub areas: Vec<String>,
}

/// One value tracked in the frontmatter of every daily note (sleep, weight, mood, anything). The Rhythm dashboard and the day cards are
/// built from this list, so what you track is up to you.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Property {
    /// The frontmatter key in the daily note (`sleep_h`).
    pub key: String,
    /// What it is called on screen.
    pub label: String,
    /// `number`, `choice` or `text`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Unit shown after a number (`h`, `lbs`).
    pub unit: String,
    /// For `choice`: the allowed values, listed from best to worst (that order colours the Rhythm strips).
    pub options: Vec<String>,
    /// `energy` marks the one property that says how much you have today; Loki shortens the day when it is low.
    pub role: String,
    /// For the energy property: the values that mean a very low day (3 items, quiet dashboard) and a lighter day (5 items).
    pub low: Vec<String>,
    pub gentle: Vec<String>,
    /// How Rhythm summarises a number: `average`, `trend` (first to last), `average_nonzero`, or `none`.
    pub summary: String,
    /// How Rhythm draws a number: `line` (default) or `bars`.
    pub chart: String,
    /// A short word for one-line summaries on the day card (`slept`). Only properties with `compact = true` appear there.
    pub short: String,
    pub compact: bool,
    /// The routine that asks you for this value (`bod`, `eod`); `{{properties:<id>}}` in that routine's prompt lists it.
    pub asked_by: String,
}

/// The headings Loki looks for in a daily note: `## <name>` for each, and `### <routines>` inside the tasks section.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct DailySections {
    /// Your top priorities for the day.
    pub mits: String,
    /// The time-blocked plan (lines like `1300-1500 Deep work`).
    pub focus_windows: String,
    /// Checkbox tasks, grouped under `###` headings.
    pub tasks: String,
    /// A `###` group inside the tasks section for routine to-dos (`/bod`, `/eod`).
    pub routines: String,
    /// A checkbox list of daily habits.
    pub savers: String,
    pub weather: String,
    /// Quick captures made during the day, processed at the end of it.
    pub captures: String,
    /// The closing section the end-of-day routine fills in, and the `###` reflection inside it.
    pub end_of_day: String,
    pub reflection: String,
}

impl Default for DailySections {
    fn default() -> Self {
        let s = |x: &str| x.to_string();
        DailySections { mits: s("Top 3"), focus_windows: s("Schedule"), tasks: s("Tasks"), routines: s("Routines"), savers: s("Habits"), weather: s("Weather"), captures: s("Captures"), end_of_day: s("End of Day"), reflection: s("Reflection") }
    }
}

/// The headings Loki looks for in a weekly (sprint) note.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct WeekSections {
    pub goal: String,
    /// The section holding the sprint's work, with `###` groups for routines, the queue, what is done and this week's events.
    pub backlog: String,
    pub routines: String,
    pub queue: String,
    pub done: String,
    pub events: String,
    /// Other `###` groups inside the backlog section that are not task lists (a standing-commitments block, say).
    pub skip_groups: Vec<String>,
}

impl Default for WeekSections {
    fn default() -> Self {
        let s = |x: &str| x.to_string();
        WeekSections { goal: s("Goal"), backlog: s("Backlog"), routines: s("Routines"), queue: s("Queue"), done: s("Done"), events: s("Events"), skip_groups: Vec::new() }
    }
}

/// The labels Loki reads in the weekly note's capacity table and progress line.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct WeekLabels {
    /// A bold marker introducing the active items: `**<active_list>:**`.
    pub active_list: String,
    /// Row labels of the capacity table (`| Available | 10h |`).
    pub available: String,
    pub committed: String,
    pub buffer: String,
    /// The progress line, `Active: 3/5 ... Done: 2`.
    pub wip_active: String,
    pub wip_done: String,
}

impl Default for WeekLabels {
    fn default() -> Self {
        let s = |x: &str| x.to_string();
        WeekLabels { active_list: s("Active this sprint"), available: s("Available"), committed: s("Committed"), buffer: s("Buffer"), wip_active: s("Active"), wip_done: s("Done") }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Daily {
    pub properties: Vec<Property>,
    pub sections: DailySections,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Week {
    pub sections: WeekSections,
    pub labels: WeekLabels,
}

/// One thing you can run: a slash command (a markdown file of instructions) or an n8n workflow. Drives the buttons on each tab and the
/// routine to-dos Loki adds to your daily and weekly notes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Routine {
    /// The slash command's name (`bod`, from `bod.md`), or the workflow's slug when `run = "workflow"`.
    pub id: String,
    pub label: String,
    pub hint: String,
    /// `daily`, `weekly`, `monthly`, `quarterly` or `anytime` (the default).
    pub period: String,
    /// Which tabs show its button (`shape`, `week`, `backlog`, `countdowns`, `rhythm`, `news`, `signals`, `leads`, `saved`, ...).
    pub tabs: Vec<String>,
    /// `chat` (the default: start the slash command in a Loki conversation) or `workflow` (start the n8n workflow of that name).
    pub run: String,
    /// `url` makes the button ask for a link first (`ask` is the prompt).
    pub input: String,
    pub ask: String,
    /// For daily and weekly chat routines: the words of the to-do line Loki keeps in the note (`- [ ] /bod — <todo>`).
    pub todo: String,
}

/// Where you are, for the weather and for routines that mention it. All optional.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Location {
    pub name: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Who the assistant is working for. Used in the prompts that sort mail and find dates in text. All optional.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Person {
    /// First name or nickname, for replies drafted in your voice.
    pub name: String,
    /// One or two sentences of context ("a nurse with two kids, a rental property and a marathon in spring").
    pub about: String,
    /// Contact address sent to push services (web push requires one). Falls back to the project page.
    pub email: String,
}

/// Web addresses the dashboard cards link to (your own self-hosted tools). Leave one out and its card has no link.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Links {
    pub news: String,
    pub leads: String,
    pub workflows: String,
    pub zotero: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub vault: Vault,
    pub person: Person,
    pub links: Links,
    pub location: Location,
    pub backlog: Backlog,
    pub daily: Daily,
    pub week: Week,
    pub routines: Vec<Routine>,
}

/// A heading's text: no `#`, no line breaks, not empty.
fn valid_heading(h: &str) -> bool {
    let t = h.trim();
    !t.is_empty() && t == h && t.chars().count() <= 60 && !t.starts_with('#') && !t.chars().any(|c| c.is_control())
}

fn valid_key(k: &str) -> bool {
    !k.is_empty() && k.len() <= 40 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') && !k.starts_with(|c: char| c.is_ascii_digit())
}

fn valid_text(t: &str, max: usize) -> bool {
    !t.trim().is_empty() && t.chars().count() <= max && !t.chars().any(|c| c.is_control())
}

/// A folder name that is safe to join onto the vault root and to put inside a shell script: no quotes, `$`, backticks, `;`, `|`,
/// redirections, wildcards, backslashes, colons or control characters.
fn valid_folder(f: &str) -> bool {
    let f = f.trim();
    !f.is_empty()
        && f == f.trim_matches('/')
        && !f.starts_with('.')
        && !f.starts_with('~')
        && !f.contains("..")
        && !f.contains("//")
        && f.len() <= 120
        && !f.chars().any(|c| c.is_control() || "\"'`$\\;|<>*?:&".contains(c))
}

/// A vault-relative path made of safe segments (`A/B/C`), optionally required to end in `.md`.
fn valid_rel_path(p: &str, file: bool) -> bool {
    !p.is_empty() && p.split('/').all(valid_folder) && (!file || (p.ends_with(".md") && p.len() > 3))
}

impl Profile {
    /// Parses and checks a profile file's text.
    pub fn from_toml(text: &str) -> Result<Profile, String> {
        let p: Profile = toml::from_str(text).map_err(|e| format!("profile.toml is not valid: {e}"))?;
        p.validate()?;
        Ok(p)
    }

    pub fn validate(&self) -> Result<(), String> {
        let v = &self.vault;
        let folders = [("inbox", &v.inbox), ("daily", &v.daily), ("weeks", &v.weeks), ("months", &v.months), ("quarters", &v.quarters), ("backlog", &v.backlog), ("projects", &v.projects), ("notes", &v.notes), ("mocs", &v.mocs), ("sources", &v.sources), ("reference", &v.reference), ("templates", &v.templates), ("archive", &v.archive), ("agent", &v.agent), ("journal", &v.journal), ("conversations", &v.conversations)];
        for (name, f) in folders {
            if !valid_folder(f) {
                return Err(format!("profile.toml: vault.{name} = {f:?} must be a plain folder name relative to the vault (no leading or trailing slash, no \"..\")"));
            }
        }
        for (i, (a, fa)) in folders.iter().enumerate() {
            if let Some((b, _)) = folders.iter().skip(i + 1).find(|(_, fb)| fb.to_lowercase() == fa.to_lowercase()) {
                return Err(format!("profile.toml: vault.{a} and vault.{b} point at the same folder ({fa})"));
            }
        }
        for (name, u) in [("news", &self.links.news), ("leads", &self.links.leads), ("workflows", &self.links.workflows), ("zotero", &self.links.zotero)] {
            if !u.is_empty() && !(u.starts_with("http://") || u.starts_with("https://")) {
                return Err(format!("profile.toml: links.{name} must start with http:// or https://"));
            }
        }
        for (name, f) in [("events", &v.events), ("priorities", &v.priorities), ("estimation_log", &v.estimation_log), ("daily_template", &v.daily_template), ("week_template", &v.week_template), ("values", &v.values), ("long_term_vision", &v.long_term_vision)] {
            if let Some(f) = f {
                if !valid_rel_path(f, true) {
                    return Err(format!("profile.toml: vault.{name} = {f:?} must be a vault-relative path ending in .md"));
                }
            }
        }
        let loc = &self.location;
        if loc.name.chars().count() > 60 || loc.name.chars().any(|c| c.is_control()) || loc.latitude.is_some() != loc.longitude.is_some() || loc.latitude.is_some_and(|x| !(-90.0..=90.0).contains(&x)) || loc.longitude.is_some_and(|x| !(-180.0..=180.0).contains(&x)) {
            return Err("profile.toml: location needs a short name and, if given, both latitude (-90..90) and longitude (-180..180)".into());
        }
        for (name, f) in [("news", &v.news), ("leads", &v.leads)] {
            if let Some(f) = f {
                if !valid_rel_path(f, false) {
                    return Err(format!("profile.toml: vault.{name} = {f:?} must be a vault-relative folder"));
                }
            }
        }
        for a in &self.backlog.areas {
            if !valid_folder(a) || a.contains('/') {
                return Err(format!("profile.toml: backlog area {a:?} must be a single folder name"));
            }
        }
        let mut seen = std::collections::HashSet::new();
        if let Some(d) = self.backlog.areas.iter().find(|a| !seen.insert(a.to_lowercase())) {
            return Err(format!("profile.toml: backlog area {d:?} is listed twice"));
        }
        self.validate_sections()?;
        self.validate_routines()?;
        self.validate_properties()
    }

    fn validate_routines(&self) -> Result<(), String> {
        let slug = |s: &str| !s.is_empty() && s.len() <= 40 && s.starts_with(|c: char| c.is_ascii_lowercase()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        let mut seen = std::collections::HashSet::new();
        for r in &self.routines {
            let at = format!("profile.toml: routine {:?}", r.id);
            if !slug(&r.id) {
                return Err(format!("{at}: id must be lowercase letters, digits, - or _ (it is the command or workflow name)"));
            }
            if !seen.insert(r.id.clone()) {
                return Err(format!("{at} is listed twice"));
            }
            if !valid_text(&r.label, 40) || r.hint.chars().count() > 240 || r.hint.chars().any(|c| c.is_control()) {
                return Err(format!("{at}: label is required (40 characters) and hint is limited to 240"));
            }
            if !matches!(r.period.as_str(), "" | "daily" | "weekly" | "monthly" | "quarterly" | "anytime") {
                return Err(format!("{at}: period must be daily, weekly, monthly, quarterly or anytime"));
            }
            if r.tabs.len() > 12 || r.tabs.iter().any(|t| !slug(t)) {
                return Err(format!("{at}: tabs must be short lowercase names such as shape, week or news"));
            }
            if !matches!(r.run.as_str(), "" | "chat" | "workflow") {
                return Err(format!("{at}: run must be chat or workflow"));
            }
            if !matches!(r.input.as_str(), "" | "url") || r.ask.chars().count() > 60 || r.ask.chars().any(|c| c.is_control()) {
                return Err(format!("{at}: input must be empty or url, and ask at most 60 characters"));
            }
            if !r.todo.is_empty() {
                if !matches!(r.period.as_str(), "daily" | "weekly") || r.run == "workflow" || !valid_text(&r.todo, 60) || r.todo.contains('\n') {
                    return Err(format!("{at}: todo needs period daily or weekly, run chat, and a short single line"));
                }
            }
        }
        Ok(())
    }

    /// Routines whose button belongs on this tab, in the order they are listed.
    pub fn routines_for_tab(&self, tab: &str) -> Vec<&Routine> {
        self.routines.iter().filter(|r| r.tabs.iter().any(|t| t == tab)).collect()
    }

    /// `("/bod", "start the day")` for every chat routine of this kind (`day` or `week`) that keeps a to-do line in its note.
    pub fn routine_todos(&self, kind: &str) -> Vec<(String, String)> {
        let period = if kind == "week" { "weekly" } else { "daily" };
        self.routines.iter().filter(|r| r.period == period && !r.todo.is_empty() && r.run != "workflow").map(|r| (format!("/{}", r.id), r.todo.clone())).collect()
    }

    /// True if `slug` is an n8n workflow that a button may start.
    pub fn is_runnable_workflow(&self, slug: &str) -> bool {
        self.routines.iter().any(|r| r.run == "workflow" && r.id == slug)
    }

    fn validate_sections(&self) -> Result<(), String> {
        let d = &self.daily.sections;
        let w = &self.week.sections;
        let groups: [(&str, Vec<(&str, &String)>); 2] = [
            ("daily.sections", vec![("mits", &d.mits), ("focus_windows", &d.focus_windows), ("tasks", &d.tasks), ("routines", &d.routines), ("savers", &d.savers), ("weather", &d.weather), ("captures", &d.captures), ("end_of_day", &d.end_of_day), ("reflection", &d.reflection)]),
            ("week.sections", vec![("goal", &w.goal), ("backlog", &w.backlog), ("routines", &w.routines), ("queue", &w.queue), ("done", &w.done), ("events", &w.events)]),
        ];
        for (table, items) in groups {
            for (name, h) in &items {
                if !valid_heading(h) {
                    return Err(format!("profile.toml: {table}.{name} = {h:?} must be the heading's text: no leading #, no line breaks, at most 60 characters"));
                }
            }
            // `routines` is a ### group inside another section, so it may share a name with a ## heading; the ## headings may not share names
            let mut seen = std::collections::HashSet::new();
            for (name, h) in items.iter().filter(|(n, _)| *n != "routines" && *n != "queue" && *n != "done" && *n != "events" && *n != "reflection") {
                if !seen.insert(h.to_lowercase()) {
                    return Err(format!("profile.toml: {table}.{name} = {h:?} is used for two different sections"));
                }
            }
        }
        for g in &w.skip_groups {
            if !valid_heading(g) {
                return Err(format!("profile.toml: week.sections.skip_groups entry {g:?} must be a heading's text"));
            }
        }
        let l = &self.week.labels;
        for (name, v) in [("active_list", &l.active_list), ("available", &l.available), ("committed", &l.committed), ("buffer", &l.buffer), ("wip_active", &l.wip_active), ("wip_done", &l.wip_done)] {
            if !valid_heading(v) || v.contains(':') || v.contains('*') || v.contains('|') {
                return Err(format!("profile.toml: week.labels.{name} = {v:?} must be plain label text (no colon, * or |)"));
            }
        }
        let subs = [&w.routines, &w.queue, &w.done, &w.events];
        let mut seen = std::collections::HashSet::new();
        if let Some(h) = subs.iter().find(|h| !seen.insert(h.to_lowercase())) {
            return Err(format!("profile.toml: week.sections has {h:?} for two different groups"));
        }
        Ok(())
    }

    fn validate_properties(&self) -> Result<(), String> {
        let mut keys = std::collections::HashSet::new();
        let mut energy = 0;
        for p in &self.daily.properties {
            let at = format!("profile.toml: daily property {:?}", p.key);
            if !valid_key(&p.key) {
                return Err(format!("{at}: key must be letters, digits, _ or - (and not start with a digit)"));
            }
            if !keys.insert(p.key.to_lowercase()) {
                return Err(format!("{at} is listed twice"));
            }
            if !valid_text(&p.label, 40) {
                return Err(format!("{at}: label is required (40 characters at most)"));
            }
            if !p.asked_by.is_empty() && !(p.asked_by.len() <= 40 && p.asked_by.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')) {
                return Err(format!("{at}: asked_by must be a routine id such as bod"));
            }
            if p.unit.chars().count() > 12 || p.unit.chars().any(|c| c.is_control()) || p.short.chars().count() > 20 || p.short.chars().any(|c| c.is_control()) {
                return Err(format!("{at}: unit and short are limited to a few characters"));
            }
            match p.kind.as_str() {
                "number" => {
                    if !matches!(p.summary.as_str(), "" | "average" | "trend" | "average_nonzero" | "none") {
                        return Err(format!("{at}: summary must be average, trend, average_nonzero or none"));
                    }
                    if !matches!(p.chart.as_str(), "" | "line" | "bars") {
                        return Err(format!("{at}: chart must be line or bars"));
                    }
                }
                "choice" => {
                    if p.options.len() < 2 || p.options.len() > 8 || p.options.iter().any(|o| !valid_text(o, 30)) {
                        return Err(format!("{at}: a choice needs 2 to 8 options, listed from best to worst"));
                    }
                }
                "text" => {}
                other => return Err(format!("{at}: type must be number, choice or text (not {other:?})")),
            }
            match p.role.as_str() {
                "" => {
                    if !p.low.is_empty() || !p.gentle.is_empty() {
                        return Err(format!("{at}: low and gentle only apply to the energy property"));
                    }
                }
                "energy" => {
                    energy += 1;
                    if p.kind != "choice" {
                        return Err(format!("{at}: the energy property must be a choice"));
                    }
                    let known = |v: &Vec<String>| v.iter().all(|x| p.options.iter().any(|o| o.eq_ignore_ascii_case(x)));
                    if p.low.is_empty() || !known(&p.low) || !known(&p.gentle) || p.low.iter().any(|l| p.gentle.iter().any(|g| g.eq_ignore_ascii_case(l))) {
                        return Err(format!("{at}: low (required) and gentle must be options of this property and not overlap"));
                    }
                }
                other => return Err(format!("{at}: role must be empty or energy (not {other:?})")),
            }
        }
        if energy > 1 {
            return Err("profile.toml: only one daily property can have role = \"energy\"".into());
        }
        Ok(())
    }

    /// The property that says how much energy you have today, if you track one.
    pub fn energy(&self) -> Option<&Property> {
        self.daily.properties.iter().find(|p| p.role == "energy")
    }

    /// `low`, `gentle` or `normal` for a value of the energy property (anything else, or no such property, is normal).
    pub fn energy_mode(&self, value: &str) -> &'static str {
        let Some(p) = self.energy() else { return "normal" };
        let v = value.trim();
        if p.low.iter().any(|x| x.eq_ignore_ascii_case(v)) {
            "low"
        } else if p.gentle.iter().any(|x| x.eq_ignore_ascii_case(v)) {
            "gentle"
        } else {
            "normal"
        }
    }

    // ---- routine prompts: placeholders ----

    /// The text for one `{{key}}` in a routine prompt, or None if the key isn't one of ours (it is then left exactly as written, which keeps
    /// other `{{...}}` uses such as `{{result:tool.field}}` intact).
    pub fn placeholder(&self, key: &str) -> Option<String> {
        let v = &self.vault;
        let d = &self.daily.sections;
        let w = &self.week.sections;
        let l = &self.week.labels;
        let s = |x: &str| Some(x.to_string());
        match key {
            "vault.inbox" => s(&v.inbox),
            "vault.daily" => s(&v.daily),
            "vault.weeks" => s(&v.weeks),
            "vault.months" => s(&v.months),
            "vault.quarters" => s(&v.quarters),
            "vault.backlog" => s(&v.backlog),
            "vault.projects" => s(&v.projects),
            "vault.notes" => s(&v.notes),
            "vault.mocs" => s(&v.mocs),
            "vault.sources" => s(&v.sources),
            "vault.reference" => s(&v.reference),
            "vault.templates" => s(&v.templates),
            "vault.archive" => s(&v.archive),
            "vault.agent" => s(&v.agent),
            "vault.journal" => s(&v.journal),
            "vault.conversations" => s(&v.conversations),
            "events" => Some(self.events_path()),
            "priorities" => Some(self.priorities_path()),
            "values" => Some(self.values_path()),
            "long_term_vision" => Some(self.long_term_vision_path()),
            "estimation_log" => Some(self.estimation_log_path()),
            "daily_template" => Some(self.daily_template_path()),
            "week_template" => Some(self.week_template_path()),
            "section.mits" => s(&d.mits),
            "section.focus_windows" => s(&d.focus_windows),
            "section.tasks" => s(&d.tasks),
            "section.routines" => s(&d.routines),
            "section.savers" => s(&d.savers),
            "section.weather" => s(&d.weather),
            "section.captures" => s(&d.captures),
            "section.end_of_day" => s(&d.end_of_day),
            "section.reflection" => s(&d.reflection),
            "week.goal" => s(&w.goal),
            "week.backlog" => s(&w.backlog),
            "week.routines" => s(&w.routines),
            "week.queue" => s(&w.queue),
            "week.done" => s(&w.done),
            "week.events" => s(&w.events),
            "label.active_list" => s(&l.active_list),
            "label.available" => s(&l.available),
            "label.committed" => s(&l.committed),
            "label.buffer" => s(&l.buffer),
            "label.wip_active" => s(&l.wip_active),
            "label.wip_done" => s(&l.wip_done),
            "backlog.item" => Some(if self.backlog.areas.is_empty() { format!("{}/<Title>.md", v.backlog) } else { format!("{}/<area>/<Title>.md", v.backlog) }),
            "archive.item" => Some(if self.backlog.areas.is_empty() { format!("{}/<Title>.md", v.archive) } else { format!("{}/<area>/<Title>.md", v.archive) }),
            "areas_note" => Some(if self.backlog.areas.is_empty() { String::new() } else { format!(" Pick the area from: {}.", self.backlog.areas.join(", ")) }),
            "areas" => Some(if self.backlog.areas.is_empty() { "(none: the backlog is a single folder)".to_string() } else { self.backlog.areas.join(", ") }),
            "location.name" => Some(if self.location.name.is_empty() { "(not set)".to_string() } else { self.location.name.clone() }),
            "energy_note" => Some(self.energy_note()),
            k if k.starts_with("properties:") => Some(self.properties_block(&k["properties:".len()..])),
            _ => None,
        }
    }

    /// What to ask for and where to save it, for the properties a routine is responsible for.
    pub fn properties_block(&self, routine: &str) -> String {
        let lines: Vec<String> = self
            .daily
            .properties
            .iter()
            .filter(|p| p.asked_by == routine)
            .map(|p| {
                let how = match p.kind.as_str() {
                    "number" => format!("a number{}", if p.unit.is_empty() { String::new() } else { format!(" ({})", p.unit) }),
                    "choice" => format!("one of {}", p.options.join(" / ")),
                    _ => "a short answer".to_string(),
                };
                format!("- **{}** — {how}. Save it as `{}` in the note's frontmatter ({}).", p.label, p.key, if p.kind == "number" { "the number only, no unit" } else { "the exact word" })
            })
            .collect();
        if lines.is_empty() {
            "(No properties are set up for this routine, so there is nothing to ask here. Skip this check-in.)".to_string()
        } else {
            lines.join("\n")
        }
    }

    /// The "lighter day" behaviour for the energy property, or an empty string when none is tracked.
    pub fn energy_note(&self) -> String {
        let Some(p) = self.energy() else { return String::new() };
        let join = |v: &Vec<String>| v.join(" or ");
        let gentle = if p.gentle.is_empty() { String::new() } else { format!(" **{}** means a lighter day: a shorter list and gentler framing.", join(&p.gentle)) };
        format!(
            "Today's **{label}** (`{key}`) sets how much to plan. **{low}** means a very low day: plan only what matters most (three items at most), keep the schedule light, and leave out tallies, streaks and stalled-item language.{gentle} Any other value is a normal day. Present the lighter plan as a default that is already in place, which the user can decline with no explanation. Never frame it as a rule.",
            label = p.label,
            key = p.key,
            low = join(&p.low),
        )
    }

    /// How prompts refer to the user: the profile name, or "the user".
    pub fn person_name(&self) -> String {
        if self.person.name.trim().is_empty() { "the user".into() } else { self.person.name.trim().to_string() }
    }

    /// "<name>" or "<name>, <about>" for a prompt that introduces the user.
    pub fn person_intro(&self) -> String {
        let about = self.person.about.trim();
        if about.is_empty() { self.person_name() } else { format!("{}, {}", self.person_name(), about.trim_end_matches('.')) }
    }

    /// The configured web address for a card ("news", "leads", "workflows", "zotero"), if any.
    pub fn link(&self, which: &str) -> Option<String> {
        let l = match which {
            "news" => &self.links.news,
            "leads" => &self.links.leads,
            "workflows" => &self.links.workflows,
            "zotero" => &self.links.zotero,
            _ => return None,
        };
        let l = l.trim();
        if l.is_empty() { None } else { Some(l.to_string()) }
    }

    /// The link to one n8n workflow, from the `workflows` base address.
    pub fn workflow_link(&self, id: &str) -> Option<String> {
        self.link("workflows").map(|b| format!("{}/workflow/{id}", b.trim_end_matches('/')))
    }

    pub fn push_contact(&self) -> String {
        let e = self.person.email.trim();
        if e.is_empty() { "https://github.com/Awzy11/Loki-Assistant".into() } else { format!("mailto:{e}") }
    }

    /// Fills the `{{...}}` placeholders in a routine prompt from this profile.
    pub fn render(&self, text: &str) -> String {
        static RE: OnceLock<regex::Regex> = OnceLock::new();
        let re = RE.get_or_init(|| regex::Regex::new(r"\{\{\s*([A-Za-z0-9_.:\-]+)\s*\}\}").unwrap());
        re.replace_all(text, |c: &regex::Captures| self.placeholder(&c[1]).unwrap_or_else(|| c[0].to_string())).into_owned()
    }

    /// The Zettelkasten + Personal Scrum starter pack, as shipped in `packs/`.
    pub fn starter() -> Profile {
        Profile::from_toml(include_str!("../../packs/scrum-zettelkasten/profile.toml")).expect("the shipped starter pack is valid")
    }

    // ---- paths ----

    /// `<folder>/` for prefix checks.
    pub fn prefix(folder: &str) -> String {
        format!("{folder}/")
    }

    /// True when `path` is inside `folder`.
    pub fn inside(path: &str, folder: &str) -> bool {
        path.starts_with(&Self::prefix(folder))
    }

    /// Folders Loki never writes into on its own: finished work, templates, the AI instructions, today's notes and Obsidian's own files.
    pub fn protected_prefixes(&self) -> Vec<String> {
        vec![Self::prefix(&self.vault.archive), Self::prefix(&self.vault.templates), Self::prefix(&self.vault.agent), Self::prefix(&self.vault.daily), ".obsidian/".to_string()]
    }

    // ---- derived places ----

    pub fn events_path(&self) -> String {
        self.vault.events.clone().unwrap_or_else(|| format!("{}/Upcoming Events.md", self.vault.mocs))
    }

    pub fn news_dir(&self) -> String {
        self.vault.news.clone().unwrap_or_else(|| format!("{}/News", self.vault.daily))
    }

    pub fn leads_dir(&self) -> String {
        self.vault.leads.clone().unwrap_or_else(|| format!("{}/Leads", self.vault.backlog))
    }

    pub fn priorities_path(&self) -> String {
        self.vault.priorities.clone().unwrap_or_else(|| format!("{}/Current Priorities.md", self.vault.mocs))
    }

    pub fn values_path(&self) -> String {
        self.vault.values.clone().unwrap_or_else(|| format!("{}/Found Wants.md", self.vault.mocs))
    }

    pub fn long_term_vision_path(&self) -> String {
        self.vault.long_term_vision.clone().unwrap_or_else(|| format!("{}/Long-Term Vision.md", self.vault.mocs))
    }

    pub fn estimation_log_path(&self) -> String {
        self.vault.estimation_log.clone().unwrap_or_else(|| format!("{}/Estimation Log.md", self.vault.reference))
    }

    pub fn daily_template_path(&self) -> String {
        self.vault.daily_template.clone().unwrap_or_else(|| format!("{}/Daily Note.md", self.vault.templates))
    }

    pub fn week_template_path(&self) -> String {
        self.vault.week_template.clone().unwrap_or_else(|| format!("{}/Weekly Note.md", self.vault.templates))
    }

    pub fn daily_path(&self, date: &str) -> String {
        format!("{}/{date}.md", self.vault.daily)
    }

    pub fn week_path(&self, week: &str) -> String {
        format!("{}/{week}.md", self.vault.weeks)
    }

    /// Where a finished week's daily notes are filed: `<archive>/Weekly/<week>/<date>.md`.
    pub fn archived_daily_path(&self, week: &str, date: &str) -> String {
        format!("{}/Weekly/{week}/{date}.md", self.vault.archive)
    }

    /// `(label, folder)` for each place backlog items live: one per area, or the backlog folder itself when it is flat.
    pub fn backlog_dirs(&self) -> Vec<(String, String)> {
        if self.backlog.areas.is_empty() {
            vec![(self.vault.backlog.clone(), self.vault.backlog.clone())]
        } else {
            self.backlog.areas.iter().map(|a| (a.clone(), format!("{}/{a}", self.vault.backlog))).collect()
        }
    }

    /// Folders whose notes Loki may link to from a card (`<folder>/` prefixes).
    pub fn linkable_prefixes(&self) -> Vec<String> {
        let v = &self.vault;
        [&v.backlog, &v.projects, &v.notes, &v.mocs, &v.reference].iter().map(|f| Self::prefix(f)).collect()
    }

    /// True for a note Loki may tick tasks in: directly inside the daily or weekly folder.
    pub fn is_task_note(&self, p: &str) -> bool {
        (Self::inside(p, &self.vault.daily) || Self::inside(p, &self.vault.weeks)) && p.ends_with(".md") && !p.contains("..") && p.matches('/').count() == 1
    }

    /// `<backlog>/<area>/<file>.md` (or `<backlog>/<file>.md` for a flat backlog) and nothing else.
    pub fn is_backlog_item(&self, p: &str) -> bool {
        let parts: Vec<&str> = p.split('/').collect();
        let file = parts.last().copied().unwrap_or("");
        let shape_ok = if self.backlog.areas.is_empty() {
            parts.len() == 2 && parts[0] == self.vault.backlog
        } else {
            parts.len() == 3 && parts[0] == self.vault.backlog && self.backlog.areas.iter().any(|a| a == parts[1])
        };
        shape_ok && file.ends_with(".md") && file.len() > 3 && !p.contains("..") && !file.starts_with('.')
    }

    /// Where a closed backlog item is filed: `<archive>/<area>/<file>` (or `<archive>/<file>`), or None if the path isn't a backlog item.
    pub fn archive_target(&self, backlog_path: &str) -> Option<String> {
        if !self.is_backlog_item(backlog_path) {
            return None;
        }
        let parts: Vec<&str> = backlog_path.split('/').collect();
        Some(if self.backlog.areas.is_empty() { format!("{}/{}", self.vault.archive, parts[1]) } else { format!("{}/{}/{}", self.vault.archive, parts[1], parts[2]) })
    }
}

fn path() -> PathBuf {
    std::env::var("LOKI_PROFILE").map(PathBuf::from).unwrap_or_else(|_| dirs::config_dir().unwrap_or_default().join("loki").join("profile.toml"))
}

static PROFILE: OnceLock<Profile> = OnceLock::new();

/// Loads the profile once. A missing file means the blank-vault defaults; a file that is present but wrong is an error.
pub fn init() -> Result<&'static Profile, String> {
    if let Some(p) = PROFILE.get() {
        return Ok(p);
    }
    let explicit = std::env::var("LOKI_PROFILE").is_ok();
    let p = match std::fs::read_to_string(path()) {
        Ok(text) => Profile::from_toml(&text)?,
        // no file at the usual place = a fresh install on the blank-vault defaults; a path that was asked for by name must exist
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => Profile::default(),
        Err(e) => return Err(format!("could not read {}: {e}", path().display())),
    };
    Ok(PROFILE.get_or_init(|| p))
}

/// The active profile. `init()` should have been called at startup; if it wasn't (tools) this falls back to the defaults.
#[cfg(not(test))]
pub fn get() -> &'static Profile {
    PROFILE.get().unwrap_or_else(|| PROFILE.get_or_init(|| init().map(|p| p.clone()).unwrap_or_default()))
}

/// In tests the active profile is the shipped starter pack, so path tests check the numbered layout; the blank defaults are tested directly.
#[cfg(test)]
pub fn get() -> &'static Profile {
    static T: OnceLock<Profile> = OnceLock::new();
    T.get_or_init(Profile::starter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_vault_defaults_are_valid_and_plain() {
        let p = Profile::default();
        assert!(p.validate().is_ok());
        assert_eq!(p.vault.inbox, "Inbox");
        assert!(p.backlog.areas.is_empty());
    }

    #[test]
    fn starter_pack_is_the_numbered_layout() {
        let p = Profile::starter();
        assert_eq!(p.vault.backlog, "30 - Backlog");
        assert_eq!(p.vault.archive, "99 - Archive");
        assert_eq!(p.backlog.areas, vec!["Tech", "Home", "School", "Self"]);
    }

    #[test]
    fn partial_files_fall_back_per_field() {
        let p = Profile::from_toml("[vault]\ninbox = \"Capture\"\n").unwrap();
        assert_eq!(p.vault.inbox, "Capture");
        assert_eq!(p.vault.daily, "Daily");
    }

    #[test]
    fn bad_profiles_are_refused() {
        for bad in ["[vault]\ninbox = \"../outside\"\n", "[vault]\ninbox = \"/abs\"\n", "[vault]\ninbox = \"Inbox/\"\n", "[vault]\ninbox = \"Notes\"\n", "[vault]\ninbox = \"\"\n", "[backlog]\nareas = [\"A/B\"]\n", "[backlog]\nareas = [\"A\", \"a\"]\n", "[vault\n"] {
            assert!(Profile::from_toml(bad).is_err(), "should refuse: {bad:?}");
        }
    }

    #[test]
    fn backlog_item_paths_follow_the_profile() {
        let s = Profile::starter();
        assert!(s.is_backlog_item("30 - Backlog/Tech/Thing.md"));
        assert!(!s.is_backlog_item("30 - Backlog/Thing.md"));
        assert!(!s.is_backlog_item("30 - Backlog/Other/Thing.md"));
        assert!(!s.is_backlog_item("30 - Backlog/Tech/../../x.md"));
        assert!(!s.is_backlog_item("10 - Daily/2026-10-28.md"));
        let flat = Profile::default();
        assert!(flat.is_backlog_item("Backlog/Thing.md"));
        assert!(!flat.is_backlog_item("Backlog/Tech/Thing.md"));
    }

    #[test]
    fn archive_targets() {
        assert_eq!(Profile::starter().archive_target("30 - Backlog/Tech/Thing.md").as_deref(), Some("99 - Archive/Tech/Thing.md"));
        assert_eq!(Profile::default().archive_target("Backlog/Thing.md").as_deref(), Some("Archive/Thing.md"));
        assert_eq!(Profile::starter().archive_target("50 - Notes/x.md"), None);
    }

    #[test]
    fn derived_places_follow_the_profile() {
        let s = Profile::starter();
        assert_eq!(s.events_path(), "60 - MOCs/Upcoming Events.md");
        assert_eq!(s.news_dir(), "10 - Daily/News");
        assert_eq!(s.leads_dir(), "30 - Backlog/Tech/Scout Leads");
        assert_eq!(s.daily_path("2026-10-29"), "10 - Daily/2026-10-29.md");
        assert_eq!(s.archived_daily_path("2026-W43", "2026-10-22"), "99 - Archive/Weekly/2026-W43/2026-10-22.md");
        assert_eq!(s.backlog_dirs()[0], ("Tech".to_string(), "30 - Backlog/Tech".to_string()));
        assert!(s.linkable_prefixes().contains(&"60 - MOCs/".to_string()));
        let d = Profile::default();
        assert_eq!(d.events_path(), "MOCs/Upcoming Events.md");
        assert_eq!(d.news_dir(), "Daily/News");
        assert_eq!(d.backlog_dirs(), vec![("Backlog".to_string(), "Backlog".to_string())]);
        let moved = Profile::from_toml("[vault]\nmocs = \"Maps\"\n").unwrap();
        assert_eq!(moved.events_path(), "Maps/Upcoming Events.md");
    }

    #[test]
    fn task_notes_are_daily_and_weekly_files_only() {
        let s = Profile::starter();
        assert!(s.is_task_note("10 - Daily/2026-10-29.md") && s.is_task_note("20 - Weeks/2026-W44.md"));
        for bad in ["10 - Daily/sub/x.md", "10 - Daily/x.txt", "10 - Daily/../AGENTS.md", "30 - Backlog/Home/x.md", "AI Infrastructure/bod.md"] {
            assert!(!s.is_task_note(bad), "{bad}");
        }
    }

    #[test]
    fn unsafe_folder_names_are_refused() {
        for bad in ["a\"b", "a$b", "a`b", "a;b", "a|b", "a'b", "a\\b", "a*b", "a:b"] {
            assert!(Profile::from_toml(&format!("[vault]\ninbox = {bad:?}\n")).is_err(), "should refuse {bad}");
        }
        assert!(Profile::from_toml("[vault]\nevents = \"Maps/Events\"\n").is_err());
        assert!(Profile::from_toml("[vault]\nnews = \"../x\"\n").is_err());
        assert!(Profile::from_toml("[vault]\nnews = \"Daily/My News\"\nleads = \"Backlog/Tech/Leads\"\nevents = \"Maps/Events.md\"\n").is_ok());
    }

    #[test]
    fn starter_properties_and_the_energy_role() {
        let s = Profile::starter();
        assert_eq!(s.daily.properties.len(), 4);
        let e = s.energy().unwrap();
        assert_eq!(e.key, "starting_state");
        assert_eq!((s.energy_mode("Depleted"), s.energy_mode("partial"), s.energy_mode("Fresh"), s.energy_mode(""), s.energy_mode("???")), ("low", "gentle", "normal", "normal", "normal"));
        assert_eq!(Profile::default().energy_mode("Depleted"), "normal");
        assert!(Profile::default().daily.properties.is_empty());
    }

    #[test]
    fn property_schema_is_checked() {
        let base = "[[daily.properties]]\nkey = \"mood\"\nlabel = \"Mood\"\n";
        assert!(Profile::from_toml(&format!("{base}type = \"number\"\nunit = \"/10\"\nsummary = \"average\"\n")).is_ok());
        for bad in [
            format!("{base}type = \"colour\"\n"),
            format!("{base}type = \"choice\"\noptions = [\"only\"]\n"),
            format!("{base}type = \"number\"\nsummary = \"median\"\n"),
            format!("{base}type = \"number\"\nlow = [\"x\"]\n"),
            format!("{base}type = \"number\"\nrole = \"energy\"\n"),
            format!("{base}type = \"choice\"\noptions = [\"Good\",\"Bad\"]\nrole = \"energy\"\n"),
            format!("{base}type = \"choice\"\noptions = [\"Good\",\"Bad\"]\nrole = \"energy\"\nlow = [\"Awful\"]\n"),
            format!("{base}type = \"choice\"\noptions = [\"Good\",\"Bad\"]\nrole = \"energy\"\nlow = [\"Bad\"]\ngentle = [\"bad\"]\n"),
            format!("{base}type = \"text\"\n{base}type = \"text\"\n"),
            "[[daily.properties]]\nkey = \"1bad key\"\nlabel = \"X\"\ntype = \"text\"\n".to_string(),
            "[[daily.properties]]\nkey = \"ok\"\nlabel = \"\"\ntype = \"text\"\n".to_string(),
        ] {
            assert!(Profile::from_toml(&bad).is_err(), "should refuse:\n{bad}");
        }
        let two = "[[daily.properties]]\nkey=\"a\"\nlabel=\"A\"\ntype=\"choice\"\noptions=[\"x\",\"y\"]\nrole=\"energy\"\nlow=[\"y\"]\n[[daily.properties]]\nkey=\"b\"\nlabel=\"B\"\ntype=\"choice\"\noptions=[\"x\",\"y\"]\nrole=\"energy\"\nlow=[\"y\"]\n";
        assert!(Profile::from_toml(two).is_err());
    }

    #[test]
    fn section_headings_follow_the_profile() {
        let s = Profile::starter();
        assert_eq!(s.daily.sections.mits, "Today's 3");
        assert_eq!(s.daily.sections.savers, "SAVERS");
        assert_eq!(s.week.sections.backlog, "Sprint Backlog");
        assert_eq!(s.week.sections.queue, "⏳ Queue");
        let d = Profile::default();
        assert_eq!(d.daily.sections.tasks, "Tasks");
        assert_eq!(d.week.sections.goal, "Goal");
        let p = Profile::from_toml("[daily.sections]\nmits = \"Priorities\"\n").unwrap();
        assert_eq!(p.daily.sections.mits, "Priorities");
        assert_eq!(p.daily.sections.tasks, "Tasks");
        for bad in ["[daily.sections]\nmits = \"## Top\"\n", "[daily.sections]\nmits = \"\"\n", "[daily.sections]\nmits = \" padded \"\n", "[daily.sections]\nmits = \"Tasks\"\n", "[week.sections]\nqueue = \"Done\"\n"] {
            assert!(Profile::from_toml(bad).is_err(), "should refuse: {bad:?}");
        }
    }

    #[test]
    fn week_labels_and_skip_groups() {
        let s = Profile::starter();
        assert_eq!(s.week.sections.skip_groups, vec!["Standing Commitments"]);
        assert_eq!(s.week.labels.available, "Available");
        assert_eq!(s.week.labels.active_list, "Active this sprint");
        assert!(Profile::from_toml("[week.labels]\navailable = \"Cap:\"\n").is_err());
        assert!(Profile::from_toml("[week.labels]\nbuffer = \"Spare\"\n[week.sections]\nskip_groups = [\"Fixed\"]\n").is_ok());
    }

    #[test]
    fn routine_registry() {
        let s = Profile::starter();
        assert_eq!(s.routines.len(), 11);
        let shape: Vec<&str> = s.routines_for_tab("shape").iter().map(|r| r.id.as_str()).collect();
        assert_eq!(shape, vec!["bod", "eod", "low-cap"]);
        let cd: Vec<&str> = s.routines_for_tab("countdowns").iter().map(|r| r.id.as_str()).collect();
        assert_eq!(cd, vec!["bow"]);
        assert_eq!(s.routine_todos("day"), vec![("/bod".to_string(), "start the day".to_string()), ("/eod".to_string(), "close the day".to_string())]);
        assert_eq!(s.routine_todos("week")[0].0, "/bow");
        let wf = Profile::from_toml("[[routines]]\nid = \"nightly\"\nlabel = \"Nightly\"\nperiod = \"daily\"\nrun = \"workflow\"\n").unwrap();
        assert!(wf.is_runnable_workflow("nightly") && !s.is_runnable_workflow("bod") && !s.is_runnable_workflow("nope"));
        assert!(Profile::default().routines.is_empty() && Profile::default().routine_todos("day").is_empty());
        let base = "[[routines]]\nid = \"x\"\nlabel = \"X\"\n";
        assert!(Profile::from_toml(base).is_ok());
        let wf_url = Profile::from_toml("[[routines]]\nid = \"save-video\"\nlabel = \"Save a video\"\nrun = \"workflow\"\ninput = \"url\"\nask = \"Paste a link\"\n").unwrap();
        assert!(wf_url.is_runnable_workflow("save-video") && wf_url.routines[0].input == "url", "a workflow can ask for a link");
        for bad in [
            "[[routines]]\nid = \"Bad Id\"\nlabel = \"X\"\n".to_string(),
            format!("{base}[[routines]]\nid = \"x\"\nlabel = \"Y\"\n"),
            format!("{base}period = \"hourly\"\n"),
            format!("{base}run = \"shell\"\n"),
            format!("{base}input = \"file\"\n"),
            format!("{base}todo = \"do it\"\n"),
            format!("{base}period = \"daily\"\nrun = \"workflow\"\ntodo = \"do it\"\n"),
            format!("{base}tabs = [\"Not A Tab\"]\n"),
            "[[routines]]\nid = \"y\"\nlabel = \"\"\n".to_string(),
        ] {
            assert!(Profile::from_toml(&bad).is_err(), "should refuse:\n{bad}");
        }
    }

    #[test]
    fn person_defaults_and_intro() {
        let d = Profile::default();
        assert_eq!(d.person_intro(), "the user");
        assert!(d.push_contact().starts_with("https://"));
        let p = Profile::from_toml("[person]\nname = \"Sam\"\nabout = \"a nurse with two kids.\"\nemail = \"s@x.org\"\n").unwrap();
        assert_eq!(p.person_intro(), "Sam, a nurse with two kids");
        assert_eq!(p.push_contact(), "mailto:s@x.org");
    }

    #[test]
    fn extra_paths_and_location() {
        let s = Profile::starter();
        assert_eq!(s.priorities_path(), "60 - MOCs/Current Priorities.md");
        assert_eq!(s.estimation_log_path(), "80 - Reference/System/Estimation Log.md");
        assert_eq!(s.daily_template_path(), "90 - Templates/Daily Note.md");
        let d = Profile::default();
        assert_eq!((d.priorities_path().as_str(), d.estimation_log_path().as_str(), d.week_template_path().as_str()), ("MOCs/Current Priorities.md", "Reference/Estimation Log.md", "Templates/Weekly Note.md"));
        assert!(Profile::from_toml("[location]\nname = \"Home\"\nlatitude = 38.5\nlongitude = -77.3\n").is_ok());
        for bad in ["[location]\nlatitude = 38.5\n", "[location]\nlatitude = 95.0\nlongitude = 0.0\n", "[location]\nlongitude = -190.0\nlatitude = 0.0\n", "[vault]\npriorities = \"Maps/Prio\"\n", "[vault]\nestimation_log = \"../x.md\"\n"] {
            assert!(Profile::from_toml(bad).is_err(), "should refuse: {bad:?}");
        }
    }

    #[test]
    fn placeholders_fill_from_the_profile_and_leave_everything_else_alone() {
        let s = Profile::starter();
        assert_eq!(s.render("Read {{vault.daily}}/ and `{{ section.tasks }}`, then {{events}}."), "Read 10 - Daily/ and `Tasks`, then 60 - MOCs/Upcoming Events.md.");
        assert_eq!(s.render("{{week.queue}} / {{label.buffer}} / {{areas}}"), "⏳ Queue / Buffer / Tech, Home, School, Self");
        assert_eq!(s.render("{{backlog.item}}|{{archive.item}}|{{areas_note}}"), "30 - Backlog/<area>/<Title>.md|99 - Archive/<area>/<Title>.md| Pick the area from: Tech, Home, School, Self.");
        assert_eq!(Profile::default().render("{{backlog.item}}|{{archive.item}}|{{areas_note}}"), "Backlog/<Title>.md|Archive/<Title>.md|");
        // not ours: unknown keys and the tool-pipe syntax stay exactly as written
        assert_eq!(s.render("{{result:get_transcript.transcript}} {{nope}} {{ vault.nope }}"), "{{result:get_transcript.transcript}} {{nope}} {{ vault.nope }}");
        let bod = s.render("{{properties:bod}}");
        assert!(bod.contains("**Sleep**") && bod.contains("`sleep_h`") && bod.contains("one of Fresh / Partial / Depleted") && bod.contains("the number only"), "{bod}");
        assert!(!bod.contains("Depletion"), "depletion is asked at EOD");
        assert!(s.render("{{properties:eod}}").contains("`day_rating`"));
        assert!(s.render("{{properties:ghost}}").starts_with("(No properties"));
        let e = s.render("{{energy_note}}");
        assert!(e.contains("Energy this morning") && e.contains("**Depleted**") && e.contains("**Partial**") && e.contains("decline"), "{e}");
        let d = Profile::default();
        assert_eq!(d.render("{{energy_note}}"), "");
        assert_eq!(d.render("{{section.tasks}} {{vault.inbox}} {{location.name}}"), "Tasks Inbox (not set)");
    }

    #[test]
    fn protected_folders_follow_the_profile() {
        let p = Profile::starter();
        let prot = p.protected_prefixes();
        for f in ["99 - Archive/", "90 - Templates/", "AI Infrastructure/", "10 - Daily/", ".obsidian/"] {
            assert!(prot.iter().any(|x| x == f), "{f} should be protected");
        }
    }
}
