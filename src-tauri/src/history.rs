//! Persistent ordered history collections for high-churn, append-heavy state.
//!
//! `History` keeps the serialized representation as a JSON array while sharing
//! its vector and lookup indexes across snapshots. Mutations made through the
//! tracked APIs carry a bounded delta log. APIs that can shift many indices
//! deliberately force a full-store fallback.

use im::{HashMap as PersistentHashMap, Vector};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::BTreeSet,
    iter::FromIterator,
    ops::{Deref, DerefMut, Index, IndexMut},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

const MAX_TRACKED_CHANGES: usize = 4096;
static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

/// Records that can be indexed by stable identity and grouped by scope.
pub trait HistoryRecord: Clone {
    fn id(&self) -> &str;

    fn scope_id(&self) -> Option<&str> {
        None
    }
}

impl<T: HistoryRecord + ?Sized> HistoryRecord for Arc<T> {
    fn id(&self) -> &str {
        (**self).id()
    }

    fn scope_id(&self) -> Option<&str> {
        (**self).scope_id()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryLookupError {
    DuplicateId(String),
}

impl std::fmt::Display for HistoryLookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(f, "history contains duplicate record id '{id}'"),
        }
    }
}

impl std::error::Error for HistoryLookupError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryChange {
    Append { index: usize },
    Update { index: usize },
    TruncateFrom { new_len: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryDelta {
    Incremental {
        from_version: u64,
        to_version: u64,
        changes: Vec<HistoryChange>,
    },
    Full,
}

/// A persistent, ordered collection with identity and scope indexes.
pub struct History<T: HistoryRecord> {
    rows: Vector<T>,
    ids: Mutex<PersistentHashMap<String, Vector<usize>>>,
    scopes: Mutex<PersistentHashMap<String, Vector<usize>>>,
    indexes_dirty: AtomicBool,
    lineage: Arc<HistoryLineage>,
    version: u64,
    base_version: u64,
    changes: Vec<HistoryChange>,
    full_change: bool,
}

#[derive(Debug)]
struct HistoryLineage;

impl<T: HistoryRecord + Clone> Clone for History<T> {
    fn clone(&self) -> Self {
        // Persistent Vector/HashMap clones only bump shared-node references.
        // A new clone token intentionally starts a fresh delta log.
        Self {
            rows: self.rows.clone(),
            ids: Mutex::new(self.ids.lock().expect("history id index poisoned").clone()),
            scopes: Mutex::new(
                self.scopes
                    .lock()
                    .expect("history scope index poisoned")
                    .clone(),
            ),
            indexes_dirty: AtomicBool::new(self.indexes_dirty.load(Ordering::Acquire)),
            lineage: self.lineage.clone(),
            version: self.version,
            base_version: self.version,
            changes: Vec::new(),
            full_change: false,
        }
    }
}

impl<T: HistoryRecord> History<T> {
    pub fn new() -> Self {
        Self::from_vec(Vec::new())
    }

    pub fn from_vec(rows: Vec<T>) -> Self {
        let rows = Vector::from(rows);
        let (ids, scopes) = build_indexes(&rows);
        let version = next_version();
        Self {
            rows,
            ids: Mutex::new(ids),
            scopes: Mutex::new(scopes),
            indexes_dirty: AtomicBool::new(false),
            lineage: Arc::new(HistoryLineage),
            version,
            base_version: version,
            changes: Vec::new(),
            full_change: false,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.rows.iter()
    }

    /// Mutable iteration is supported, but may alter any ID or scope. It
    /// therefore invalidates both indexes and forces a full delta fallback.
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> + ExactSizeIterator {
        self.mark_full_and_dirty();
        self.rows.iter_mut()
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.rows.get(index)
    }

    pub fn last(&self) -> Option<&T> {
        self.rows.back()
    }

    pub fn last_mut(&mut self) -> Option<&mut T> {
        self.len()
            .checked_sub(1)
            .and_then(|index| self.get_mut(index))
    }

    /// Raw mutable access marks the touched row and defers index repair until
    /// the next identity/scope lookup. Prefer `get_by_id_mut` for bounded index
    /// maintenance when changing a known record.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len() {
            return None;
        }
        self.indexes_dirty.store(true, Ordering::Release);
        self.record(HistoryChange::Update { index });
        self.rows.get_mut(index)
    }

    pub fn push(&mut self, value: T) {
        let index = self.len();
        let id = value.id().to_owned();
        let scope = value.scope_id().map(str::to_owned);
        self.rows.push_back(value);
        self.insert_index(&id, index);
        if let Some(scope) = scope {
            self.insert_scope(&scope, index);
        }
        self.record(HistoryChange::Append { index });
    }

    pub fn extend<I: IntoIterator<Item = T>>(&mut self, values: I) {
        for value in values {
            self.push(value);
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        let index = self.len().checked_sub(1)?;
        let value = self.rows.back()?;
        let id = value.id().to_owned();
        let scope = value.scope_id().map(str::to_owned);
        self.remove_index(&id, index);
        if let Some(scope) = scope {
            self.remove_scope(&scope, index);
        }
        let removed = self.rows.pop_back();
        self.record(HistoryChange::TruncateFrom { new_len: index });
        removed
    }

    /// Truncation is a suffix operation and remains incrementally representable.
    pub fn truncate(&mut self, new_len: usize) {
        if new_len >= self.len() {
            return;
        }
        for index in (new_len..self.len()).rev() {
            let row = &self.rows[index];
            let id = row.id().to_owned();
            let scope = row.scope_id().map(str::to_owned);
            self.remove_index(&id, index);
            if let Some(scope) = scope {
                self.remove_scope(&scope, index);
            }
        }
        self.rows.truncate(new_len);
        self.record(HistoryChange::TruncateFrom { new_len });
    }

    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Retain can shift an arbitrary suffix of indices; record a full fallback.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&T) -> bool,
    {
        if self.is_empty() {
            return;
        }
        self.mark_full_and_dirty();
        self.rows.retain(|row| keep(row));
    }

    /// Insertion/removal in the middle shifts subsequent indices, so these
    /// Vec-like operations are deliberately represented as a full fallback.
    pub fn insert(&mut self, index: usize, value: T) {
        assert!(index <= self.len(), "insertion index out of bounds");
        self.mark_full_and_dirty();
        self.rows.insert(index, value);
    }

    pub fn remove(&mut self, index: usize) -> T {
        assert!(index < self.len(), "removal index out of bounds");
        self.mark_full_and_dirty();
        self.rows.remove(index)
    }

    /// Returns the unique row index for `id`. Duplicate IDs return a visible
    /// error rather than silently replacing an entry in the index.
    pub fn index_of_id(&self, id: &str) -> Result<Option<usize>, HistoryLookupError> {
        self.ensure_indexes();
        let indexes = self
            .ids
            .lock()
            .expect("history id index poisoned")
            .get(id)
            .cloned();
        match indexes {
            None => Ok(None),
            Some(indexes) if indexes.len() == 1 => Ok(indexes.front().copied()),
            Some(_) => Err(HistoryLookupError::DuplicateId(id.to_owned())),
        }
    }

    pub fn get_by_id(&self, id: &str) -> Result<Option<&T>, HistoryLookupError> {
        self.index_of_id(id)
            .map(|index| index.and_then(|index| self.rows.get(index)))
    }

    /// Mutable access compares old/new ID and scope on guard drop. Same-key
    /// updates retain a precise point delta and avoid rebuilding either index.
    /// If a caller intentionally forgets the guard, dirty indexes force a safe
    /// rebuild on the next lookup, and the point delta was recorded up front.
    pub fn get_by_id_mut(
        &mut self,
        id: &str,
    ) -> Result<Option<HistoryMutGuard<'_, T>>, HistoryLookupError> {
        self.ensure_indexes_mut();
        let index = self.index_of_id(id)?;
        let Some(index) = index else {
            return Ok(None);
        };
        Ok(self.get_mut_tracked(index))
    }

