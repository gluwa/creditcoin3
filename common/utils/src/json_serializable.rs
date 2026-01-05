//! JSON serialization utilities for file I/O operations.
//!
//! This module provides a trait for objects that can be serialized to and from
//! JSON files with proper file locking to ensure safe concurrent access.

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

/// Trait for types that can be serialized to and from JSON files.
///
/// This trait provides convenient methods for saving objects to JSON files
/// and loading them back, with proper file locking to prevent corruption
/// during concurrent access.
///
/// # Safety
///
/// File operations use exclusive locking for writes and shared locking for reads
/// to ensure data integrity in multi-process environments.
///
/// # Example
///
/// ```rust
/// use serde::{Deserialize, Serialize};
/// use utils::JsonSerializable;
///
/// #[derive(Serialize, Deserialize, PartialEq, Debug)]
/// struct Config {
///     name: String,
///     version: u32,
/// }
///
/// impl JsonSerializable for Config {}
///
/// # fn main() -> anyhow::Result<()> {
/// let config = Config {
///     name: "test".to_string(),
///     version: 1,
/// };
///
/// // Save to file
/// config.to_file("config.json")?;
///
/// // Load from file
/// let loaded_config = Config::try_from_file("config.json")?;
/// assert_eq!(config, loaded_config);
/// # Ok(())
/// # }
/// ```
pub trait JsonSerializable: Sized + Serialize + for<'de> Deserialize<'de> {
    /// Saves the object to a JSON file with pretty formatting.
    ///
    /// This method creates or overwrites the specified file with the JSON
    /// representation of the object. The file is locked exclusively during
    /// the write operation to prevent corruption from concurrent access.
    ///
    /// # Arguments
    ///
    /// * `path` - The file path where the JSON should be saved
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the operation succeeds
    /// * `Err(anyhow::Error)` if file creation, locking, or serialization fails
    ///
    /// # Errors
    ///
    /// This method can fail if:
    /// - The file cannot be created or opened for writing
    /// - File locking fails (e.g., if another process has the file locked)
    /// - JSON serialization fails
    /// - Writing to the file fails
    /// - Flushing the buffer fails
    fn to_file<P: AsRef<Path>>(&self, path: P) -> anyhow::Result<()> {
        let file = File::create(path.as_ref()).map_err(|e| {
            anyhow::anyhow!("Failed to create file '{}': {}", path.as_ref().display(), e)
        })?;

        file.lock_exclusive().map_err(|e| {
            anyhow::anyhow!("Failed to lock file '{}': {}", path.as_ref().display(), e)
        })?;

        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, self)
            .map_err(|e| anyhow::anyhow!("Failed to serialize to JSON: {e}"))?;

        writer.flush().map_err(|e| {
            anyhow::anyhow!("Failed to flush file '{}': {}", path.as_ref().display(), e)
        })?;

        Ok(())
    }

    /// Loads an object from a JSON file.
    ///
    /// This method opens the specified file, locks it for shared reading,
    /// and deserializes the JSON content into an object of the implementing type.
    ///
    /// # Arguments
    ///
    /// * `path` - The file path to read from
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` containing the deserialized object if successful
    /// * `Err(anyhow::Error)` if file opening, locking, or deserialization fails
    ///
    /// # Errors
    ///
    /// This method can fail if:
    /// - The file does not exist or cannot be opened
    /// - File locking fails
    /// - The file contains invalid JSON
    /// - The JSON structure doesn't match the expected type
    /// - I/O errors occur during reading
    fn try_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let file = File::open(path.as_ref()).map_err(|e| {
            anyhow::anyhow!("Failed to open file '{}': {}", path.as_ref().display(), e)
        })?;

        FileExt::lock_shared(&file).map_err(|e| {
            anyhow::anyhow!("Failed to lock file '{}': {}", path.as_ref().display(), e)
        })?;

        let reader = BufReader::new(file);
        serde_json::from_reader(reader).map_err(|e| {
            anyhow::anyhow!(
                "Failed to deserialize JSON from '{}': {}",
                path.as_ref().display(),
                e
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::NamedTempFile;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct TestData {
        name: String,
        value: i32,
        enabled: bool,
    }

    impl JsonSerializable for TestData {}

    #[test]
    fn test_basic_serialization() -> anyhow::Result<()> {
        let temp_file = NamedTempFile::new()?;
        let path = temp_file.path();

        let original = TestData {
            name: "test".to_string(),
            value: 42,
            enabled: true,
        };

        // Save to file
        original.to_file(path)?;

        // Load from file
        let loaded = TestData::try_from_file(path)?;

        assert_eq!(original, loaded);
        Ok(())
    }

    #[test]
    fn test_error_handling() {
        // Test loading from non-existent file
        let result = TestData::try_from_file("/nonexistent/path/file.json");
        assert!(result.is_err());

        // Test saving to invalid path (assuming root permissions are not available)
        let data = TestData {
            name: "error_test".to_string(),
            value: 0,
            enabled: false,
        };

        let result = data.to_file("/root/restricted/file.json");
        // This should fail on most systems without root privileges
        assert!(result.is_err());
    }
}
