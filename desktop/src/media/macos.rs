use crate::Preview;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub struct Renderer {
    path: PathBuf,
}
impl Renderer {
    pub fn new(cache: &Path) -> anyhow::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/ae-preview"));
        let mut hash = DefaultHasher::new();
        bytes.hash(&mut hash);
        let path = cache.join(format!("renderer-{:x}", hash.finish()));
        if path.exists() {
            anyhow::ensure!(
                std::fs::symlink_metadata(&path)?.is_file(),
                "描画ヘルパーにシンボリックリンクは使えません。"
            );
        }
        // A supplied catalog may contain working files; execute only our bundled helper.
        if std::fs::read(&path).ok().as_deref() != Some(bytes) {
            use std::io::Write;
            let mut temporary = tempfile::NamedTempFile::new_in(cache)?;
            temporary.write_all(bytes)?;
            temporary
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o755))?;
            temporary.persist(&path)?;
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        Ok(Self { path })
    }
    pub fn cache_key(&self) -> String {
        self.path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }
    pub fn render(
        &self,
        source: &Path,
        destination: &Path,
        bound: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> anyhow::Result<Preview> {
        let mut command = std::process::Command::new(&self.path);
        command.arg(source).arg(destination).arg(bound.to_string());
        let mut value: serde_json::Value =
            super::run_worker(command, Some(destination), cancelled)?;
        value["path"] = "".into();
        Ok(serde_json::from_value(value)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supplied_working_directory_cannot_replace_the_bundled_executable() {
        let dir = tempfile::tempdir().unwrap();
        let renderer = Renderer::new(dir.path()).unwrap();
        std::fs::write(&renderer.path, b"untrusted executable").unwrap();
        let repaired = Renderer::new(dir.path()).unwrap();
        assert_eq!(
            std::fs::read(&repaired.path).unwrap(),
            include_bytes!(concat!(env!("OUT_DIR"), "/ae-preview"))
        );
        let outside = dir.path().join("outside");
        std::fs::write(&outside, b"preserve").unwrap();
        std::fs::remove_file(&repaired.path).unwrap();
        std::os::unix::fs::symlink(&outside, &repaired.path).unwrap();
        assert!(Renderer::new(dir.path()).is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"preserve");
    }
}