    /// Guarded point mutation by a known row index. It is useful when a store
    /// already resolved a cursor or record ID and should avoid another lookup.
    pub fn get_mut_tracked(&mut self, index: usize) -> Option<HistoryMutGuard<'_, T>> {
        if index >= self.len() {
            return None;
        }
        let old_id = self.rows[index].id().to_owned();
        let old_scope = self.rows[index].scope_id().map(str::to_owned);
        self.indexes_dirty.store(true, Ordering::Release);
        self.record(HistoryChange::Update { index });
        Some(HistoryMutGuard {
            history: self,
            index,
            old_id,
            old_scope,
        })
    }

    pub fn update_at<R, F>(&mut self, index: usize, update: F) -> Option<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut row = self.get_mut_tracked(index)?;
        Some(update(&mut row))
    }

    pub fn update_by_id<R, F>(
        &mut self,
        id: &str,
        update: F,
    ) -> Result<Option<R>, HistoryLookupError>
    where
        F: FnOnce(&mut T) -> R,
    {
        let Some(mut row) = self.get_by_id_mut(id)? else {
            return Ok(None);
        };
        Ok(Some(update(&mut row)))
    }

    /// Clone of the persistent ordered index list; iterate it in reverse for
    /// newest-first scoped paging without scanning other scopes.
    pub fn scope_indices(&self, scope: &str) -> Vector<usize> {
        self.ensure_indexes();
        self.scopes
            .lock()
            .expect("history scope index poisoned")
            .get(scope)
            .cloned()
            .unwrap_or_default()
    }

    pub fn duplicate_ids(&self) -> Vec<String> {
        self.ensure_indexes();
        self.ids
            .lock()
            .expect("history id index poisoned")
            .iter()
            .filter_map(|(id, indexes)| (indexes.len() > 1).then(|| id.clone()))
            .collect()
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// Incremental changes are available only when `self` is a direct clone
    /// of `before` followed by tracked operations. Any other lineage or an
    /// unsupported mutation requests a full-row fallback.
    pub fn delta_since(&self, before: &Self) -> HistoryDelta {
        if !Arc::ptr_eq(&self.lineage, &before.lineage) {
            return HistoryDelta::Full;
        }
        if self.version == before.version && self.rows.ptr_eq(&before.rows) {
            return HistoryDelta::Incremental {
                from_version: before.version,
                to_version: self.version,
                changes: Vec::new(),
            };
        }
        if self.base_version != before.version || self.full_change {
            return HistoryDelta::Full;
        }
        HistoryDelta::Incremental {
            from_version: before.version,
            to_version: self.version,
            changes: self.changes.clone(),
        }
    }

    fn insert_index(&self, id: &str, index: usize) {
        if self.indexes_dirty.load(Ordering::Acquire) {
            return;
        }
        let mut ids = self.ids.lock().expect("history id index poisoned");
        let list = ids.entry(id.to_owned()).or_default();
        list.push_back(index);
    }

    fn remove_index(&self, id: &str, index: usize) {
        if self.indexes_dirty.load(Ordering::Acquire) {
            return;
        }
        let mut ids = self.ids.lock().expect("history id index poisoned");
        let empty = if let Some(list) = ids.get_mut(id) {
            if list.back().copied() == Some(index) {
                list.pop_back();
            } else {
                list.retain(|candidate| *candidate != index);
            }
            list.is_empty()
        } else {
            false
        };
        if empty {
            ids.remove(id);
        }
    }

    fn insert_scope(&self, scope: &str, index: usize) {
        if self.indexes_dirty.load(Ordering::Acquire) {
            return;
        }
        self.scopes
            .lock()
            .expect("history scope index poisoned")
            .entry(scope.to_owned())
            .or_default()
            .push_back(index);
    }

    fn remove_scope(&self, scope: &str, index: usize) {
        if self.indexes_dirty.load(Ordering::Acquire) {
            return;
        }
        let mut scopes = self.scopes.lock().expect("history scope index poisoned");
        let empty = if let Some(list) = scopes.get_mut(scope) {
            if list.back().copied() == Some(index) {
                list.pop_back();
            } else {
                list.retain(|candidate| *candidate != index);
            }
            list.is_empty()
        } else {
            false
        };
        if empty {
            scopes.remove(scope);
        }
    }

    fn ensure_indexes_mut(&mut self) {
        if !self.indexes_dirty.load(Ordering::Acquire) {
            return;
        }
        let (ids, scopes) = build_indexes(&self.rows);
        *self.ids.lock().expect("history id index poisoned") = ids;
        *self.scopes.lock().expect("history scope index poisoned") = scopes;
        self.indexes_dirty.store(false, Ordering::Release);
    }

    fn ensure_indexes(&self) {
        if !self.indexes_dirty.load(Ordering::Acquire) {
            return;
        }
        // Shared readers may race to repair an index, so recheck while holding
        // both index locks. History rows themselves are immutable under `&self`.
        let mut ids = self.ids.lock().expect("history id index poisoned");
        let mut scopes = self.scopes.lock().expect("history scope index poisoned");
        if !self.indexes_dirty.load(Ordering::Acquire) {
            return;
        }
        let (rebuilt_ids, rebuilt_scopes) = build_indexes(&self.rows);
        *ids = rebuilt_ids;
        *scopes = rebuilt_scopes;
        // This is safe because any mutable borrow of this History has ended;
        // the flag is only atomic to permit lazy repair from shared readers.
        self.set_indexes_clean();
    }

    fn set_indexes_clean(&self) {
        // `indexes_dirty` is stored in an AtomicBool in order for `get_by_id`
        // and scoped queries to repair indexes through a shared reference.
        self.indexes_dirty.store(false, Ordering::Release);
    }

    fn record(&mut self, change: HistoryChange) {
        self.version = next_version();
        if self.full_change {
            return;
        }
        if self.changes.len() >= MAX_TRACKED_CHANGES {
            self.changes.clear();
            self.full_change = true;
        } else {
            self.changes.push(change);
        }
    }

    fn mark_full_and_dirty(&mut self) {
        self.version = next_version();
        self.indexes_dirty.store(true, Ordering::Release);
        self.changes.clear();
        self.full_change = true;
    }
}

