use crate::DocumentId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    path::{Path, PathBuf},
};

const MAX_RECENT_FILES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Project {
    pub id: ProjectId,
    pub root: PathBuf,
    pub name: String,
    pub alias: Option<String>,
    pub favorite: bool,
    pub manual: bool,
    pub recent_files: Vec<PathBuf>,
}

impl Project {
    pub fn display_name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: ProjectId(1),
            root: PathBuf::new(),
            name: String::new(),
            alias: None,
            favorite: false,
            manual: false,
            recent_files: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct PersistedRegistry {
    active_project: Option<ProjectId>,
    next_id: u64,
    recent_files: Vec<PathBuf>,
    projects: Vec<Project>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRegistry {
    active_project: Option<ProjectId>,
    next_id: u64,
    recent_files: Vec<PathBuf>,
    projects: BTreeMap<ProjectId, Project>,
    projects_by_root: BTreeMap<PathBuf, ProjectId>,
    documents: BTreeMap<DocumentId, ProjectId>,
    project_documents: BTreeMap<ProjectId, VecDeque<DocumentId>>,
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        Self {
            active_project: None,
            next_id: 1,
            recent_files: Vec::new(),
            projects: BTreeMap::new(),
            projects_by_root: BTreeMap::new(),
            documents: BTreeMap::new(),
            project_documents: BTreeMap::new(),
        }
    }
}

impl ProjectRegistry {
    pub fn registry_path() -> PathBuf {
        helix_loader::data_dir().join("projects.toml")
    }

    pub fn load() -> Self {
        Self::load_from_path(Self::registry_path()).unwrap_or_default()
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let persisted = toml::from_str::<PersistedRegistry>(&contents)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                Ok(Self::from_persisted(persisted))
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to_path(Self::registry_path())
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> io::Result<()> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(&self.to_persisted())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        std::fs::write(path, contents)
    }

    pub fn active_project(&self) -> Option<ProjectId> {
        self.active_project
    }

    pub fn set_active(&mut self, id: Option<ProjectId>) {
        self.active_project = id.filter(|id| self.projects.contains_key(id));
    }

    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.values()
    }

    pub fn recent_files(&self) -> &[PathBuf] {
        &self.recent_files
    }

