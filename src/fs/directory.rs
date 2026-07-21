use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock}
};

use anyhow::Context;
use js_sys::Reflect;
use tracing::Instrument;
use virtual_fs::{AsyncReadExt, AsyncWriteExt, FileSystem};
use wasm_bindgen::{prelude::wasm_bindgen, JsCast, JsValue};

use shared_buffer::OwnedBuffer;
use crate::{utils::Error, StringOrBytes, fs::hooks::Hooks};

/// A directory that can be mounted inside a WASIX instance.
#[derive(Debug, wasm_bindgen_derive::TryFromJsValue)]
#[wasm_bindgen]
pub struct Directory(Arc<dyn FileSystem>, RwLock<Hooks>);

impl Clone for Directory {
    /// Each clone gets its own Hooks
    fn clone(&self) -> Self {
        Self(self.0.clone(), RwLock::new(self.1.read().unwrap().clone()))
    }
}

#[wasm_bindgen]
impl Directory {
    /// Create a new {@link Directory}.
    #[wasm_bindgen(constructor)]
    pub fn new(init: Option<DirectoryInit>) -> Result<Directory, Error> {
        match init {
            Some(init) => {
                let fs = init.initialize()?;
                Ok(Directory(fs, Default::default()))
            }
            None => Ok(Directory::default()),
        }
    }

    #[wasm_bindgen(js_name = "setHooks")]
    pub fn set_hooks(&mut self, hooks: Hooks) {
        self.1.set(hooks).expect("poisoned")
    }

    /// Read the contents of a directory.
    #[wasm_bindgen(js_name = "readDir")]
    pub async fn read_dir(&self, mut path: String) -> Result<ListOfDirEntry, Error> {
        if !path.starts_with('/') {
            path.insert(0, '/');
        }

        let contents = js_sys::Array::new();

        let ty = JsValue::from_str("type");
        let file = JsValue::from_str("file");
        let dir = JsValue::from_str("dir");
        let symlink = JsValue::from_str("symlink");
        let unknown = JsValue::from_str("unknown");
        let name = JsValue::from_str("name");

        for entry in FileSystem::read_dir(self, path.as_ref())? {
            let entry = entry?;

            let entry_name = entry.file_name().to_string_lossy().to_string();
            let entry_type = match entry.file_type() {
                Ok(ft) if ft.is_dir() => &dir,
                Ok(ft) if ft.is_file() => &file,
                Ok(ft) if ft.is_symlink() => &symlink,
                _ => &unknown,
            };

            let dir_entry = js_sys::Object::new();
            Reflect::set(&dir_entry, &name, &JsValue::from(entry_name)).map_err(Error::js)?;
            Reflect::set(&dir_entry, &ty, entry_type).map_err(Error::js)?;

            contents.push(&dir_entry);
        }

        Ok(contents.unchecked_into())
    }

    /// Write to a file.
    ///
    /// If a string is provided, it is encoded as UTF-8.
    #[wasm_bindgen(js_name = "writeFile")]
    pub async fn write_file(&self, mut path: String, contents: StringOrBytes) -> Result<(), Error> {
        slashify(&mut path);

        let mut f = self
            .new_open_options()
            .write(true)
            .create(true)
            .open(&path)?;

        let contents = contents.as_bytes();
        f.write_all(&contents).await?;

        Ok(())
    }

    /// Writes a read-only, offloaded file.
    ///
    /// This is slightly more efficient than a regular memfs file.
    #[wasm_bindgen(js_name = "writeFileRO")]
    pub async fn write_file_ro(&self, mut path: String, contents: StringOrBytes) -> Result<(), Error> {
        slashify(&mut path);
        let path = PathBuf::from(path);

        if let Some(fs) = self.0.downcast_ref::<virtual_fs::mem_fs::FileSystem>() {
            fs.insert_ro_file(&path, OwnedBuffer::from_bytes(contents.as_bytes()))?;
            Ok(())
        }
        else { Err(Error::js("cannot create ro file: not a memfs")) }
    }

    /// Read the contents of a file from this directory.
    ///
    /// Note that the path is relative to the directory's root.
    #[wasm_bindgen(js_name = "readFile")]
    pub async fn read_file(&self, path: String) -> Result<js_sys::Uint8Array, Error> {
        let buffer = self._read_file(path).await?;
        Ok(js_sys::Uint8Array::from(&buffer[..]))
    }

