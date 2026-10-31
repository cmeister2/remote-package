//! Support for remote RPM packages
use std::{future::Future, io::Read};

use fez::RPMPackageMetadata;
use futures::{io::AllowStdIo, FutureExt};
use tokio_util::compat::FuturesAsyncReadCompatExt;

use crate::{PkgError, RemotePackage};

/// A structure representing a remote RPM package.
pub struct RpmRemotePackage {
    metadata: RPMPackageMetadata,
}

// The synchronous Read API must finish in one poll, including when called from
// an existing executor. Keep that policy separate from Fez's metadata parsing.
fn finish_metadata_parse(
    parse: impl Future<Output = Result<RPMPackageMetadata, fez::RPMError>>,
) -> Result<RPMPackageMetadata, fez::RPMError> {
    parse.now_or_never().ok_or_else(|| {
        fez::RPMError::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "RPM metadata parser unexpectedly suspended on a blocking reader",
        ))
    })?
}

impl RpmRemotePackage {
    /// Attempts to create an `RpmRemotePackage` from a URL.
    ///
    /// Uses a blocking tokio client to download the remote package - if
    /// using this in an async environment, surround this with tokio::spawn_blocking.
    #[cfg(feature = "http")]
    pub fn new_from_url(url: &str) -> Result<Self, PkgError> {
        let client = reqwest::blocking::Client::new();

        // Send an HTTP request for the package and get the Response.
        let response = client
            .get(url)
            .timeout(std::time::Duration::from_secs(10))
            .send()?;

        // blocking::Response impls Read, so we can pass it to new_from_read.
        Self::new_from_read(response)
    }

    /// Attempts to create a `RpmRemotePackage` from something that impls
    /// Read.
    pub fn new_from_read<R: Read>(reader: R) -> Result<Self, PkgError> {
        // Box preserves the public Read-only bound for readers that are not Unpin.
        // The compatibility adapter performs exact metadata reads without a Tokio
        // runtime or a buffering layer that could fetch the package payload.
        let mut reader = AllowStdIo::new(Box::new(reader)).compat();
        // Fez's metadata parser only awaits exact reads. AllowStdIo always
        // returns Ready, and Tokio's read_exact adds no cooperative suspension.
        // Polling once also preserves use inside an existing async executor.
        let metadata = finish_metadata_parse(RPMPackageMetadata::parse_async(&mut reader))?;

        Ok(Self { metadata })
    }
}

impl RemotePackage for RpmRemotePackage {
    fn package_type(&self) -> crate::RemotePackageType {
        crate::RemotePackageType::Rpm
    }

    fn package_name(&self) -> Result<&str, PkgError> {
        Ok(self.metadata.header.get_name()?)
    }

    fn package_version(&self) -> Result<&str, PkgError> {
        Ok(self.metadata.header.get_version()?)
    }

    /// For RPM, the package iteration is the release.
    fn package_iteration(&self) -> Option<&str> {
        self.metadata.header.get_release().ok()
    }

    fn package_arch(&self) -> Result<&str, PkgError> {
        Ok(self.metadata.header.get_arch()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_metadata_parser_that_suspends_without_polling_again() {
        use std::{cell::Cell, task::Poll};

        let polls = Cell::new(0);
        let parse = futures::future::poll_fn(|_| {
            polls.set(polls.get() + 1);
            Poll::Pending
        });
        let error =
            finish_metadata_parse(parse).expect_err("A suspended parser must return an error");
        match error {
            fez::RPMError::Io(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock)
            }
            other => panic!("Unexpected parser error: {:?}", other),
        }
        assert_eq!(polls.get(), 1);
    }

    #[test]
    fn propagates_truncated_metadata_errors() {
        let reader = std::io::Cursor::new(Vec::<u8>::new());
        let error = RpmRemotePackage::new_from_read(reader)
            .err()
            .expect("Truncated RPM metadata must return an error");
        match error {
            PkgError::RpmError(fez::RPMError::Io(error)) => {
                assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof)
            }
            other => panic!("Unexpected package error: {:?}", other),
        }
    }

    #[test]
    fn reads_metadata_inside_an_executor_without_reading_payload_or_requiring_unpin() {
        use std::{cell::Cell, io::Cursor, marker::PhantomPinned, rc::Rc};

        let package = fez::RPMBuilder::new("metadata-test", "1.2.3", "MIT", "x86_64", "fixture")
            .build()
            .unwrap();
        assert!(!package.content.is_empty());
        let mut bytes = Vec::new();
        package.write(&mut bytes).unwrap();
        let metadata_length = bytes.len() - package.content.len();

        struct MetadataReader {
            bytes: Cursor<Vec<u8>>,
            metadata_length: usize,
            bytes_read: Rc<Cell<usize>>,
            _pinned: PhantomPinned,
        }

        impl Read for MetadataReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let remaining = self.metadata_length - self.bytes.position() as usize;
                assert!(
                    remaining > 0,
                    "RPM metadata parsing tried to read the payload"
                );
                let length = buffer.len().min(remaining);
                let read = self.bytes.read(&mut buffer[..length])?;
                self.bytes_read.set(self.bytes.position() as usize);
                Ok(read)
            }
        }

        let bytes_read = Rc::new(Cell::new(0));
        let reader = MetadataReader {
            bytes: Cursor::new(bytes),
            metadata_length,
            bytes_read: Rc::clone(&bytes_read),
            _pinned: PhantomPinned,
        };
        let metadata =
            futures::executor::block_on(
                async move { RpmRemotePackage::new_from_read(reader).unwrap() },
            );
        assert_eq!(metadata.package_name().unwrap(), "metadata-test");
        assert_eq!(metadata.package_version().unwrap(), "1.2.3");
        assert_eq!(metadata.package_arch().unwrap(), "x86_64");
        assert_eq!(bytes_read.get(), metadata_length);
    }

    #[cfg(feature = "http")]
    #[test]
    fn test_package() {
        // Kibana is huge but should finish very quickly if just getting metadata.
        let url = "https://artifacts.elastic.co/downloads/kibana/kibana-8.2.1-x86_64.rpm";

        let package = RpmRemotePackage::new_from_url(url).expect("Failed to download package");
        assert_eq!(package.package_name().unwrap(), "kibana");
    }
}
