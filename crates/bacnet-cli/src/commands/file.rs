//! File transfer commands: AtomicReadFile and AtomicWriteFile.

use std::error::Error as StdError;
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_client::client::BACnetClient;
use bacnet_services::file::FileWriteAccessMethod;
use bacnet_services::file::{AtomicReadFileAck, FileAccessMethod, FileReadAckMethod};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::args::FileReadAccess;
use crate::output::{self, OutputFormat};

type BoxError = Box<dyn StdError>;

static STAGING_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default)]
struct FileReadSummary {
    octets: u64,
    records: u64,
    windows: u64,
    display_data: Vec<u8>,
}

trait FileReadSink {
    fn write_stream(&mut self, data: &[u8]) -> io::Result<()>;
    fn write_record(&mut self, index: i32, data: &[u8]) -> io::Result<()>;
}

enum FileReadDestination {
    Display {
        data: Vec<u8>,
    },
    Stream {
        final_path: PathBuf,
        staging_path: PathBuf,
        file: Option<File>,
        active: bool,
    },
    Record {
        final_path: PathBuf,
        staging_path: PathBuf,
        active: bool,
    },
}

impl FileReadDestination {
    fn create(access: FileReadAccess, output: Option<&Path>) -> Result<Self, BoxError> {
        match (access, output) {
            (FileReadAccess::Stream, None) => Ok(Self::Display { data: Vec::new() }),
            (FileReadAccess::Stream, Some(final_path)) => {
                refuse_existing_target(final_path)?;
                let (staging_path, file) = create_staging_file(final_path)?;
                Ok(Self::Stream {
                    final_path: final_path.to_path_buf(),
                    staging_path,
                    file: Some(file),
                    active: true,
                })
            }
            (FileReadAccess::Record, Some(final_path)) => {
                refuse_existing_target(final_path)?;
                let staging_path = create_staging_directory(final_path)?;
                Ok(Self::Record {
                    final_path: final_path.to_path_buf(),
                    staging_path,
                    active: true,
                })
            }
            (FileReadAccess::Record, None) => Err("record access requires --output DIR".into()),
        }
    }

    fn display_data(&self) -> &[u8] {
        match self {
            Self::Display { data } => data,
            _ => &[],
        }
    }

    fn staging_path(&self) -> Option<&Path> {
        match self {
            Self::Display { .. } => None,
            Self::Stream { staging_path, .. } | Self::Record { staging_path, .. } => {
                Some(staging_path)
            }
        }
    }

    fn publish(&mut self) -> io::Result<()> {
        match self {
            Self::Display { .. } => Ok(()),
            Self::Stream {
                final_path,
                staging_path,
                file,
                active,
            } => {
                if path_entry_exists(final_path)? {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("output target already exists: {}", final_path.display()),
                    ));
                }
                if let Some(mut staging_file) = file.take() {
                    staging_file.flush()?;
                }
                fs::rename(&*staging_path, &*final_path)?;
                *active = false;
                Ok(())
            }
            Self::Record {
                final_path,
                staging_path,
                active,
            } => {
                if path_entry_exists(final_path)? {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("output target already exists: {}", final_path.display()),
                    ));
                }
                fs::rename(&*staging_path, &*final_path)?;
                *active = false;
                Ok(())
            }
        }
    }

    fn cleanup(&mut self) -> io::Result<()> {
        match self {
            Self::Display { .. } => Ok(()),
            Self::Stream {
                staging_path,
                file,
                active,
                ..
            } => {
                file.take();
                if !*active {
                    return Ok(());
                }
                let result = remove_file_if_present(staging_path);
                *active = false;
                result
            }
            Self::Record {
                staging_path,
                active,
                ..
            } => {
                if !*active {
                    return Ok(());
                }
                let result = remove_directory_if_present(staging_path);
                *active = false;
                result
            }
        }
    }

    fn cleanup_error(&mut self, error: BoxError) -> BoxError {
        let retained = self.staging_path().map(Path::to_path_buf);
        match self.cleanup() {
            Ok(()) => error,
            Err(cleanup_error) => format!(
                "{error}; cleanup failed: {cleanup_error}; retained staging path: {}",
                retained.as_deref().map_or_else(
                    || "<unknown>".to_string(),
                    |path| path.display().to_string()
                )
            )
            .into(),
        }
    }
}

impl Drop for FileReadDestination {
    fn drop(&mut self) {
        let retained = self.staging_path().map(Path::to_path_buf);
        if let Err(error) = self.cleanup() {
            eprintln!(
                "Error: cleanup failed during file-read cancellation: {error}; retained staging path: {}",
                retained
                    .as_deref()
                    .map_or_else(|| "<unknown>".to_string(), |path| path.display().to_string())
            );
        }
    }
}

