// App Documents container operations via house_arrest + AFC.

use std::collections::HashMap;

use idevice::{
    IdeviceService,
    afc::{AfcClient, opcode::AfcFopenMode},
    house_arrest::HouseArrestClient,
    provider::IdeviceProvider,
};

pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// One exclusive AFC connection to an app's Documents container.
///
/// `DocumentsPool` owns idle instances and lends each one to exactly one
/// operation at a time.
pub struct DocumentsSession {
    afc: AfcClient,
}

pub async fn open_documents_session(
    provider: &dyn IdeviceProvider,
    bundle_id: &str,
) -> Result<DocumentsSession, Box<dyn std::error::Error>> {
    let ha = HouseArrestClient::connect(provider).await?;
    Ok(DocumentsSession { afc: ha.vend_documents(bundle_id).await? })
}

impl DocumentsSession {
    /// Lists entries and fetches metadata with this same AFC connection.
    pub async fn list_dir_with_metadata(
        &mut self,
        path: &str,
    ) -> Result<(Vec<DirEntry>, HashMap<String, (u64, String)>), Box<dyn std::error::Error>> {
        let mut names = self.afc.list_dir(path).await?;
        names.retain(|e| e != "." && e != "..");
        names.sort_unstable();

        let mut entries = Vec::with_capacity(names.len());
        let mut info = HashMap::with_capacity(names.len());

        for name in names {
            let full = format!("{path}/{name}");
            if let Ok(fi) = self.afc.get_file_info(&full).await {
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

    pub async fn make_dir(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.afc.mk_dir(path).await?;
        Ok(())
    }

    /// Deletes all requested paths with this same AFC connection.
    pub async fn delete_items(&mut self, abs_paths: &[String]) -> Result<(), Box<dyn std::error::Error>> {
        let mut first_error = None;
        for path in abs_paths {
            if let Err(e) = self.afc.remove_all(path).await {
                log::warn!("delete skipped: {path}: {e}");
                first_error.get_or_insert_with(|| e.to_string());
            }
        }
        if let Some(error) = first_error {
            return Err(std::io::Error::other(error).into());
        }
        Ok(())
    }

    pub async fn rename_file(
        &mut self,
        old_abs: &str,
        new_abs: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.afc.rename(old_abs, new_abs).await?;
        Ok(())
    }

    /// Uploads directly to the final name. A device loss can leave a partial file.
    pub async fn upload_file_with_progress<F>(
        &mut self,
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

        let dest = format!("{current_dir}/{filename}");
        let total = tokio::fs::metadata(local_path).await?.len();
        let mut local_file = tokio::fs::File::open(local_path).await?;
        let mut ios_file = self.afc.open(&dest, AfcFopenMode::WrOnly).await?;

        const CHUNK: usize = 256 * 1024;
        let mut buf = vec![0u8; CHUNK];
        let mut written = 0u64;

        let write_result: Result<bool, Box<dyn std::error::Error>> = async {
            loop {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(true);
                }
                let n = local_file.read(&mut buf).await?;
                if n == 0 { return Ok(false); }
                ios_file.write_entire(&buf[..n]).await?;
                written += n as u64;
                on_progress(written, total);
            }
        }.await;
        let close_result = ios_file.close().await;
        let cancelled = write_result?;
        close_result?;

        if cancelled {
            let _ = self.afc.remove(&dest).await;
            log::warn!("upload cancelled mid-transfer, partial file removed: {dest}");
            return Err("cancelled".into());
        }
        Ok(())
    }

    /// Downloads a file in chunks and explicitly closes its AFC handle.
    pub async fn download_file_with_progress<F>(
        &mut self,
        current_dir: &str,
        filename: &str,
        dest_path: &std::path::Path,
        cancel: &std::sync::atomic::AtomicBool,
        mut on_progress: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnMut(u64, u64),
    {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let path = format!("{current_dir}/{filename}");
        let total = self.afc.get_file_info(&path).await
            .map(|i| i.size as u64)
            .unwrap_or(0);
        let mut ios_file = self.afc.open(path, AfcFopenMode::RdOnly).await?;
        let mut local_file = tokio::fs::File::create(dest_path).await?;

        const CHUNK: usize = 256 * 1024;
        let mut buf = vec![0u8; CHUNK];
        let mut read_bytes = 0u64;

        let read_result: Result<bool, Box<dyn std::error::Error>> = async {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(true);
                }
                let n = ios_file.read(&mut buf).await?;
                if n == 0 { return Ok(false); }
                local_file.write_all(&buf[..n]).await?;
                read_bytes += n as u64;
                on_progress(read_bytes, total.max(read_bytes));
            }
        }.await;
        let close_result = ios_file.close().await;
        let cancelled = read_result?;
        close_result?;

        if cancelled {
            drop(local_file);
            let _ = tokio::fs::remove_file(dest_path).await;
            return Err("cancelled".into());
        }
        Ok(())
    }

    /// Recursively scans selected paths using this same AFC connection.
    pub async fn scan_for_download(
        &mut self,
        selected: &[String],
    ) -> Result<(Vec<DownloadTask>, u64), Box<dyn std::error::Error>> {
        let mut tasks = Vec::new();
        let mut total_bytes = 0u64;

        for ios_path in selected {
            let name = ios_path.rsplit('/').next().unwrap_or(ios_path.as_str());
            let local_rel = std::path::PathBuf::from(name);

            let fi = self.afc.get_file_info(ios_path).await?;
            if fi.st_ifmt == "S_IFDIR" {
                let mut stack = vec![(ios_path.clone(), local_rel)];
                while let Some((ios_dir, lrel)) = stack.pop() {
                    let mut names = self.afc.list_dir(&ios_dir).await?;
                    names.retain(|n| n != "." && n != "..");

                    for n in names {
                        let child_ios = format!("{ios_dir}/{n}");
                        let child_lrel = lrel.join(&n);
                        let cfi = self.afc.get_file_info(&child_ios).await?;
                        if cfi.st_ifmt == "S_IFDIR" {
                            stack.push((child_ios, child_lrel));
                        } else {
                            total_bytes += cfi.size as u64;
                            tasks.push(DownloadTask {
                                ios_abs: child_ios,
                                local_rel: child_lrel,
                                size: cfi.size as u64,
                            });
                        }
                    }
                }
            } else {
                total_bytes += fi.size as u64;
                tasks.push(DownloadTask {
                    ios_abs: ios_path.clone(),
                    local_rel,
                    size: fi.size as u64,
                });
            }
        }

        Ok((tasks, total_bytes))
    }
}

/// A single file task produced by the recursive export scan.
pub struct DownloadTask {
    pub ios_abs: String,
    pub local_rel: std::path::PathBuf,
    pub size: u64,
}
