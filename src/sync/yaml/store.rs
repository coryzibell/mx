//! YAML file store - read/write with snapshot management
//!
//! Handles reading/writing YAML files and managing the sync directory.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::schema::{LastSynced, SyncYaml};

/// YAML store for a sync directory
pub struct YamlStore {
    dir: PathBuf,
}

impl YamlStore {
    /// Create a new store for the given directory
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Get the store directory path
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Ensure the store directory exists
    pub fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("Failed to create directory: {}", self.dir.display()))
    }

    /// List all YAML files in the store
    pub fn list_files(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        if !self.dir.exists() {
            return Ok(files);
        }

        for entry in fs::read_dir(&self.dir)
            .with_context(|| format!("Failed to read directory: {}", self.dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();

            if let Some(ext) = path.extension()
                && (ext == "yaml" || ext == "yml")
            {
                files.push(path);
            }
        }

        files.sort();
        Ok(files)
    }

    /// Read a YAML file
    pub fn read(&self, path: &Path) -> Result<SyncYaml> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;

        serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse YAML: {}", path.display()))
    }

    /// Read all YAML files in the store
    pub fn read_all(&self) -> Result<Vec<(PathBuf, SyncYaml)>> {
        let files = self.list_files()?;
        let mut results = Vec::with_capacity(files.len());

        for path in files {
            match self.read(&path) {
                Ok(yaml) => results.push((path, yaml)),
                Err(e) => eprintln!("Warning: skipping {}: {}", path.display(), e),
            }
        }

        Ok(results)
    }

    /// Write a YAML file
    pub fn write(&self, path: &Path, yaml: &SyncYaml) -> Result<()> {
        let content = serde_yaml::to_string(yaml)
            .with_context(|| format!("Failed to serialize YAML for: {}", path.display()))?;

        fs::write(path, content)
            .with_context(|| format!("Failed to write file: {}", path.display()))
    }

    /// Write a YAML file with a generated filename
    pub fn write_new(&self, filename: &str, yaml: &SyncYaml) -> Result<PathBuf> {
        self.ensure_dir()?;
        let path = self.dir.join(filename);
        self.write(&path, yaml)?;
        Ok(path)
    }

    /// Find a YAML file by GitHub issue number
    pub fn find_by_issue_number(&self, number: u64) -> Result<Option<(PathBuf, SyncYaml)>> {
        for path in self.list_files()? {
            if let Ok(yaml) = self.read(&path)
                && yaml.metadata.github_issue_number == Some(number)
            {
                return Ok(Some((path, yaml)));
            }
        }
        Ok(None)
    }

    /// Find a YAML file by GitHub discussion ID
    pub fn find_by_discussion_id(&self, id: &str) -> Result<Option<(PathBuf, SyncYaml)>> {
        for path in self.list_files()? {
            if let Ok(yaml) = self.read(&path)
                && yaml.metadata.github_discussion_id.as_deref() == Some(id)
            {
                return Ok(Some((path, yaml)));
            }
        }
        Ok(None)
    }

    /// Find a YAML file by title (exact match)
    pub fn find_by_title(&self, title: &str) -> Result<Option<(PathBuf, SyncYaml)>> {
        for path in self.list_files()? {
            if let Ok(yaml) = self.read(&path)
                && yaml.title() == title
            {
                return Ok(Some((path, yaml)));
            }
        }
        Ok(None)
    }

    /// Update the last_synced snapshot in a YAML file
    pub fn update_snapshot(&self, path: &Path, snapshot: LastSynced) -> Result<()> {
        let mut yaml = self.read(path)?;
        yaml.metadata.last_synced = Some(snapshot);
        self.write(path, &yaml)
    }

    /// Set the GitHub issue number for a YAML file
    pub fn set_issue_number(&self, path: &Path, number: u64) -> Result<()> {
        let mut yaml = self.read(path)?;
        yaml.metadata.github_issue_number = Some(number);
        self.write(path, &yaml)
    }

    /// Set the GitHub discussion ID for a YAML file
    pub fn set_discussion_id(&self, path: &Path, id: &str, number: u64) -> Result<()> {
        let mut yaml = self.read(path)?;
        yaml.metadata.github_discussion_id = Some(id.to_string());
        yaml.metadata.github_discussion_number = Some(number);
        self.write(path, &yaml)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::yaml::schema::Metadata;
    use std::fs;
    use tempfile::TempDir;

    fn setup_store() -> (TempDir, YamlStore) {
        let temp_dir = TempDir::new().unwrap();
        let store = YamlStore::new(temp_dir.path().to_path_buf());
        (temp_dir, store)
    }

    #[test]
    fn test_ensure_dir() {
        let temp_dir = TempDir::new().unwrap();
        let nested = temp_dir.path().join("a").join("b").join("c");
        let store = YamlStore::new(nested.clone());

        assert!(!nested.exists());
        store.ensure_dir().unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn test_list_files() {
        let (temp_dir, store) = setup_store();

        // Create some files
        fs::write(temp_dir.path().join("a.yaml"), "metadata: {}").unwrap();
        fs::write(temp_dir.path().join("b.yml"), "metadata: {}").unwrap();
        fs::write(temp_dir.path().join("c.txt"), "not yaml").unwrap();

        let files = store.list_files().unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].to_string_lossy().contains("a.yaml"));
        assert!(files[1].to_string_lossy().contains("b.yml"));
    }

    #[test]
    fn test_read_write() {
        let (temp_dir, store) = setup_store();
        let path = temp_dir.path().join("test.yaml");

        let yaml = SyncYaml {
            metadata: Metadata {
                title: Some("Test Issue".to_string()),
                github_issue_number: Some(42),
                ..Default::default()
            },
            body_markdown: "Test body".to_string(),
            ..Default::default()
        };

        store.write(&path, &yaml).unwrap();
        let read_back = store.read(&path).unwrap();

        assert_eq!(read_back.title(), "Test Issue");
        assert_eq!(read_back.github_issue_number(), Some(42));
        assert!(read_back.body().contains("Test body"));
    }

    #[test]
    fn test_find_by_issue_number() {
        let (temp_dir, store) = setup_store();

        // Create files with different issue numbers
        let yaml1 = SyncYaml {
            metadata: Metadata {
                title: Some("Issue 1".to_string()),
                github_issue_number: Some(1),
                ..Default::default()
            },
            ..Default::default()
        };
        let yaml2 = SyncYaml {
            metadata: Metadata {
                title: Some("Issue 2".to_string()),
                github_issue_number: Some(2),
                ..Default::default()
            },
            ..Default::default()
        };

        store
            .write(&temp_dir.path().join("1.yaml"), &yaml1)
            .unwrap();
        store
            .write(&temp_dir.path().join("2.yaml"), &yaml2)
            .unwrap();

        let found = store.find_by_issue_number(2).unwrap();
        assert!(found.is_some());
        let (_, yaml) = found.unwrap();
        assert_eq!(yaml.title(), "Issue 2");

        let not_found = store.find_by_issue_number(999).unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_find_by_title() {
        let (temp_dir, store) = setup_store();

        let yaml = SyncYaml {
            metadata: Metadata {
                title: Some("Specific Title".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        store
            .write(&temp_dir.path().join("test.yaml"), &yaml)
            .unwrap();

        let found = store.find_by_title("Specific Title").unwrap();
        assert!(found.is_some());

        let not_found = store.find_by_title("Nonexistent").unwrap();
        assert!(not_found.is_none());
    }
}
