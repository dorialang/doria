use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::build_plan::{Package, Source};
use crate::runtime_digest::sha256_hex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRequest<'a> {
    pub package: &'a Package,
    pub canonical_package_root: &'a Path,
    pub source: &'a Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeRequest<'a> {
    pub package: &'a Package,
    pub canonical_package_root: &'a Path,
    pub including_relative_path: &'a str,
    pub include_path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvidedSource {
    pub package_relative_path: String,
    pub display_path: String,
    pub canonical_path: Option<PathBuf>,
    pub text: String,
    pub content_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceProviderErrorKind {
    Missing,
    Directory,
    OutsidePackage,
    CaseMismatch,
    InvalidUtf8,
    Unreadable,
    InvalidPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceProviderError {
    pub kind: SourceProviderErrorKind,
    pub display_path: String,
    pub details: String,
}

pub trait SourceProvider {
    fn read_source(
        &self,
        request: SourceRequest<'_>,
    ) -> Result<ProvidedSource, SourceProviderError>;

    fn read_included_source(
        &self,
        request: IncludeRequest<'_>,
    ) -> Result<ProvidedSource, SourceProviderError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FileSystemSourceProvider;

impl SourceProvider for FileSystemSourceProvider {
    fn read_source(
        &self,
        request: SourceRequest<'_>,
    ) -> Result<ProvidedSource, SourceProviderError> {
        read_filesystem_source(
            request.canonical_package_root,
            &request.source.path,
            &request.package.identity,
        )
    }

    fn read_included_source(
        &self,
        request: IncludeRequest<'_>,
    ) -> Result<ProvidedSource, SourceProviderError> {
        let including = Path::new(request.including_relative_path);
        let parent = including.parent().unwrap_or_else(|| Path::new(""));
        let relative =
            normalize_relative_path(&parent.join(request.include_path)).map_err(|details| {
                SourceProviderError {
                    kind: SourceProviderErrorKind::InvalidPath,
                    display_path: request.include_path.to_string(),
                    details,
                }
            })?;
        read_filesystem_source(
            request.canonical_package_root,
            &relative,
            &request.package.identity,
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemorySourceProvider {
    sources: BTreeMap<(String, String), String>,
}

impl InMemorySourceProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        package: impl Into<String>,
        package_relative_path: impl AsRef<str>,
        text: impl Into<String>,
    ) {
        let path = normalized_slashes(package_relative_path.as_ref());
        self.sources.insert((package.into(), path), text.into());
    }
}

impl SourceProvider for InMemorySourceProvider {
    fn read_source(
        &self,
        request: SourceRequest<'_>,
    ) -> Result<ProvidedSource, SourceProviderError> {
        self.read(&request.package.identity, &request.source.path)
    }

    fn read_included_source(
        &self,
        request: IncludeRequest<'_>,
    ) -> Result<ProvidedSource, SourceProviderError> {
        let including = Path::new(request.including_relative_path);
        let parent = including.parent().unwrap_or_else(|| Path::new(""));
        let relative =
            normalize_relative_path(&parent.join(request.include_path)).map_err(|details| {
                SourceProviderError {
                    kind: SourceProviderErrorKind::InvalidPath,
                    display_path: request.include_path.to_string(),
                    details,
                }
            })?;
        self.read(&request.package.identity, &relative)
    }
}

impl InMemorySourceProvider {
    fn read(&self, package: &str, path: &str) -> Result<ProvidedSource, SourceProviderError> {
        let path =
            normalize_relative_path(Path::new(path)).map_err(|details| SourceProviderError {
                kind: SourceProviderErrorKind::InvalidPath,
                display_path: path.to_string(),
                details,
            })?;
        let text = self
            .sources
            .get(&(package.to_string(), path.clone()))
            .cloned()
            .ok_or_else(|| SourceProviderError {
                kind: SourceProviderErrorKind::Missing,
                display_path: path.clone(),
                details: "the in-memory source provider has no source at this path".to_string(),
            })?;
        Ok(ProvidedSource {
            package_relative_path: path.clone(),
            display_path: format!("{package}:{path}"),
            canonical_path: None,
            content_fingerprint: sha256_hex(text.as_bytes()),
            text,
        })
    }
}

fn read_filesystem_source(
    canonical_root: &Path,
    relative_path: &str,
    package: &str,
) -> Result<ProvidedSource, SourceProviderError> {
    let relative = normalize_relative_path(Path::new(relative_path)).map_err(|details| {
        SourceProviderError {
            kind: SourceProviderErrorKind::InvalidPath,
            display_path: relative_path.to_string(),
            details,
        }
    })?;
    let authored_path = canonical_root.join(&relative);
    if !path_uses_exact_case(canonical_root, Path::new(&relative))? {
        return Err(SourceProviderError {
            kind: SourceProviderErrorKind::CaseMismatch,
            display_path: relative.clone(),
            details: "the authored path casing does not match the filesystem entry".to_string(),
        });
    }
    let canonical = authored_path
        .canonicalize()
        .map_err(|error| SourceProviderError {
            kind: if error.kind() == std::io::ErrorKind::NotFound {
                SourceProviderErrorKind::Missing
            } else {
                SourceProviderErrorKind::Unreadable
            },
            display_path: relative.clone(),
            details: error.to_string(),
        })?;
    if !canonical.starts_with(canonical_root) {
        return Err(SourceProviderError {
            kind: SourceProviderErrorKind::OutsidePackage,
            display_path: relative.clone(),
            details: "canonicalization resolves outside the package root".to_string(),
        });
    }
    if canonical.is_dir() {
        return Err(SourceProviderError {
            kind: SourceProviderErrorKind::Directory,
            display_path: relative.clone(),
            details: "the source path resolves to a directory".to_string(),
        });
    }
    let bytes = fs::read(&canonical).map_err(|error| SourceProviderError {
        kind: SourceProviderErrorKind::Unreadable,
        display_path: relative.clone(),
        details: error.to_string(),
    })?;
    let text = String::from_utf8(bytes).map_err(|error| SourceProviderError {
        kind: SourceProviderErrorKind::InvalidUtf8,
        display_path: relative.clone(),
        details: error.to_string(),
    })?;
    Ok(ProvidedSource {
        package_relative_path: relative.clone(),
        display_path: format!("{package}:{relative}"),
        canonical_path: Some(canonical),
        content_fingerprint: sha256_hex(text.as_bytes()),
        text,
    })
}

pub fn normalize_relative_path(path: &Path) -> Result<String, String> {
    if path.is_absolute() {
        return Err("absolute paths are not package-relative".to_string());
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| "source paths must be valid UTF-8".to_string())?;
                parts.push(part.to_string());
            }
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err("the path escapes the package root".to_string());
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err("absolute paths are not package-relative".to_string())
            }
        }
    }
    if parts.is_empty() {
        return Err("the source path is empty".to_string());
    }
    Ok(parts.join("/"))
}

fn normalized_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

pub(crate) fn path_uses_exact_case(
    root: &Path,
    relative: &Path,
) -> Result<bool, SourceProviderError> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(expected) = component else {
            continue;
        };
        let entries = fs::read_dir(&current).map_err(|error| SourceProviderError {
            kind: if error.kind() == std::io::ErrorKind::NotFound {
                SourceProviderErrorKind::Missing
            } else {
                SourceProviderErrorKind::Unreadable
            },
            display_path: relative.display().to_string(),
            details: error.to_string(),
        })?;
        let mut exact = false;
        let mut folded = false;
        for entry in entries {
            let name = entry
                .map_err(|error| SourceProviderError {
                    kind: SourceProviderErrorKind::Unreadable,
                    display_path: relative.display().to_string(),
                    details: error.to_string(),
                })?
                .file_name();
            exact |= name == expected;
            folded |= name
                .to_string_lossy()
                .eq_ignore_ascii_case(&expected.to_string_lossy());
        }
        if !exact {
            return Ok(!folded && !current.join(expected).exists());
        }
        current.push(expected);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::build_plan::{SourceOrigin, SourceScope};

    fn package(root: &Path) -> Package {
        Package {
            identity: "acme/application".to_string(),
            root: root.display().to_string(),
            namespace_mappings: Vec::new(),
            sources: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    fn source(path: &str) -> Source {
        Source {
            identity: format!("acme/application:{path}"),
            path: path.to_string(),
            scope: SourceScope::Main,
            origin: SourceOrigin::Explicit,
            generated_for: None,
        }
    }

    fn fixture_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "doria-source-provider-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create fixture root");
        root.canonicalize().expect("canonical fixture root")
    }

    #[test]
    fn filesystem_provider_reads_valid_utf8_and_rejects_directories() {
        let root = fixture_root("utf8");
        fs::write(root.join("main.doria"), "function main(): void {}").expect("write source");
        fs::create_dir(root.join("directory.doria")).expect("create directory");
        let package = package(&root);
        let valid = source("main.doria");
        let directory = source("directory.doria");
        let provider = FileSystemSourceProvider;

        assert!(provider
            .read_source(SourceRequest {
                package: &package,
                canonical_package_root: &root,
                source: &valid,
            })
            .is_ok());
        let error = provider
            .read_source(SourceRequest {
                package: &package,
                canonical_package_root: &root,
                source: &directory,
            })
            .expect_err("directories are not source files");
        assert_eq!(error.kind, SourceProviderErrorKind::Directory);
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn filesystem_provider_rejects_invalid_utf8_and_case_mismatch() {
        let root = fixture_root("bytes-case");
        fs::write(root.join("invalid.doria"), [0xff, 0xfe]).expect("write invalid bytes");
        fs::write(root.join("Exact.doria"), "function helper(): void {}")
            .expect("write exact-case source");
        let package = package(&root);
        let invalid = source("invalid.doria");
        let wrong_case = source("exact.doria");
        let provider = FileSystemSourceProvider;

        let error = provider
            .read_source(SourceRequest {
                package: &package,
                canonical_package_root: &root,
                source: &invalid,
            })
            .expect_err("invalid UTF-8 is rejected");
        assert_eq!(error.kind, SourceProviderErrorKind::InvalidUtf8);

        let error = provider
            .read_source(SourceRequest {
                package: &package,
                canonical_package_root: &root,
                source: &wrong_case,
            })
            .expect_err("case mismatch is rejected portably");
        assert_eq!(error.kind, SourceProviderErrorKind::CaseMismatch);
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[cfg(unix)]
    #[test]
    fn filesystem_provider_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("symlink-root");
        let outside = fixture_root("symlink-outside");
        fs::write(outside.join("escaped.doria"), "function escaped(): void {}")
            .expect("write external source");
        symlink(outside.join("escaped.doria"), root.join("escaped.doria"))
            .expect("create source symlink");
        let package = package(&root);
        let escaped = source("escaped.doria");

        let error = FileSystemSourceProvider
            .read_source(SourceRequest {
                package: &package,
                canonical_package_root: &root,
                source: &escaped,
            })
            .expect_err("symlink escape is rejected");
        assert_eq!(error.kind, SourceProviderErrorKind::OutsidePackage);
        fs::remove_dir_all(root).expect("remove root fixture");
        fs::remove_dir_all(outside).expect("remove outside fixture");
    }
}
