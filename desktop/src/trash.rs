//! Move an original into the system trash, or return an error.
use anyhow::Result;
use std::path::Path;

#[cfg(windows)]
pub(crate) fn recycle(path: &Path) -> Result<()> {
    use windows::{
        Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        Win32::UI::Shell::{
            BHID_Transfer, FOLDERID_RecycleBinFolder, IShellItem, ITransferSource, KF_FLAG_DEFAULT,
            SHCreateItemFromParsingName, SHGetKnownFolderItem,
        },
        core::HSTRING,
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = Apartment;
        let item: IShellItem =
            SHCreateItemFromParsingName(&HSTRING::from(dunce::simplified(path).as_os_str()), None)?;
        let source: ITransferSource = item.GetParent()?.BindToHandler(None, &BHID_Transfer)?;
        let bin: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)?;
        source.RecycleItem(&item, &bin, 0)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn recycle(path: &Path) -> Result<()> {
    use objc2_foundation::{NSFileManager, NSURL};
    let url = NSURL::from_file_path(path).ok_or_else(|| anyhow::anyhow!("Invalid file URL"))?;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|err| anyhow::anyhow!("{err}"))
}

#[cfg(target_os = "linux")]
pub(crate) fn recycle(path: &Path) -> Result<()> {
    use gio::prelude::FileExt;
    gio::File::for_path(path).trash(None::<&gio::Cancellable>)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "uses the real OS trash with a disposable test file"]
    fn system_trash_accepts_a_disposable_photo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(format!("OpenRoom-trash-test-{}.jpg", std::process::id()));
        std::fs::write(&path, b"OpenRoom disposable recycle test").unwrap();
        super::recycle(&path.canonicalize().unwrap()).unwrap();
        assert!(!path.exists());
        println!("Recycled {}", path.display());
    }
}
