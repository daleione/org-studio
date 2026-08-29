use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use ::typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Dict, Str, Value},
    syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot},
    text::{Font, FontBook},
    utils::LazyHash,
};

pub(super) struct SharedResources {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main_id: FileId,
}

impl SharedResources {
    pub(super) fn new(custom_font_data: &[Vec<u8>]) -> Self {
        let mut fonts = Vec::with_capacity(20 + custom_font_data.len() * 4);
        for data in typst_assets::fonts() {
            add_faces(Bytes::new(data), &mut fonts);
        }
        for data in custom_font_data {
            add_faces(Bytes::new(data.clone()), &mut fonts);
        }
        let mut book = FontBook::new();
        for font in &fonts {
            book.push(font.info().clone());
        }
        Self {
            library: LazyHash::new(Library::builder().build()),
            book: LazyHash::new(book),
            fonts,
            main_id: FileId::new(RootedPath::new(
                VirtualRoot::Project,
                VirtualPath::new("/main.typ").expect("valid main path"),
            )),
        }
    }

    pub(super) fn family_names(&self) -> Vec<String> {
        let mut names = std::collections::BTreeSet::new();
        for font in &self.fonts {
            names.insert(font.info().family.to_string());
        }
        names.into_iter().collect()
    }
}

fn add_faces(bytes: Bytes, fonts: &mut Vec<Font>) {
    for index in 0.. {
        match Font::new(bytes.clone(), index) {
            Some(font) => fonts.push(font),
            None => break,
        }
    }
}

pub(super) struct OrgStudioWorld<'a> {
    shared: &'a SharedResources,
    source: Source,
    root: Option<PathBuf>,
    canonical_root: Option<PathBuf>,
    library_override: Option<LazyHash<Library>>,
}

impl<'a> OrgStudioWorld<'a> {
    pub(super) fn new(
        shared: &'a SharedResources,
        source_text: String,
        root: Option<&Path>,
        inputs: &BTreeMap<String, String>,
    ) -> Self {
        let root = root.map(Path::to_path_buf);
        let canonical_root = root.as_ref().and_then(|root| root.canonicalize().ok());
        let library_override = if inputs.is_empty() {
            None
        } else {
            let mut dict = Dict::new();
            for (key, value) in inputs {
                dict.insert(
                    Str::from(key.as_str()),
                    Value::Str(Str::from(value.as_str())),
                );
            }
            Some(LazyHash::new(Library::builder().with_inputs(dict).build()))
        };
        Self {
            shared,
            source: Source::new(shared.main_id, source_text),
            root,
            canonical_root,
            library_override,
        }
    }

    fn resolve_path(&self, id: FileId) -> FileResult<PathBuf> {
        if !matches!(id.get().root(), VirtualRoot::Project) {
            return Err(FileError::AccessDenied);
        }
        let relative = id.get().vpath().get_without_slash();
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| FileError::NotFound(relative.into()))?;
        let canonical_root = self
            .canonical_root
            .as_ref()
            .ok_or_else(|| FileError::NotFound(relative.into()))?;
        let full = root.join(relative);
        let canonical = full
            .canonicalize()
            .map_err(|_| FileError::NotFound(relative.into()))?;
        if !canonical.starts_with(canonical_root) {
            return Err(FileError::AccessDenied);
        }
        Ok(canonical)
    }
}

impl World for OrgStudioWorld<'_> {
    fn library(&self) -> &LazyHash<Library> {
        self.library_override
            .as_ref()
            .unwrap_or(&self.shared.library)
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.shared.book
    }

    fn main(&self) -> FileId {
        self.source.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.source.id() {
            Ok(self.source.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let path = self.resolve_path(id)?;
        let metadata = std::fs::metadata(&path)
            .map_err(|_| FileError::NotFound(id.get().vpath().get_without_slash().into()))?;
        if metadata.len() > 100 * 1024 * 1024 {
            return Err(FileError::AccessDenied);
        }
        let data = std::fs::read(&path)
            .map_err(|_| FileError::NotFound(id.get().vpath().get_without_slash().into()))?;
        Ok(Bytes::new(data))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.shared.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<::typst::foundations::Duration>) -> Option<Datetime> {
        None
    }
}
