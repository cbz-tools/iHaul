// App container operations via house_arrest + AFC

use std::collections::HashMap;
use idevice::{
    IdeviceService,
    afc::opcode::AfcFopenMode,
    house_arrest::HouseArrestClient,
    provider::IdeviceProvider,
};

pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Lists entries and fetches metadata (is_dir, size, mtime) in a single connection.
pub async fn list_dir_with_metadata(
    provider: &dyn IdeviceProvider,
    bundle_id: &str,
    path: &str,
) -> Result<(Vec<DirEntry>, HashMap<String, (u64, String)>), Box<dyn std::error::Error>> {
    let ha = HouseArrestClient::connect(provider).await?;
    let mut afc = ha.vend_documents(bundle_id).await?;

    let mut names = afc.list_dir(path).await?;
    names.retain(|e| e != "." && e != "..");
    names.sort_unstable();

    let mut entries = Vec::with_capacity(names.len());
    let mut info: HashMap<String, (u64, String)> = HashMap::with_capacity(names.len()); // (size_bytes, modified_str)

    for name in names {
        let full = format!("{path}/{name}");
        if let Ok(fi) = afc.get_file_info(&full).await {
            let is_dir = fi.st_ifmt == "S_IFDIR";
            let size = fi.size as u64;
            let modified = fi.modified.format("%Y/%m/%d %H:%M").to_string();
            entries.push(DirEntry { name: name.clone(), is_dir });
            info.insert(name, (size, modified));
        } else {
            entries.push(DirEntry { name, is_dir: false });
        }
    }

    Ok((entries, info))
}

/// Creates a directory at the given path.
pub async fn make_dir(
    provider: &dyn IdeviceProvider,
    bundle_id: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ha = HouseArrestClient::connect(provider).await?;
    let mut afc = ha.vend_documents(bundle_id).await?;
    afc.mk_dir(path).await?;
    Ok(())
}

/// Removes a file or folder recursively.
#[allow(dead_code)]
pub async fn remove_item(
    provider: &dyn IdeviceProvider,
    bundle_id: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ha = HouseArrestClient::connect(provider).await?;
    let mut afc = ha.vend_documents(bundle_id).await?;
    afc.remove_all(path).await?;
    Ok(())
}

/// Uploads a file to the device in chunks (constant memory usage).
pub async fn upload_file_with_progress<F>(
    provider: &dyn IdeviceProvider,
    bundle_id: &str,
    local_path: &std::path::Path,
    current_dir: &str,
    filename: &str,
    cancel: &std::sync::atomic::AtomicBool,
    mut on_progress: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnMut(u64, u64),
{
    use tokio::io::AsyncReadExt;

    let ha = HouseArrestClient::connect(provider).await?;
    let mut afc = ha.vend_documents(bundle_id).await?;

    let dest = format!("{current_dir}/{filename}");
    let total = tokio::fs::metadata(local_path).await?.len();
    let mut local_file = tokio::fs::File::open(local_path).await?;
    let mut ios_file = afc.open(&dest, AfcFopenMode::WrOnly).await?;

    const CHUNK: usize = 256 * 1024;
    let mut buf = vec![0u8; CHUNK];
    let mut written = 0u64;

    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            drop(ios_file);
            let _ = afc.remove(&dest).await;
            log::warn!("upload cancelled mid-transfer, partial file removed: {dest}");
            return Err("cancelled".into());
        }
        let n = local_file.read(&mut buf).await?;
        if n == 0 { break; }
        ios_file.write_entire(&buf[..n]).await?;
        written += n as u64;
        on_progress(written, total);
    }
    Ok(())
}

/// Deletes a file or folder at the given absolute path (recursively).
pub async fn delete_item(
    provider: &dyn IdeviceProvider,
    bundle_id: &str,
    abs_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ha = HouseArrestClient::connect(provider).await?;
    let mut afc = ha.vend_documents(bundle_id).await?;
    afc.remove_all(abs_path).await?;
    Ok(())
}