/// Guard that keeps a point update bounded while detecting changes to the
/// record's identity or scope.
pub struct HistoryMutGuard<'a, T: HistoryRecord> {
    history: &'a mut History<T>,
    index: usize,
    old_id: String,
    old_scope: Option<String>,
}

impl<T: HistoryRecord> Deref for HistoryMutGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.history.rows[self.index]
    }
}

impl<T: HistoryRecord> DerefMut for HistoryMutGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.history
            .rows
            .get_mut(self.index)
            .expect("guard index remains valid")
    }
}

impl<T: HistoryRecord> Drop for HistoryMutGuard<'_, T> {
    fn drop(&mut self) {
        let new_id = self.history.rows[self.index].id().to_owned();
        let new_scope = self.history.rows[self.index].scope_id().map(str::to_owned);
        if self.old_scope == new_scope {
            // Restore the clean state before the local ID-index edit. If this
            // guard was forgotten, Drop never runs and lazy repair sees dirty.
            self.history.indexes_dirty.store(false, Ordering::Release);
        }
        if self.old_id != new_id {
            self.history.remove_index(&self.old_id, self.index);
            self.history.insert_index(&new_id, self.index);
        }
        if self.old_scope != new_scope {
            if let Some(scope) = &self.old_scope {
                self.history.remove_scope(scope, self.index);
            }
            if let Some(scope) = &new_scope {
                self.history.insert_scope(scope, self.index);
            }
        }
        if self.old_scope != new_scope {
            self.history.changes.clear();
            self.history.full_change = true;
            // A scope move is rare and may invalidate a long ordered scope
            // index. Keep the row delta safe, but rebuild indexes on demand.
        }
    }
}

