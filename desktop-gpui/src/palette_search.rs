//! Ephemeral content index, updated with the catalog rather than on input events.
use super::{PaletteCategory, PaletteEntry, match_score};
use rusqlite::{Connection, params};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct PaletteSearch {
    connection: Option<Connection>,
    // One-deletion keys restrict typo candidates without scanning documents or vocabulary.
    vocabulary: HashSet<String>,
    documents: HashMap<String, (i64, String, String)>,
    next_rowid: i64,
    snippets: HashMap<String, String>,
    deletions: HashMap<String, Vec<String>>,
}
fn deletion_keys(word: &str) -> Vec<String> {
    let chars: Vec<_> = word.chars().collect();
    if chars.len() > 64 {
        return vec![];
    }
    (0..chars.len())
        .map(|skip| {
            chars
                .iter()
                .enumerate()
                .filter_map(|(i, c)| (i != skip).then_some(*c))
                .collect()
        })
        .collect()
}
fn tokens(query: &str) -> Vec<String> {
    query
        .chars()
        .take(256)
        .collect::<String>()
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .take(32)
        .map(str::to_owned)
        .collect()
}
fn quoted(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}
impl PaletteSearch {
    pub(super) fn update(&mut self, entries: &[PaletteEntry]) {
        if let Err(error) = self.update_index(entries) {
            eprintln!("Palette content index unavailable: {error}");
            *self = Self::default();
        }
    }
    fn update_index(&mut self, entries: &[PaletteEntry]) -> rusqlite::Result<()> {
        if self.connection.is_none() {
            let db = Connection::open_in_memory()?;
            db.execute_batch("CREATE VIRTUAL TABLE content USING fts5(body, metadata, tokenize='unicode61'); CREATE VIRTUAL TABLE vocabulary USING fts5vocab(content, 'row');")?;
            self.connection = Some(db);
        }
        let db = self.connection.as_mut().unwrap();
        let tx = db.transaction()?;
        let ids: HashSet<_> = entries.iter().map(|entry| entry.id.as_str()).collect();
        let removed: Vec<_> = self
            .documents
            .keys()
            .filter(|id| !ids.contains(id.as_str()))
            .cloned()
            .collect();
        let mut changed = !removed.is_empty();
        for id in removed {
            if let Some((rowid, _, _)) = self.documents.remove(&id) {
                tx.execute("DELETE FROM content WHERE rowid=?1", [rowid])?;
            }
        }
        for entry in entries {
            let metadata = format!(
                "{} {} {} {}",
                entry.label,
                entry.subtitle,
                entry.keywords,
                entry.kind.label()
            );
            if self
                .documents
                .get(&entry.id)
                .is_some_and(|(_, body, old_metadata)| {
                    body == &entry.search_text && old_metadata == &metadata
                })
            {
                continue;
            }
            changed = true;
            let rowid = if let Some((rowid, _, _)) = self.documents.get(&entry.id) {
                tx.execute("DELETE FROM content WHERE rowid=?1", [rowid])?;
                *rowid
            } else {
                self.next_rowid += 1;
                self.next_rowid
            };
            tx.execute(
                "INSERT INTO content(rowid,body,metadata) VALUES (?1,?2,?3)",
                params![rowid, entry.search_text, metadata],
            )?;
            self.documents.insert(
                entry.id.clone(),
                (rowid, entry.search_text.clone(), metadata),
            );
        }
        tx.commit()?;
        if changed {
            let words: Vec<String> = db.prepare("SELECT term FROM vocabulary WHERE length(term) BETWEEN 4 AND 24 ORDER BY doc DESC, term LIMIT 5000")?
                .query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
            self.vocabulary = words.iter().cloned().collect();
            self.deletions.clear();
            for word in words {
                for key in deletion_keys(&word) {
                    let candidates = self.deletions.entry(key).or_default();
                    if candidates.len() < 16 {
                        candidates.push(word.clone());
                    }
                }
            }
        }
        self.snippets.clear();
        Ok(())
    }
    pub(super) fn snippets(&self) -> &HashMap<String, String> {
        &self.snippets
    }
    fn expression(&self, query: &str, fuzzy: bool) -> Option<String> {
        let terms = tokens(query);
        if terms.is_empty() {
            return None;
        }
        Some(
            terms
                .iter()
                .map(|term| {
                    let mut alternatives = vec![format!("{}*", quoted(term))];
                    if fuzzy && term.chars().count() >= 4 {
                        let keys = std::iter::once(term.clone()).chain(deletion_keys(term));
                        for key in keys {
                            if self.vocabulary.contains(&key) {
                                alternatives.push(quoted(&key));
                            }
                            if let Some(words) = self.deletions.get(&key) {
                                alternatives.extend(words.iter().take(16).map(|word| quoted(word)));
                            }
                            if alternatives.len() >= 64 {
                                break;
                            }
                        }
                    }
                    alternatives.sort();
                    alternatives.dedup();
                    format!("({})", alternatives.join(" OR "))
                })
                .collect::<Vec<_>>()
                .join(" AND "),
        )
    }
    pub(super) fn ranked(
        &mut self,
        entries: &[PaletteEntry],
        category: PaletteCategory,
        query: &str,
    ) -> Vec<usize> {
        let mut content = HashMap::new();
        self.snippets.clear();
        // Exact token/prefix hits outrank vocabulary-expanded typo hits, regardless
        // of catalog order. Title and descriptor matches retain scores 0..5.
        for (fuzzy, score) in [(false, 6), (true, 7)] {
            if let (Some(db), Some(expression)) = (&self.connection, self.expression(query, fuzzy))
            {
                let hits = (|| -> rusqlite::Result<Vec<(i64, String)>> {
                    db.prepare("SELECT rowid, snippet(content,0,'','',' … ',24) FROM content WHERE content MATCH ?1")?
                        .query_map([expression], |row| Ok((row.get(0)?, row.get(1)?)))?.collect()
                })();
                if let Ok(hits) = hits {
                    let ids: HashMap<_, _> = self
                        .documents
                        .iter()
                        .map(|(id, (rowid, _, _))| (*rowid, id))
                        .collect();
                    for (rowid, snippet) in hits {
                        if let Some(id) = ids.get(&rowid) {
                            content.entry((*id).clone()).or_insert(score);
                            if !snippet.trim().is_empty() {
                                self.snippets.entry((*id).clone()).or_insert_with(|| {
                                    snippet.split_whitespace().collect::<Vec<_>>().join(" ")
                                });
                            }
                        }
                    }
                }
            }
        }
        let mut ranked: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| category.includes_entry(entry))
            .filter_map(|(i, entry)| {
                match_score(entry, query)
                    .or_else(|| content.get(&entry.id).copied())
                    .map(|score| (entry.open_priority, score, i))
            })
            .collect();
        ranked.sort_unstable();
        ranked.into_iter().map(|(_, _, i)| i).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_palette::EntryKind;
    fn entry(label: &str, text: &str, priority: u8) -> PaletteEntry {
        PaletteEntry {
            id: label.into(),
            label: label.into(),
            subtitle: String::new(),
            kind: EntryKind::Pane,
            keywords: String::new(),
            search_text: text.into(),
            open_priority: priority,
            disabled: false,
            checked: None,
            keyboard_hint: None,
        }
    }
    #[test]
    fn priority_then_title_then_full_content() {
        let entries = vec![
            entry("Other", "needle", 2),
            entry("Content", &format!("{} needle", "padding ".repeat(1000)), 0),
            entry("needle", "", 0),
        ];
        let mut index = PaletteSearch::default();
        index.update(&entries);
        assert_eq!(
            index.ranked(&entries, PaletteCategory::All, "needle"),
            vec![2, 1, 0]
        );
    }
    #[test]
    fn exact_and_prefix_content_outrank_typo_hits_in_reverse_catalog_order() {
        let entries = vec![
            entry("Typo first", "authenticatoin", 0),
            entry("Prefix second", "authenticationflow", 0),
            entry("Exact last", "authentication", 0),
        ];
        let mut index = PaletteSearch::default();
        index.update(&entries);
        assert_eq!(
            index.ranked(&entries, PaletteCategory::All, "authentication"),
            vec![1, 2, 0]
        );
    }
    #[test]
    fn incremental_updates_keep_rows_and_support_mixed_terms_and_snippets() {
        let mut entries = vec![entry("Authentication", "callback implementation", 0)];
        let mut index = PaletteSearch::default();
        index.update(&entries);
        assert!(
            index.connection.is_some(),
            "FTS5 initialization must succeed"
        );
        let rowid = index.documents["Authentication"].0;
        assert_eq!(
            index.ranked(&entries, PaletteCategory::All, "authentication callback"),
            vec![0]
        );
        assert!(index.snippets()["Authentication"].contains("callback"));
        entries[0].search_text = "replacement".into();
        index.update(&entries);
        assert_eq!(index.documents["Authentication"].0, rowid);
        assert!(
            index
                .ranked(&entries, PaletteCategory::All, "callback")
                .is_empty()
        );
        assert!(
            index
                .connection
                .as_ref()
                .unwrap()
                .prepare("SELECT rowid FROM content WHERE content MATCH ?1")
                .unwrap()
                .query_map(["(broken"], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .is_err()
        );
        assert!(
            index
                .ranked(&entries, PaletteCategory::All, "(replacement")
                .contains(&0)
        );
    }
    #[test]
    fn typo_unicode_safe_queries_and_stale_removal() {
        let entries = vec![entry("Document", "authentication café 東京", 0)];
        let mut index = PaletteSearch::default();
        index.update(&entries);
        for query in [
            "authentcation",
            "authenticatoin",
            "CAFÉ",
            "東京",
            "\"authentication\" OR (",
        ] {
            assert_eq!(
                index.ranked(&entries, PaletteCategory::All, query).len(),
                if query.contains("OR") { 0 } else { 1 },
                "{query}"
            );
        }
        assert!(
            index
                .ranked(&entries, PaletteCategory::Commands, "café")
                .is_empty()
        );
        assert!(
            index
                .ranked(&entries, PaletteCategory::All, "atei")
                .is_empty()
        );
        index.update(&[entry("Replacement", "unrelated", 0)]);
        assert!(
            index
                .ranked(&entries, PaletteCategory::All, "authentication")
                .is_empty()
        );
    }
}