    pub fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.get(&id)
    }

    pub fn project_mut(&mut self, id: ProjectId) -> Option<&mut Project> {
        self.projects.get_mut(&id)
    }

    pub fn project_by_root(&self, root: impl AsRef<Path>) -> Option<&Project> {
        let root = normalize_root(root.as_ref().to_path_buf());
        let id = self.projects_by_root.get(&root)?;
        self.projects.get(id)
    }

    pub fn ensure_project(&mut self, root: impl Into<PathBuf>) -> ProjectId {
        let root = normalize_root(root.into());
        if let Some(id) = self.projects_by_root.get(&root) {
            return *id;
        }

        let id = ProjectId(self.next_id.max(1));
        self.next_id = id.0 + 1;
        let name = project_name_from_root(&root);
        let project = Project {
            id,
            root: root.clone(),
            name,
            alias: None,
            favorite: false,
            manual: false,
            recent_files: Vec::new(),
        };

        self.projects.insert(id, project);
        self.projects_by_root.insert(root, id);
        id
    }

    pub fn project_for_path(&self, path: impl AsRef<Path>) -> Option<ProjectId> {
        let path = path.as_ref();
        self.projects_by_root
            .iter()
            .filter(|(root, _)| path.starts_with(root))
            .max_by_key(|(root, _)| root.components().count())
            .map(|(_, id)| *id)
    }

    pub fn assign_document(&mut self, doc_id: DocumentId, project: ProjectId) {
        if !self.projects.contains_key(&project) {
            return;
        }

        if let Some(old_project) = self.documents.insert(doc_id, project) {
            if old_project != project {
                if let Some(docs) = self.project_documents.get_mut(&old_project) {
                    docs.retain(|id| *id != doc_id);
                }
            }
        }
        self.record_project_focus(project, doc_id);
    }

    pub fn remove_document(&mut self, doc_id: DocumentId) {
        if let Some(project) = self.documents.remove(&doc_id) {
            if let Some(docs) = self.project_documents.get_mut(&project) {
                docs.retain(|id| *id != doc_id);
            }
            if self
                .project(project)
                .is_some_and(|project| project.recent_files.is_empty())
            {
                self.project_documents.remove(&project);
            }
        }
    }

    pub fn project_for_document(&self, doc_id: DocumentId) -> Option<ProjectId> {
        self.documents.get(&doc_id).copied()
    }

    pub fn record_project_focus(&mut self, project: ProjectId, doc_id: DocumentId) {
        if !self.projects.contains_key(&project) {
            return;
        }

        let docs = self.project_documents.entry(project).or_default();
        docs.retain(|id| *id != doc_id);
        docs.push_front(doc_id);
    }

    pub fn documents_for_project(
        &self,
        project: ProjectId,
    ) -> impl Iterator<Item = DocumentId> + '_ {
        self.project_documents
            .get(&project)
            .into_iter()
            .flat_map(|docs| docs.iter().copied())
    }

    pub fn record_recent_file(&mut self, project: ProjectId, path: impl Into<PathBuf>) {
        let Some(project) = self.project_mut(project) else {
            return;
        };

        let path = helix_stdx::path::normalize(path.into());
        push_recent_file(&mut project.recent_files, path);
    }

    pub fn record_global_recent_file(&mut self, path: impl Into<PathBuf>) {
        let path = helix_stdx::path::normalize(path.into());
        push_recent_file(&mut self.recent_files, path);
    }

    fn from_persisted(persisted: PersistedRegistry) -> Self {
        let mut registry = Self {
            active_project: persisted.active_project,
            next_id: persisted.next_id.max(1),
            recent_files: normalize_recent_files(persisted.recent_files),
            ..Self::default()
        };

        for mut project in persisted.projects {
            project.root = normalize_root(project.root);
            project.recent_files = normalize_recent_files(project.recent_files);
            registry.next_id = registry.next_id.max(project.id.0 + 1);
            registry
                .projects_by_root
                .insert(project.root.clone(), project.id);
            registry.projects.insert(project.id, project);
        }

        if registry
            .active_project
            .is_some_and(|id| !registry.projects.contains_key(&id))
        {
            registry.active_project = None;
        }

        registry
    }

    fn to_persisted(&self) -> PersistedRegistry {
        PersistedRegistry {
            active_project: self.active_project,
            next_id: self.next_id,
            recent_files: self.recent_files.clone(),
            projects: self.projects.values().cloned().collect(),
        }
    }
}

fn normalize_root(root: impl Into<PathBuf>) -> PathBuf {
    helix_stdx::path::normalize(root.into())
}

fn project_name_from_root(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| root.to_str().unwrap_or("project"))
        .to_string()
}

fn push_recent_file(recent_files: &mut Vec<PathBuf>, path: PathBuf) {
    recent_files.retain(|recent| recent != &path);
    recent_files.insert(0, path);
    recent_files.truncate(MAX_RECENT_FILES);
}