impl<T: HistoryRecord> Index<usize> for History<T> {
    type Output = T;
    fn index(&self, index: usize) -> &Self::Output {
        &self.rows[index]
    }
}

impl<T: HistoryRecord> IndexMut<usize> for History<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        assert!(index < self.len(), "history index out of bounds");
        self.indexes_dirty.store(true, Ordering::Release);
        self.record(HistoryChange::Update { index });
        self.rows
            .get_mut(index)
            .expect("index was checked against history length")
    }
}

impl<'a, T: HistoryRecord> IntoIterator for &'a History<T> {
    type Item = &'a T;
    type IntoIter = im::vector::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.rows.iter()
    }
}

impl<'a, T: HistoryRecord> IntoIterator for &'a mut History<T> {
    type Item = &'a mut T;
    type IntoIter = im::vector::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.mark_full_and_dirty();
        self.rows.iter_mut()
    }
}

impl<T: HistoryRecord> Extend<T> for History<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        History::extend(self, iter);
    }
}

impl<T: HistoryRecord + std::fmt::Debug> std::fmt::Debug for History<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: HistoryRecord> Default for History<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: HistoryRecord> From<Vec<T>> for History<T> {
    fn from(value: Vec<T>) -> Self {
        Self::from_vec(value)
    }
}