    /// Read the contents of a file from this directory as a UTF-8 string.
    ///
    /// Note that the path is relative to the directory's root.
    #[wasm_bindgen(js_name = "readTextFile")]
    pub async fn read_text_file(&self, path: String) -> Result<js_sys::JsString, Error> {
        let buffer = self._read_file(path).await?;
        let string = String::from_utf8(buffer)?;
        Ok(string.into())
    }

    /// Read the contents of a file from this directory, synchronously.
    /// This is only possible if the file is in-memory.
    ///
    /// Note that the path is relative to the directory's root.
    #[wasm_bindgen(js_name = "readFileSync")]
    pub fn read_file_sync(&self, path: String) -> Result<js_sys::Uint8Array, Error> {
        let buffer = self._read_file_sync(path)?;
        Ok(js_sys::Uint8Array::from(&buffer[..]))
    }

    /// Create a directory.
    #[wasm_bindgen(js_name = "createDir")]
    pub async fn create_dir(&self, mut path: String) -> Result<(), Error> {
        slashify(&mut path);

        FileSystem::create_dir(self, path.as_ref())?;

        Ok(())
    }

    /// Create a directory.
    #[wasm_bindgen(js_name = "createDirs")]
    pub async fn create_dirs(&self, mut path: String) -> Result<(), Error> {
        slashify(&mut path);

        create_dir_all(self, path.as_ref())?;

        Ok(())
    }

    /// Remove a directory.
    #[wasm_bindgen(js_name = "removeDir")]
    pub async fn remove_dir(&self, mut path: String) -> Result<(), Error> {
        if !path.starts_with('/') {
            path.insert(0, '/');
        }

        FileSystem::remove_dir(self, path.as_ref())?;

        Ok(())
    }

    /// Remove a file.
    #[wasm_bindgen(js_name = "removeFile")]
    pub async fn remove_file(&self, mut path: String) -> Result<(), Error> {
        if !path.starts_with('/') {
            path.insert(0, '/');
        }

        FileSystem::remove_file(self, path.as_ref())?;

        Ok(())
    }

    #[wasm_bindgen(js_name = "mountDir")]
    pub fn mount_dir(&self, mut path: String, dir: &Directory) -> Result<(), Error> {
        slashify(&mut path);

        if let Some(mfs) = self.0.downcast_ref::<virtual_fs::MountFileSystem>() {
            mfs.mount(path, Arc::new(dir.clone()))?;
        }

        Ok(())
    }

    #[wasm_bindgen(js_name = "createSymlink")]
    pub fn create_symlink(&self, mut source: String, mut target: String) -> Result<(), Error> {
        slashify(&mut source);
        slashify(&mut target);

        FileSystem::create_symlink(self, source.as_ref(), target.as_ref())?;

        Ok(())
    }
}

impl Directory {
    pub fn wrap(fs: Arc<dyn FileSystem>) -> Self {
        Directory(fs.clone(), Default::default())
    }

    async fn _read_file(&self, mut path: String) -> Result<Vec<u8>, Error> {
        slashify(&mut path);

        let mut f = self.new_open_options().read(true).open(&path)?;
        let mut buffer = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buffer).await?;

        Ok(buffer)
    }

    fn _read_file_sync(&self, mut path: String) -> Result<Vec<u8>, Error> {
        slashify(&mut path);

        let f = self.new_open_options().read(true).open(&path)?;
        if let Some(obuf) = f.as_owned_buffer() {
            let buffer = obuf.as_slice().to_vec();
            Ok(buffer)
        }
        else { Err(Error::JavaScript("not synchronous file".into())) }
    }
}

impl Default for Directory {
    fn default() -> Self {
        Directory(Arc::new(virtual_fs::mem_fs::FileSystem::default()), Default::default())
    }
}

