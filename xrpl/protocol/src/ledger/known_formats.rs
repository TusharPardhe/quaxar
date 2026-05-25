//! Generic immutable format registry ported from `KnownFormats.h`.

use std::collections::BTreeMap;

use crate::so_template::{SOTemplate, TemplateError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownFormatItem<K, M = ()> {
    name: &'static str,
    type_: K,
    so_template: SOTemplate,
    metadata: M,
}

impl<K: Copy, M> KnownFormatItem<K, M> {
    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn format_type(&self) -> K {
        self.type_
    }

    pub const fn so_template(&self) -> &SOTemplate {
        &self.so_template
    }

    pub const fn metadata(&self) -> &M {
        &self.metadata
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnownFormatsError {
    DuplicateType {
        registry: &'static str,
        name: &'static str,
        existing_name: &'static str,
    },
    DuplicateName {
        registry: &'static str,
        name: &'static str,
    },
    UnknownFormatName {
        registry: &'static str,
        name: String,
    },
    Template(TemplateError),
}

impl From<TemplateError> for KnownFormatsError {
    fn from(value: TemplateError) -> Self {
        Self::Template(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownFormats<K, M = ()> {
    registry_name: &'static str,
    items: Vec<KnownFormatItem<K, M>>,
    names: BTreeMap<&'static str, usize>,
    types: BTreeMap<K, usize>,
}

impl<K: Copy + Ord, M> KnownFormats<K, M> {
    pub fn new(registry_name: &'static str) -> Self {
        Self {
            registry_name,
            items: Vec::new(),
            names: BTreeMap::new(),
            types: BTreeMap::new(),
        }
    }

    pub fn registry_name(&self) -> &'static str {
        self.registry_name
    }

    pub fn add(
        &mut self,
        name: &'static str,
        type_: K,
        so_template: SOTemplate,
        metadata: M,
    ) -> Result<(), KnownFormatsError> {
        if let Some(existing_index) = self.types.get(&type_) {
            let existing = &self.items[*existing_index];
            return Err(KnownFormatsError::DuplicateType {
                registry: self.registry_name,
                name,
                existing_name: existing.name,
            });
        }
        if self.names.contains_key(name) {
            return Err(KnownFormatsError::DuplicateName {
                registry: self.registry_name,
                name,
            });
        }

        let index = self.items.len();
        self.items.push(KnownFormatItem {
            name,
            type_,
            so_template,
            metadata,
        });
        self.names.insert(name, index);
        self.types.insert(type_, index);
        Ok(())
    }

    pub fn find_type_by_name(&self, name: &str) -> Result<K, KnownFormatsError> {
        self.find_by_name(name)
            .map(|item| item.format_type())
            .ok_or_else(|| KnownFormatsError::UnknownFormatName {
                registry: self.registry_name,
                name: name.chars().take(32).collect(),
            })
    }

    pub fn find_by_type(&self, type_: K) -> Option<&KnownFormatItem<K, M>> {
        self.types.get(&type_).map(|index| &self.items[*index])
    }

    pub fn find_by_name(&self, name: &str) -> Option<&KnownFormatItem<K, M>> {
        self.names.get(name).map(|index| &self.items[*index])
    }

    pub fn iter(&self) -> impl Iterator<Item = &KnownFormatItem<K, M>> {
        self.items.iter()
    }
}