impl<T: HistoryRecord> FromIterator<T> for History<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from_vec(iter.into_iter().collect())
    }
}

impl<T: HistoryRecord + PartialEq> PartialEq for History<T> {
    fn eq(&self, other: &Self) -> bool {
        if self.rows.ptr_eq(&other.rows) {
            return true;
        }
        if self.rows.len() != other.rows.len() {
            return false;
        }
        if Arc::ptr_eq(&self.lineage, &other.lineage) {
            if self.version == other.version {
                return true;
            }
            if self.base_version == other.version && !self.full_change {
                return related_rows_equal(&self.rows, &other.rows, &self.changes);
            }
            if other.base_version == self.version && !other.full_change {
                return related_rows_equal(&other.rows, &self.rows, &other.changes);
            }
        }
        self.rows == other.rows
    }
}

impl<T: HistoryRecord + Eq> Eq for History<T> {}

impl<T: HistoryRecord + Serialize> Serialize for History<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.rows.serialize(serializer)
    }
}

impl<'de, T: HistoryRecord + Deserialize<'de>> Deserialize<'de> for History<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<T>::deserialize(deserializer).map(Self::from_vec)
    }
}

fn build_indexes<T: HistoryRecord>(
    rows: &Vector<T>,
) -> (
    PersistentHashMap<String, Vector<usize>>,
    PersistentHashMap<String, Vector<usize>>,
) {
    let mut ids = PersistentHashMap::new();
    let mut scopes = PersistentHashMap::new();
    for (index, row) in rows.iter().enumerate() {
        ids.entry(row.id().to_owned())
            .or_insert_with(Vector::new)
            .push_back(index);
        if let Some(scope) = row.scope_id() {
            scopes
                .entry(scope.to_owned())
                .or_insert_with(Vector::new)
                .push_back(index);
        }
    }
    (ids, scopes)
}

fn related_rows_equal<T: PartialEq + Clone>(
    changed: &Vector<T>,
    base: &Vector<T>,
    changes: &[HistoryChange],
) -> bool {
    let mut indices = BTreeSet::new();
    for change in changes {
        match *change {
            HistoryChange::Append { index } | HistoryChange::Update { index } => {
                indices.insert(index);
            }
            HistoryChange::TruncateFrom { new_len } => {
                indices.extend(new_len..changed.len());
            }
        }
    }
    indices
        .into_iter()
        .all(|index| changed.get(index) == base.get(index))
}