fn normalize_recent_files(recent_files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut normalized = Vec::new();
    for path in recent_files.into_iter().rev() {
        push_recent_file(&mut normalized, helix_stdx::path::normalize(path));
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_registry_round_trips_alias_and_mru() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("alpha");
        std::fs::create_dir(&root).unwrap();
        let loose_file = temp.path().join("ghostty.config");
        std::fs::write(&loose_file, "").unwrap();

        let mut registry = ProjectRegistry::default();
        let id = registry.ensure_project(root.clone());
        registry.project_mut(id).unwrap().alias = Some("work".into());
        registry.record_global_recent_file(loose_file.clone());
        registry.record_recent_file(id, root.join("src/main.rs"));
        registry.set_active(Some(id));

        let path = temp.path().join("projects.toml");
        registry.save_to_path(&path).unwrap();

        let loaded = ProjectRegistry::load_from_path(&path).unwrap();
        let project = loaded.project_by_root(&root).unwrap();
        assert_eq!(project.display_name(), "work");
        assert_eq!(project.recent_files, vec![root.join("src/main.rs")]);
        assert_eq!(loaded.recent_files(), &[loose_file]);
        assert_eq!(loaded.active_project(), Some(project.id));
    }

    #[test]
    fn project_name_defaults_to_root_basename() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("helix-fork");
        std::fs::create_dir(&root).unwrap();

        let mut registry = ProjectRegistry::default();
        let id = registry.ensure_project(root);

        assert_eq!(registry.project(id).unwrap().display_name(), "helix-fork");
    }

    #[test]
    fn document_membership_uses_longest_project_root() {
        let temp = tempfile::tempdir().unwrap();
        let outer = temp.path().join("repo");
        let inner = outer.join("crates/core");
        std::fs::create_dir_all(inner.join("src")).unwrap();

        let mut registry = ProjectRegistry::default();
        let outer_id = registry.ensure_project(outer.clone());
        let inner_id = registry.ensure_project(inner.clone());

        assert_eq!(
            registry.project_for_path(&inner.join("src/lib.rs")),
            Some(inner_id)
        );
        assert_eq!(
            registry.project_for_path(&outer.join("README.md")),
            Some(outer_id)
        );
    }

    #[test]
    fn document_ids_and_recent_files_are_mru() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let loose_file = temp.path().join("ghostty.config");
        std::fs::write(&loose_file, "").unwrap();

        let mut registry = ProjectRegistry::default();
        let project = registry.ensure_project(root.clone());
        registry.assign_document(DocumentId::new(1), project);
        registry.assign_document(DocumentId::new(2), project);
        registry.record_project_focus(project, DocumentId::new(1));
        registry.record_project_focus(project, DocumentId::new(2));

        registry.record_global_recent_file(root.join("a.rs"));
        registry.record_global_recent_file(loose_file.clone());
        registry.record_global_recent_file(root.join("a.rs"));
        registry.record_recent_file(project, root.join("a.rs"));
        registry.record_recent_file(project, root.join("b.rs"));
        registry.record_recent_file(project, root.join("a.rs"));

        assert_eq!(
            registry.documents_for_project(project).collect::<Vec<_>>(),
            vec![DocumentId::new(2), DocumentId::new(1)]
        );
        assert_eq!(
            registry.project(project).unwrap().recent_files,
            vec![root.join("a.rs"), root.join("b.rs")]
        );
        assert_eq!(
            registry.recent_files(),
            &[root.join("a.rs"), loose_file.clone()]
        );
        assert!(!registry
            .project(project)
            .unwrap()
            .recent_files
            .contains(&loose_file));
    }

    #[test]
    fn recent_files_are_capped() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();

        let mut registry = ProjectRegistry::default();
        let project = registry.ensure_project(root.clone());
        for index in 0..(MAX_RECENT_FILES + 1) {
            let path = root.join(format!("{index}.rs"));
            registry.record_global_recent_file(path.clone());
            registry.record_recent_file(project, path);
        }

        assert_eq!(registry.recent_files().len(), MAX_RECENT_FILES);
        assert_eq!(
            registry.project(project).unwrap().recent_files.len(),
            MAX_RECENT_FILES
        );
        assert_eq!(registry.recent_files()[0], root.join("100.rs"));
        assert_eq!(
            registry.project(project).unwrap().recent_files[0],
            root.join("100.rs")
        );
        assert!(!registry.recent_files().contains(&root.join("0.rs")));
        assert!(!registry
            .project(project)
            .unwrap()
            .recent_files
            .contains(&root.join("0.rs")));
    }
}