impl FileReadSink for FileReadDestination {
    fn write_stream(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            Self::Display { data: displayed } => {
                displayed.extend_from_slice(data);
                Ok(())
            }
            Self::Stream { file, .. } => file
                .as_mut()
                .ok_or_else(|| io::Error::other("stream staging file is closed"))?
                .write_all(data),
            Self::Record { .. } => Err(io::Error::other(
                "stream payload received for record destination",
            )),
        }
    }

    fn write_record(&mut self, index: i32, data: &[u8]) -> io::Result<()> {
        match self {
            Self::Record { staging_path, .. } => {
                let path = staging_path.join(record_file_name(index));
                let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
                file.write_all(data)
            }
            _ => Err(io::Error::other(
                "record payload received without a record destination",
            )),
        }
    }
}

fn record_file_name(index: i32) -> String {
    format!("record-{index:010}.bin")
}

fn output_parent_and_name(path: &Path) -> Result<(&Path, String), BoxError> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("output target has no file name: {}", path.display()))?
        .to_string_lossy()
        .into_owned();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    Ok((parent, name))
}

fn staging_candidate(path: &Path, attempt: u64, directory: bool) -> Result<PathBuf, BoxError> {
    let (parent, name) = output_parent_and_name(path)?;
    let kind = if directory { "dir" } else { "file" };
    Ok(parent.join(format!(
        ".{name}.bacnet-read-{kind}-{}-{}-{attempt}.staging",
        std::process::id(),
        STAGING_ID.fetch_add(1, Ordering::Relaxed)
    )))
}

