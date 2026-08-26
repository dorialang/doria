use std::collections::{BTreeMap, HashMap};

use crate::names::{PackageIdentity, SourceIdentity};
use crate::source::{SourceFile, SourceId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecord {
    pub identity: SourceIdentity,
    pub package: PackageIdentity,
    pub display_path: String,
    pub canonical_path: Option<String>,
    pub content_fingerprint: String,
    pub source: SourceFile,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMap {
    by_identity: BTreeMap<String, SourceRecord>,
    identity_by_id: HashMap<SourceId, String>,
    identity_by_path: HashMap<String, String>,
}

impl SourceMap {
    pub fn from_ordered_records(records: Vec<SourceRecord>) -> Self {
        let mut source_map = Self::default();
        for record in records {
            source_map.insert(record);
        }
        source_map
    }

    pub fn insert(&mut self, record: SourceRecord) {
        self.identity_by_id
            .insert(record.source.id, record.identity.0.clone());
        self.identity_by_path
            .insert(record.display_path.clone(), record.identity.0.clone());
        self.by_identity.insert(record.identity.0.clone(), record);
    }

    pub fn get(&self, identity: &SourceIdentity) -> Option<&SourceRecord> {
        self.by_identity.get(&identity.0)
    }

    pub fn by_id(&self, id: SourceId) -> Option<&SourceRecord> {
        self.identity_by_id
            .get(&id)
            .and_then(|identity| self.by_identity.get(identity))
    }

    pub fn by_path(&self, path: &str) -> Option<&SourceRecord> {
        self.identity_by_path
            .get(path)
            .and_then(|identity| self.by_identity.get(identity))
    }

    pub fn iter(&self) -> impl Iterator<Item = &SourceRecord> {
        self.by_identity.values()
    }

    pub fn len(&self) -> usize {
        self.by_identity.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_identity.is_empty()
    }
}