impl FileSystem for Directory {
    #[tracing::instrument(level = "trace", skip(self))]
    fn read_dir(&self, path: &std::path::Path) -> virtual_fs::Result<virtual_fs::ReadDir> {
        self.1.write().unwrap().trigger_populate();
        self.0.read_dir(path)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn create_dir(&self, path: &std::path::Path) -> virtual_fs::Result<()> {
        self.0.create_dir(path)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn create_symlink(&self, source: &Path, target: &Path) -> virtual_fs::Result<()> {
        self.0.create_symlink(source, target)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn remove_dir(&self, path: &std::path::Path) -> virtual_fs::Result<()> {
        self.0.remove_dir(path)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn rename<'a>(
        &'a self,
        from: &'a std::path::Path,
        to: &'a std::path::Path,
    ) -> futures::future::BoxFuture<'a, virtual_fs::Result<()>> {
        Box::pin(self.0.rename(from, to).in_current_span())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn metadata(&self, path: &std::path::Path) -> virtual_fs::Result<virtual_fs::Metadata> {
        self.0.metadata(path)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn remove_file(&self, path: &std::path::Path) -> virtual_fs::Result<()> {
        self.0.remove_file(path)
    }

    fn new_open_options(&self) -> virtual_fs::OpenOptions<'_> {
        self.1.write().unwrap().trigger_populate();
        virtual_fs::OpenOptions::new(self)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn readlink(&self, path: &Path) -> virtual_fs::Result<PathBuf> {
        self.0.readlink(path)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn symlink_metadata(&self, path: &Path) -> virtual_fs::Result<virtual_fs::Metadata> {
        self.1.write().unwrap().trigger_populate();
        self.0.symlink_metadata(path)
    }

    /*
    #[tracing::instrument(level = "trace", skip(self))]
    fn mount(&self, name: String, path: &Path, fs: Box<dyn FileSystem + Send + Sync>)
            -> virtual_fs::Result<()> {
        self.0.mount(name, path, fs)
    }*/
}

impl virtual_fs::FileOpener for Directory {
    #[tracing::instrument(level = "trace", skip(self))]
    fn open(
        &self,
        path: &std::path::Path,
        conf: &virtual_fs::OpenOptionsConfig,
    ) -> virtual_fs::Result<Box<dyn virtual_fs::VirtualFile + Send + Sync + 'static>> {
        self.0.new_open_options().options(conf.clone()).open(path)
    }
}

#[wasm_bindgen(typescript_custom_section)]
const DIRENTRY_TYPE_DEF: &'static str = r#"
/**
 * An entry in a {@link Directory}.
 */
export type DirEntry = {
    /**
     * What type of entry is this?
     */
    type: "file" | "dir" | "unknown";
    /**
     * What is the item's name? (the last component in the path)
     */
    name: string;
};
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "DirEntry[]")]
    pub type ListOfDirEntry;
}

#[wasm_bindgen(typescript_custom_section)]
const DIRECTORY_INIT_TYPE_DEF: &'static str = r#"
/**
 * A mapping from file paths to their contents that can be used to initialize
 * a {@link Directory}.
 */
export type DirectoryInit = Record<string, string | Uint8Array>;
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "DirectoryInit", extends = js_sys::Object)]
    #[derive(Debug, Clone, PartialEq)]
    pub type DirectoryInit;
}

impl DirectoryInit {
    fn initialize(&self) -> Result<Arc<dyn FileSystem>, Error> {
        if let Some(record) = self.dyn_ref::<js_sys::Object>() {
            let fs = in_memory_filesystem(record)?;
            Ok(Arc::new(fs))
        } else {
            unreachable!()
        }
    }
}

fn slashify(path: &mut String) {
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
}

/// Construct an in-memory [`FileSystem`] based on an object mapping paths to
/// their contents (`Record<string, string | Uint8Array>`).
fn in_memory_filesystem(record: &js_sys::Object) -> Result<virtual_fs::mem_fs::FileSystem, Error> {
    let fs = virtual_fs::mem_fs::FileSystem::default();

    for (key, contents) in crate::utils::object_entries(record)? {
        let mut path = String::from(key);
        slashify(&mut path);
        let path = PathBuf::from(path);

        let contents: StringOrBytes = contents.unchecked_into();
        let contents = contents.as_bytes();

        if let Some(parent) = path.parent() {
            create_dir_all(&fs, parent)?;
        }


        tracing::trace!(
            path=%path.display(),
            file.length=contents.len(),
            "Adding file to directory",
        );
        fs.insert_ro_file(&path, OwnedBuffer::from_bytes(contents))?;

        /*
        InlineWaker::block_on(async {
            let mut f = fs
                .new_open_options()
                .write(true)
                .create_new(true)
                .open(&path)?;
            f.write_all(&contents).await?;
            f.flush().await
        })
        .with_context(|| format!("Unable to write to \"{}\"", path.display()))?;*/
    }

    Ok(fs)
}

#[tracing::instrument(level = "trace", skip(fs))]
fn create_dir_all(fs: &dyn FileSystem, path: &Path) -> Result<(), anyhow::Error> {
    let ancestors: Vec<&Path> = path.ancestors().collect();

    for ancestor in ancestors.into_iter().rev() {
        if fs.read_dir(ancestor).is_ok() {
            continue;
        }

        fs.create_dir(ancestor).with_context(|| {
            format!("Unable to create directory: \"{}\"", ancestor.display())
        })?;
    }

    Ok(())
}
