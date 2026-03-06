//! File transfer commands: AtomicReadFile and AtomicWriteFile.

use bacnet_client::client::BACnetClient;
use bacnet_services::file::{FileAccessMethod, FileWriteAccessMethod};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use crate::output::{self, OutputFormat};

/// Read a file from a remote device via AtomicReadFile (stream access).
pub async fn file_read_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    file_instance: u32,
    start_position: i32,
    count: u32,
    output_path: Option<&str>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let file_oid = ObjectIdentifier::new(ObjectType::FILE, file_instance)?;

    let access = FileAccessMethod::Stream {
        file_start_position: start_position,
        requested_octet_count: count,
    };

    let response = client.atomic_read_file(mac, file_oid, access).await?;

    if let Some(path) = output_path {
        std::fs::write(path, &response)?;
        output::print_success(&format!("Wrote {} bytes to {path}", response.len()), format);
    } else {
        // Display the data.
        let hex: String = response
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        output::print_success(&format!("Read {} bytes: {hex}", response.len()), format);
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