/// Downloads a file in chunks and writes directly to disk (constant memory usage).
pub async fn download_file_with_progress<F>(
    provider: &dyn IdeviceProvider,
    bundle_id: &str,
    current_dir: &str,
    filename: &str,
    dest_path: &std::path::Path,
    cancel: &std::sync::atomic::AtomicBool,
    mut on_progress: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnMut(u64, u64),
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use std::sync::atomic::Ordering;

    let ha = HouseArrestClient::connect(provider).await?;
    let mut afc = ha.vend_documents(bundle_id).await?;
    let path = format!("{current_dir}/{filename}");

    let total = afc.get_file_info(&path).await
        .map(|i| i.size as u64)
        .unwrap_or(0);

    let mut ios_file = afc.open(path, AfcFopenMode::RdOnly).await?;
    let mut local_file = tokio::fs::File::create(dest_path).await?;

    const CHUNK: usize = 256 * 1024;
    let mut buf = vec![0u8; CHUNK];
    let mut read_bytes = 0u64;

    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(local_file);
            let _ = tokio::fs::remove_file(dest_path).await;
            return Err("cancelled".into());
        }
        let n = ios_file.read(&mut buf).await?;
        if n == 0 { break; }
        local_file.write_all(&buf[..n]).await?;
        read_bytes += n as u64;
        on_progress(read_bytes, total.max(read_bytes));
    }
    Ok(())
}

/// Renames a file or folder (old_abs and new_abs must be absolute paths).
pub async fn rename_file(
    provider: &dyn IdeviceProvider,
    bundle_id: &str,
    old_abs: &str,
    new_abs: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ha = HouseArrestClient::connect(provider).await?;
    let mut afc = ha.vend_documents(bundle_id).await?;
    afc.rename(old_abs, new_abs).await?;
    Ok(())
}

/// A single file task produced by the recursive export scan.
pub struct DownloadTask {
    pub ios_abs:   String,           // absolute path on iOS
    pub local_rel: std::path::PathBuf, // relative local path under dest_dir
    pub size:      u64,
}

/// Scans selected iOS paths recursively and returns a flat list of file tasks + total bytes.
/// Uses a single AFC connection (BFS, no async recursion).
pub async fn scan_for_download(
    provider:  &dyn IdeviceProvider,
    bundle_id: &str,
    selected:  &[String],
) -> Result<(Vec<DownloadTask>, u64), Box<dyn std::error::Error>> {
    let ha  = HouseArrestClient::connect(provider).await?;
    let mut afc = ha.vend_documents(bundle_id).await?;

    let mut tasks       = Vec::new();
    let mut total_bytes = 0u64;

    for ios_path in selected {
        let name      = ios_path.rsplit('/').next().unwrap_or(ios_path.as_str());
        let local_rel = std::path::PathBuf::from(name);

        if let Ok(fi) = afc.get_file_info(ios_path).await {
            if fi.st_ifmt == "S_IFDIR" {
                let mut stack: Vec<(String, std::path::PathBuf)> =
                    vec![(ios_path.clone(), local_rel)];

                while let Some((ios_dir, lrel)) = stack.pop() {
                    let mut names = afc.list_dir(&ios_dir).await.unwrap_or_default();
                    names.retain(|n| n != "." && n != "..");

                    for n in names {
                        let child_ios  = format!("{ios_dir}/{n}");
                        let child_lrel = lrel.join(&n);

                        if let Ok(cfi) = afc.get_file_info(&child_ios).await {
                            if cfi.st_ifmt == "S_IFDIR" {
                                stack.push((child_ios, child_lrel));
                            } else {
                                total_bytes += cfi.size as u64;
                                tasks.push(DownloadTask {
                                    ios_abs:   child_ios,
                                    local_rel: child_lrel,
                                    size:      cfi.size as u64,
                                });
                            }
                        }
                    }
                }
            } else {
                total_bytes += fi.size as u64;
                tasks.push(DownloadTask {
                    ios_abs:   ios_path.clone(),
                    local_rel,
                    size:      fi.size as u64,
                });
            }
        }
    }

    Ok((tasks, total_bytes))
}

/// Debug helper: connects via house_arrest and lists /Documents.
#[allow(dead_code)]
pub async fn poc_connect(
    provider: &dyn IdeviceProvider,
    bundle_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ha = HouseArrestClient::connect(provider).await?;
    let mut afc = ha.vend_documents(bundle_id).await?;
    let entries = afc.list_dir("/Documents").await?;
    for entry in &entries {
        println!("  {entry}");
    }
    Ok(())
}
