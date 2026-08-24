use std::{borrow::Borrow, collections::HashMap, fmt, sync::Arc};

const MAX_CONTEXTS: usize = u64::BITS as usize;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ContextName(Arc<str>);

impl Borrow<str> for ContextName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ContextKey(u8);

impl ContextKey {
    const fn mask(self) -> u64 {
        1_u64 << self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContextSet(u64);

impl ContextSet {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn insert(&mut self, key: ContextKey) {
        self.0 |= key.mask();
    }

    pub fn remove(&mut self, key: ContextKey) {
        self.0 &= !key.mask();
    }

    pub const fn contains(self, key: ContextKey) -> bool {
        self.0 & key.mask() != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextPredicate {
    required: ContextSet,
    forbidden: ContextSet,
}

impl ContextPredicate {
    pub const ALWAYS: Self = Self {
        required: ContextSet::empty(),
        forbidden: ContextSet::empty(),
    };

    pub const fn new(required: ContextSet, forbidden: ContextSet) -> Self {
        Self {
            required,
            forbidden,
        }
    }

    pub const fn matches(self, active: ContextSet) -> bool {
        active.0 & self.required.0 == self.required.0 && active.0 & self.forbidden.0 == 0
    }
}

#[derive(Default)]
pub struct ContextRegistryBuilder {
    names: HashMap<ContextName, ContextKey>,
}

impl ContextRegistryBuilder {
    pub fn register(&mut self, name: &str) -> Result<ContextKey, ContextBuildError> {
        if !valid_name(name) {
            return Err(ContextBuildError::InvalidName(name.into()));
        }
        if self.names.contains_key(name) {
            return Err(ContextBuildError::DuplicateName(name.into()));
        }
        if self.names.len() == MAX_CONTEXTS {
            return Err(ContextBuildError::CapacityExceeded);
        }
        let key = ContextKey(self.names.len() as u8);
        self.names.insert(ContextName(name.into()), key);
        Ok(key)
    }

    pub fn build(self) -> ContextRegistry {
        ContextRegistry { names: self.names }
    }
}

#[derive(Clone, Debug)]
pub struct ContextRegistry {
    names: HashMap<ContextName, ContextKey>,
}

impl ContextRegistry {
    pub fn key(&self, name: &str) -> Option<ContextKey> {
        self.names.get(name).copied()
    }

    pub fn set<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<ContextSet, ContextBuildError> {
        let mut set = ContextSet::empty();
        for name in names {
            let key = self
                .key(name)
                .ok_or_else(|| ContextBuildError::UnknownName(name.into()))?;
            set.insert(key);
        }
        Ok(set)
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextBuildError {
    InvalidName(Arc<str>),
    DuplicateName(Arc<str>),
    UnknownName(Arc<str>),
    CapacityExceeded,
}

impl fmt::Display for ContextBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_predicate_is_a_required_and_forbidden_mask_check() {
        let mut builder = ContextRegistryBuilder::default();
        let preview = builder.register("preview").unwrap();
        let prompt = builder.register("prompt").unwrap();
        let registry = builder.build();
        let required = registry.set(["preview"]).unwrap();
        let forbidden = registry.set(["prompt"]).unwrap();
        let predicate = ContextPredicate::new(required, forbidden);
        assert!(predicate.matches(required));
        assert!(!predicate.matches(required.union(forbidden)));
        assert!(required.contains(preview));
        assert!(!required.contains(prompt));
    }

    #[test]
    fn registry_rejects_unknown_and_duplicate_names() {
        let mut builder = ContextRegistryBuilder::default();
        builder.register("workspace").unwrap();
        assert!(matches!(
            builder.register("workspace"),
            Err(ContextBuildError::DuplicateName(_))
        ));
        let registry = builder.build();
        assert!(matches!(
            registry.set(["editor"]),
            Err(ContextBuildError::UnknownName(_))
        ));
    }
}
