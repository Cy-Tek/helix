use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperation {
    CreateFile { path: PathBuf },
    CreateDirectory { path: PathBuf },
    Rename { from: PathBuf, to: PathBuf },
    Move { from: PathBuf, to: PathBuf },
    Copy { from: PathBuf, to: PathBuf },
    Trash { paths: Vec<PathBuf> },
    ForceDelete { paths: Vec<PathBuf> },
}

#[derive(Debug)]
pub enum FileOperationError {
    Io(io::Error),
    TargetExists(PathBuf),
    Trash(String),
}

impl fmt::Display for FileOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(formatter, "{err}"),
            Self::TargetExists(path) => {
                write!(formatter, "target already exists: {}", path.display())
            }
            Self::Trash(err) => write!(formatter, "failed to move to trash: {err}"),
        }
    }
}

impl From<io::Error> for FileOperationError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FileOperationService;

impl FileOperationService {
    pub fn execute(&self, operation: FileOperation) -> Result<(), FileOperationError> {
        match operation {
            FileOperation::CreateFile { path } => {
                ensure_parent_exists(&path)?;
                ensure_absent(&path)?;
                fs::File::create(path)?;
            }
            FileOperation::CreateDirectory { path } => {
                ensure_absent(&path)?;
                fs::create_dir_all(path)?;
            }
            FileOperation::Rename { from, to } | FileOperation::Move { from, to } => {
                ensure_parent_exists(&to)?;
                ensure_absent(&to)?;
                fs::rename(from, to)?;
            }
            FileOperation::Copy { from, to } => {
                ensure_parent_exists(&to)?;
                ensure_absent(&to)?;
                copy_recursively(&from, &to)?;
            }
            FileOperation::Trash { paths } => {
                trash::delete_all(&paths)
                    .map_err(|err| FileOperationError::Trash(err.to_string()))?;
            }
            FileOperation::ForceDelete { paths } => {
                for path in paths {
                    if path.is_dir() {
                        fs::remove_dir_all(path)?;
                    } else {
                        fs::remove_file(path)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn ensure_absent(path: &Path) -> Result<(), FileOperationError> {
    if path.exists() {
        Err(FileOperationError::TargetExists(path.to_path_buf()))
    } else {
        Ok(())
    }
}

fn ensure_parent_exists(path: &Path) -> Result<(), FileOperationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn copy_recursively(from: &Path, to: &Path) -> Result<(), FileOperationError> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        fs::copy(from, to)?;
    }
    Ok(())
}
