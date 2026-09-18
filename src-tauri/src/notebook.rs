//! Notebook metadata is synced alongside tasks; the legacy notebook keeps its paths.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::store::{hash_text, CmdError};

pub const DEFAULT_NOTEBOOK: &str = "default";

#[derive(Debug, serde::Serialize)]
pub struct NotebookDto {
    pub id: String,
    pub name: String,
    pub base_version: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Metadata {
    version: u32,
    name: String,
}

fn directory(repo: &Path, id: &str) -> Result<PathBuf, CmdError> {
    if id.is_empty() || id.len() > 80 || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err(CmdError::bad_request("无效的笔记本 ID"));
    }
    let dir = repo.join("notebooks").join(id);
    // Never follow notebook directory links outside the journal.
    for path in [repo.join("notebooks"), dir.clone()] {
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(CmdError::bad_request("笔记本目录不能是链接"));
        }
    }
    Ok(dir)
}

pub fn read(repo: &Path, id: &str) -> Result<NotebookDto, CmdError> {
    let path = directory(repo, id)?.join("notebook.json");
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && id == DEFAULT_NOTEBOOK => {
            return Ok(NotebookDto { id: id.into(), name: "工作".into(), base_version: "0".into() });
        }
        Err(e) => return Err(CmdError::bad_request(format!("笔记本不存在或无法读取: {e}"))),
    };
    let meta: Metadata = serde_json::from_str(&text)
        .map_err(|e| CmdError::internal(format!("笔记本信息损坏: {e}")))?;
    if meta.version != 1 {
        return Err(CmdError::bad_request("请升级应用以读取此笔记本"));
    }
    validate_name(&meta.name)?;
    Ok(NotebookDto { id: id.into(), name: meta.name, base_version: hash_text(&text).to_string() })
}

pub fn root(repo: &Path, id: &str) -> Result<PathBuf, CmdError> {
    read(repo, id)?;
    if id == DEFAULT_NOTEBOOK { Ok(repo.to_path_buf()) } else { directory(repo, id) }
}

pub fn list(repo: &Path) -> Result<Vec<NotebookDto>, CmdError> {
    let mut books = vec![read(repo, DEFAULT_NOTEBOOK)?];
    let parent = directory(repo, DEFAULT_NOTEBOOK)?.parent().unwrap().to_path_buf();
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(books),
        Err(e) => return Err(CmdError::internal(format!("读取笔记本列表失败: {e}"))),
    };
    for entry in entries {
        let entry = entry.map_err(|e| CmdError::internal(e.to_string()))?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if id != DEFAULT_NOTEBOOK && entry.path().join("notebook.json").exists() {
            books.push(read(repo, &id)?);
        }
    }
    books[1..].sort_by(|a, b| a.id.cmp(&b.id));
    Ok(books)
}

fn validate_name(name: &str) -> Result<&str, CmdError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return Err(CmdError::bad_request("笔记本名称需要 1–40 个字符，不能包含换行或控制字符"));
    }
    Ok(name)
}

pub fn save(repo: &Path, id: Option<&str>, name: &str, base_version: u64) -> Result<NotebookDto, CmdError> {
    let name = validate_name(name)?;
    if let Some(id) = id {
        if read(repo, id)?.base_version != base_version.to_string() {
            return Err(CmdError::stale());
        }
    }
    if list(repo)?.iter().any(|b| Some(b.id.as_str()) != id && b.name.to_lowercase() == name.to_lowercase()) {
        return Err(CmdError::bad_request("已有同名笔记本，请使用其他名称"));
    }
    let id = id.map(str::to_owned).unwrap_or_else(|| format!("nb-{}-{}", unique_stamp(), std::process::id()));
    let dir = directory(repo, &id)?;
    std::fs::create_dir_all(&dir).map_err(|e| CmdError::internal(e.to_string()))?;
    let meta = serde_json::to_string_pretty(&Metadata { version: 1, name: name.into() })
        .map_err(|e| CmdError::internal(e.to_string()))?;
    atomic_write(&dir.join("notebook.json"), &(meta + "\n"))?;
    read(repo, &id)
}

fn unique_stamp() -> u128 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
        + u128::from(SEQ.fetch_add(1, Ordering::Relaxed))
}

/// Same-directory replacement keeps the old file intact if writing fails.
pub(crate) fn atomic_write(path: &Path, text: &str) -> Result<(), CmdError> {
    let tmp = path.with_extension(format!("{}.tmp", unique_stamp()));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
            let from: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        #[cfg(not(windows))]
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(tmp); }
    result.map_err(|e| CmdError::internal(format!("保存文件失败: {e}")))
}