fn next_version() -> u64 {
    NEXT_VERSION.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::sync::atomic::AtomicUsize;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Row {
        id: String,
        scope: String,
        value: usize,
    }

    impl HistoryRecord for Row {
        fn id(&self) -> &str {
            &self.id
        }

        fn scope_id(&self) -> Option<&str> {
            Some(&self.scope)
        }
    }

    fn row(id: usize, scope: usize, value: usize) -> Row {
        Row {
            id: format!("id-{id}"),
            scope: format!("scope-{scope}"),
            value,
        }
    }

    fn rows(history: &History<Row>) -> Vec<Row> {
        history.iter().cloned().collect()
    }

    fn assert_indexes_match(history: &History<Row>, expected: &[Row]) {
        for (index, row) in expected.iter().enumerate() {
            assert_eq!(history.index_of_id(&row.id).unwrap(), Some(index));
            let scoped = history.scope_indices(&row.scope);
            let expected_scoped: Vec<_> = expected
                .iter()
                .enumerate()
                .filter_map(|(i, candidate)| (candidate.scope == row.scope).then_some(i))
                .collect();
            assert_eq!(scoped.iter().copied().collect::<Vec<_>>(), expected_scoped);
        }
    }

    #[test]
    fn clones_are_immutable_and_tracked_row_deltas_are_precise() {
        let before = History::from_vec((0..8).map(|n| row(n, n % 2, n)).collect());
        let mut candidate = before.clone();
        candidate
            .update_by_id("id-3", |record| record.value = 300)
            .unwrap();

        assert_eq!(before[3].value, 3);
        assert_eq!(candidate[3].value, 300);
        assert_eq!(
            candidate.delta_since(&before),
            HistoryDelta::Incremental {
                from_version: before.version(),
                to_version: candidate.version(),
                changes: vec![HistoryChange::Update { index: 3 }],
            }
        );
        assert_ne!(candidate, before);

        let clone_after_update = candidate.clone();
        assert_eq!(clone_after_update, candidate);
        assert_eq!(
            clone_after_update.delta_since(&candidate),
            HistoryDelta::Incremental {
                from_version: candidate.version(),
                to_version: candidate.version(),
                changes: vec![],
            }
        );
        assert_eq!(clone_after_update.delta_since(&before), HistoryDelta::Full);
    }

    #[test]
    fn ids_scopes_and_duplicate_detection_survive_mutation() {
        let mut history = History::from_vec(vec![row(1, 7, 0), row(2, 7, 0), row(3, 8, 0)]);
        let before = history.clone();
        history
            .update_by_id("id-2", |record| record.value += 1)
            .unwrap();
        assert_eq!(
            history
                .scope_indices("scope-7")
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert!(matches!(
            history.delta_since(&before),
            HistoryDelta::Incremental { .. }
        ));

        history
            .update_by_id("id-2", |record| record.id = "moved-id".into())
            .unwrap();
        assert_eq!(history.index_of_id("id-2").unwrap(), None);
        assert_eq!(history.index_of_id("moved-id").unwrap(), Some(1));

        let before_scope_move = history.clone();
        history
            .update_by_id("moved-id", |record| record.scope = "scope-9".into())
            .unwrap();
        assert_eq!(
            history
                .scope_indices("scope-7")
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(
            history
                .scope_indices("scope-9")
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(history.delta_since(&before_scope_move), HistoryDelta::Full);

        history.push(row(4, 9, 0));
        history.push(row(4, 9, 1));
        assert_eq!(history.duplicate_ids(), vec!["id-4".to_owned()]);
        assert!(matches!(
            history.index_of_id("id-4"),
            Err(HistoryLookupError::DuplicateId(_))
        ));
    }

    #[test]
    fn raw_mutable_apis_rebuild_indexes_and_forget_guard_is_safe() {
        let mut history = History::from_vec(vec![row(1, 1, 0), row(2, 1, 0)]);
        let before = history.clone();
        history.get_mut(0).unwrap().id = "raw-id".into();
        assert_eq!(history.index_of_id("id-1").unwrap(), None);
        assert_eq!(history.index_of_id("raw-id").unwrap(), Some(0));
        assert_eq!(
            history.delta_since(&before),
            HistoryDelta::Incremental {
                from_version: before.version(),
                to_version: history.version(),
                changes: vec![HistoryChange::Update { index: 0 }],
            }
        );

        let before_forget = history.clone();
        let mut forgotten_candidate = before_forget.clone();
        let mut guard = forgotten_candidate
            .get_by_id_mut("raw-id")
            .unwrap()
            .unwrap();
        guard.id = "forgotten-id".into();
        std::mem::forget(guard);
        assert_eq!(
            forgotten_candidate.index_of_id("forgotten-id").unwrap(),
            Some(0)
        );
        assert!(matches!(
            forgotten_candidate.delta_since(&before_forget),
            HistoryDelta::Incremental { .. }
        ));

        let before_iteration = history.clone();
        for record in &mut history {
            record.value += 1;
        }
        assert_eq!(history.delta_since(&before_iteration), HistoryDelta::Full);
        assert_eq!(
            history
                .iter()
                .map(|record| record.value)
                .collect::<Vec<_>>(),
            [1, 1]
        );
    }

    #[test]
    fn suffix_changes_are_incremental_and_index_shifts_fall_back() {
        let before = History::from_vec((0..5).map(|n| row(n, 0, n)).collect());
        let mut candidate = before.clone();
        candidate.extend([row(5, 0, 5), row(6, 0, 6)]);
        candidate.truncate(6);
        assert_eq!(
            candidate.delta_since(&before),
            HistoryDelta::Incremental {
                from_version: before.version(),
                to_version: candidate.version(),
                changes: vec![
                    HistoryChange::Append { index: 5 },
                    HistoryChange::Append { index: 6 },
                    HistoryChange::TruncateFrom { new_len: 6 },
                ],
            }
        );
        assert_indexes_match(&candidate, &rows(&candidate));

        let mut shifted = candidate.clone();
        shifted.insert(1, row(10, 1, 10));
        assert_eq!(shifted.delta_since(&candidate), HistoryDelta::Full);
        shifted.remove(1);
        assert_indexes_match(&shifted, &rows(&candidate));
        assert_eq!(shifted, candidate);
    }

    #[test]
    fn serde_keeps_the_json_array_shape() {
        let history = History::from_vec(vec![row(1, 2, 3), row(4, 5, 6)]);
        let encoded = serde_json::to_value(&history).unwrap();
        assert!(encoded.is_array());
        let decoded: History<Row> = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, history);
        assert_indexes_match(&decoded, &rows(&history));
    }

    #[test]
    fn deterministic_random_operations_match_vec_and_scope_indexes() {
        let mut history = History::new();
        let mut expected = Vec::new();
        let mut seed = 0x5eed_u64;
        let mut next_id = 0;
        for step in 0..1_200 {
            let before_history = history.clone();
            let before_rows = expected.clone();
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            match (seed >> 32) % 5 {
                0 | 1 => {
                    let item = row(next_id, next_id % 7, step);
                    next_id += 1;
                    history.push(item.clone());
                    expected.push(item);
                }
                2 if !expected.is_empty() => {
                    let index = (seed as usize) % expected.len();
                    let id = expected[index].id.clone();
                    history
                        .update_by_id(&id, |item| item.value = item.value.wrapping_add(1))
                        .unwrap();
                    expected[index].value = expected[index].value.wrapping_add(1);
                }
                3 if !expected.is_empty() => {
                    let new_len = (seed as usize) % (expected.len() + 1);
                    history.truncate(new_len);
                    expected.truncate(new_len);
                }
                4 if !expected.is_empty() => {
                    let keep_below = (seed as usize) % (expected.len() + 1);
                    history.retain(|record| record.value < keep_below);
                    expected.retain(|record| record.value < keep_below);
                }
                _ => {}
            }
            assert_eq!(rows(&history), expected);
            assert_indexes_match(&history, &expected);
            if let HistoryDelta::Incremental { changes, .. } = history.delta_since(&before_history)
            {
                let mut replay = before_rows;
                for change in changes {
                    match change {
                        HistoryChange::Append { index } => replay.push(history[index].clone()),
                        HistoryChange::Update { index } => replay[index] = history[index].clone(),
                        HistoryChange::TruncateFrom { new_len } => replay.truncate(new_len),
                    }
                }
                assert_eq!(replay, expected, "delta replay mismatch at step {step}");
            }
        }
    }

    struct CloneProbe {
        id: String,
        clones: Arc<AtomicUsize>,
    }

    impl Clone for CloneProbe {
        fn clone(&self) -> Self {
            self.clones.fetch_add(1, Ordering::Relaxed);
            Self {
                id: self.id.clone(),
                clones: self.clones.clone(),
            }
        }
    }

    impl HistoryRecord for CloneProbe {
        fn id(&self) -> &str {
            &self.id
        }
    }

    #[test]
    fn cloning_histories_at_1k_and_100k_rows_does_not_clone_records() {
        for size in [1_000, 100_000] {
            let clones = Arc::new(AtomicUsize::new(0));
            let history = History::from_vec(
                (0..size)
                    .map(|index| CloneProbe {
                        id: format!("row-{index}"),
                        clones: clones.clone(),
                    })
                    .collect(),
            );
            let snapshot = history.clone();
            assert_eq!(clones.load(Ordering::Relaxed), 0, "size={size}");
            assert_eq!(snapshot.len(), size);

            let mut edited = snapshot.clone();
            edited
                .update_by_id("row-500", |record| record.id.push_str("-updated"))
                .unwrap();
            assert!(clones.load(Ordering::Relaxed) < 256, "size={size}");
            assert_eq!(history.index_of_id("row-500").unwrap(), Some(500));
            assert_eq!(edited.index_of_id("row-500-updated").unwrap(), Some(500));
        }
    }
}