fn create_staging_file(final_path: &Path) -> Result<(PathBuf, File), BoxError> {
    for attempt in 0..128 {
        let candidate = staging_candidate(final_path, attempt, false)?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err("unable to allocate a unique sibling staging file".into())
}

fn create_staging_directory(final_path: &Path) -> Result<PathBuf, BoxError> {
    for attempt in 0..128 {
        let candidate = staging_candidate(final_path, attempt, true)?;
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err("unable to allocate a unique sibling staging directory".into())
}

fn refuse_existing_target(path: &Path) -> Result<(), BoxError> {
    if path_entry_exists(path)? {
        return Err(format!("output target already exists: {}", path.display()).into());
    }
    Ok(())
}

fn path_entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_directory_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_file_read_options(
    access: FileReadAccess,
    start: i32,
    count: u32,
    output: Option<&Path>,
) -> Result<(), BoxError> {
    if start < 0 {
        return Err("--start must be non-negative".into());
    }
    if count == 0 {
        return Err("--count must be greater than zero".into());
    }
    if access == FileReadAccess::Record && output.is_none() {
        return Err("record access requires --output DIR".into());
    }
    Ok(())
}

fn checked_next_cursor(returned_start: i32, returned_count: usize) -> Result<i32, BoxError> {
    if returned_start < 0 {
        return Err("AtomicReadFile ACK returned a negative start position".into());
    }
    let delta = i32::try_from(returned_count)
        .map_err(|_| "AtomicReadFile ACK next cursor is out of range")?;
    returned_start
        .checked_add(delta)
        .ok_or_else(|| "AtomicReadFile ACK next cursor is out of range".into())
}

async fn retrieve_windows<S, F, Fut>(
    access: FileReadAccess,
    start: i32,
    count: u32,
    sink: &mut S,
    mut fetch: F,
) -> Result<FileReadSummary, BoxError>
where
    S: FileReadSink,
    F: FnMut(FileAccessMethod) -> Fut,
    Fut: Future<Output = Result<AtomicReadFileAck, Error>>,
{
    validate_file_read_options(access, start, count, Some(Path::new("validated")))?;
    let mut cursor = start;
    let mut summary = FileReadSummary::default();

    loop {
        let request = match access {
            FileReadAccess::Stream => FileAccessMethod::Stream {
                file_start_position: cursor,
                requested_octet_count: count,
            },
            FileReadAccess::Record => FileAccessMethod::Record {
                file_start_record: cursor,
                requested_record_count: count,
            },
        };
        let ack = fetch(request).await?;
        summary.windows = summary
            .windows
            .checked_add(1)
            .ok_or("AtomicReadFile window count overflow")?;

        match (access, ack.access) {
            (
                FileReadAccess::Stream,
                FileReadAckMethod::Stream {
                    file_start_position,
                    file_data,
                },
            ) => {
                if file_data.len() > count as usize {
                    return Err("AtomicReadFile ACK exceeds the requested octet window".into());
                }
                let next = checked_next_cursor(file_start_position, file_data.len())?;
                if !ack.end_of_file && (file_data.is_empty() || next <= cursor) {
                    return Err("AtomicReadFile stream ACK made no forward progress".into());
                }
                sink.write_stream(&file_data)?;
                summary.octets = summary
                    .octets
                    .checked_add(file_data.len() as u64)
                    .ok_or("AtomicReadFile octet count overflow")?;
                if ack.end_of_file {
                    break;
                }
                cursor = next;
            }
            (
                FileReadAccess::Record,
                FileReadAckMethod::Record {
                    file_start_record,
                    returned_record_count,
                    file_record_data,
                },
            ) => {
                if returned_record_count as usize != file_record_data.len() {
                    return Err("AtomicReadFile ACK record cardinality mismatch".into());
                }
                if returned_record_count > count {
                    return Err("AtomicReadFile ACK exceeds the requested record window".into());
                }
                let next = checked_next_cursor(file_start_record, file_record_data.len())?;
                if !ack.end_of_file && (file_record_data.is_empty() || next <= cursor) {
                    return Err("AtomicReadFile record ACK made no forward progress".into());
                }
                let mut indexes = Vec::with_capacity(file_record_data.len());
                for offset in 0..file_record_data.len() {
                    indexes.push(checked_next_cursor(file_start_record, offset)?);
                }
                let window_octets = file_record_data.iter().try_fold(0u64, |total, record| {
                    total
                        .checked_add(record.len() as u64)
                        .ok_or("AtomicReadFile record octet count overflow")
                })?;
                for (index, record) in indexes.into_iter().zip(&file_record_data) {
                    sink.write_record(index, record)?;
                }
                summary.records = summary
                    .records
                    .checked_add(u64::from(returned_record_count))
                    .ok_or("AtomicReadFile record count overflow")?;
                summary.octets = summary
                    .octets
                    .checked_add(window_octets)
                    .ok_or("AtomicReadFile octet count overflow")?;
                if ack.end_of_file {
                    break;
                }
                cursor = next;
            }
            _ => {
                return Err("AtomicReadFile ACK access method does not match the request".into());
            }
        }
    }

    Ok(summary)
}

async fn read_file_with<F, Fut>(
    access: FileReadAccess,
    start: i32,
    count: u32,
    output: Option<&Path>,
    fetch: F,
) -> Result<FileReadSummary, BoxError>
where
    F: FnMut(FileAccessMethod) -> Fut,
    Fut: Future<Output = Result<AtomicReadFileAck, Error>>,
{
    validate_file_read_options(access, start, count, output)?;
    let mut destination = FileReadDestination::create(access, output)?;
    let result = retrieve_windows(access, start, count, &mut destination, fetch).await;
    let mut summary = match result {
        Ok(summary) => summary,
        Err(error) => return Err(destination.cleanup_error(error)),
    };
    summary.display_data = destination.display_data().to_vec();
    if let Err(error) = destination.publish() {
        return Err(destination.cleanup_error(error.into()));
    }
    Ok(summary)
}

/// Read a complete file from a remote device via successive AtomicReadFile ACKs.
pub async fn file_read_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    file_instance: u32,
    access: FileReadAccess,
    start_position: i32,
    count: u32,
    output_path: Option<&str>,
    format: OutputFormat,
) -> Result<(), BoxError> {
    let output_path = output_path.map(Path::new);
    validate_file_read_options(access, start_position, count, output_path)?;
    let file_oid = ObjectIdentifier::new(ObjectType::FILE, file_instance)?;
    let summary = read_file_with(access, start_position, count, output_path, |request| {
        client.atomic_read_file_decoded(mac, file_oid, request)
    })
    .await?;

    match (access, output_path) {
        (FileReadAccess::Stream, Some(path)) => output::print_success(
            &format!("Wrote {} bytes to {}", summary.octets, path.display()),
            format,
        ),
        (FileReadAccess::Stream, None) => {
            let hex = summary
                .display_data
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            output::print_success(&format!("Read {} bytes: {hex}", summary.octets), format);
        }
        (FileReadAccess::Record, Some(path)) => output::print_success(
            &format!(
                "Wrote {} records ({} bytes) to {}",
                summary.records,
                summary.octets,
                path.display()
            ),
            format,
        ),
        (FileReadAccess::Record, None) => unreachable!("validated record output"),
    }
    Ok(())
}

/// Write a file to a remote device via AtomicWriteFile (stream access).
pub async fn file_write_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    file_instance: u32,
    start_position: i32,
    input_path: &str,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let file_oid = ObjectIdentifier::new(ObjectType::FILE, file_instance)?;

    let file_data = std::fs::read(input_path)?;
    let data_len = file_data.len();

    let access = FileWriteAccessMethod::Stream {
        file_start_position: start_position,
        file_data,
    };

    client.atomic_write_file(mac, file_oid, access).await?;

    output::print_success(&format!("Wrote {data_len} bytes from {input_path}"), format);
    Ok(())
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
