//! Support for remote RPM packages
use std::io::{BufReader, Read};

use ::rpm::PackageMetadata;

use crate::{PkgError, RemotePackage};

/// A structure representing a remote RPM package.
pub struct RpmRemotePackage {
    /// The package headers, without its payload.
    metadata: PackageMetadata,
}

impl RpmRemotePackage {
    /// Attempts to create an `RpmRemotePackage` from a URL.
    ///
    /// Uses a blocking HTTP client. In an async environment, call this from
    /// `tokio::task::spawn_blocking`.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the server returns an error
    /// status, or the RPM metadata cannot be read or parsed.
    #[cfg(feature = "http")]
    pub fn new_from_url(url: &str) -> Result<Self, PkgError> {
        let client = reqwest::blocking::Client::new();

        // Send an HTTP request for the package and get the Response.
        let response = client
            .get(url)
            .timeout(std::time::Duration::from_secs(10))
            .send()?
            .error_for_status()?;

        // blocking::Response impls Read, so we can pass it to new_from_read.
        Self::new_from_read(response)
    }

    /// Attempts to create a `RpmRemotePackage` from something that impls
    /// Read.
    ///
    /// Reads only the RPM metadata, leaving the package payload unread.
    ///
    /// # Errors
    ///
    /// Returns an error if the reader fails or the RPM metadata is invalid or
    /// truncated.
    pub fn new_from_read<R: Read>(reader: R) -> Result<Self, PkgError> {
        // rpm-rs requires BufRead but parses metadata using exact reads. A
        // one-byte buffer lets each nonempty read bypass buffering, preventing
        // a larger buffer from fetching bytes from the package payload.
        let mut reader = BufReader::with_capacity(1, reader);
        let metadata = PackageMetadata::parse(&mut reader)?;

        Ok(Self { metadata })
    }
}

impl RemotePackage for RpmRemotePackage {
    fn package_type(&self) -> crate::RemotePackageType {
        crate::RemotePackageType::Rpm
    }

    fn package_name(&self) -> Result<&str, PkgError> {
        Ok(self.metadata.get_name()?)
    }

    fn package_version(&self) -> Result<&str, PkgError> {
        Ok(self.metadata.get_version()?)
    }

    /// For RPM, the package iteration is the release.
    fn package_iteration(&self) -> Option<&str> {
        self.metadata.get_release().ok()
    }

    fn package_arch(&self) -> Result<&str, PkgError> {
        Ok(self.metadata.get_arch()?)
    }
}

#[cfg(test)]
/// Regression tests for metadata-only RPM parsing.
mod tests {
    use std::io;

    use super::*;

    /// Build a valid RPM and return its bytes and metadata boundary.
    fn package_fixture() -> (Vec<u8>, usize) {
        let package =
            ::rpm::PackageBuilder::new("metadata-test", "1.2.3", "MIT", "x86_64", "fixture")
                .release("4")
                .build()
                .unwrap();
        assert!(!package.payload.is_empty());
        let mut bytes = Vec::new();
        package.write(&mut bytes).unwrap();
        let metadata_length = bytes.len() - package.payload.len();
        (bytes, metadata_length)
    }

    /// Missing bytes at each metadata section propagate as unexpected EOF.
    #[test]
    fn propagates_truncated_metadata_errors() {
        let (bytes, metadata_length) = package_fixture();
        for length in [0, 1, 95, 96, 97, metadata_length - 1] {
            let error = RpmRemotePackage::new_from_read(&bytes[..length])
                .err()
                .expect("Truncated RPM metadata must return an error");
            match error {
                PkgError::RpmError(::rpm::Error::Io(error)) => {
                    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof)
                }
                other => panic!("Unexpected package error: {other:?}"),
            }
        }
    }

    /// Metadata parsing succeeds even when the stream ends before the payload.
    #[test]
    fn accepts_metadata_without_payload() {
        let (bytes, metadata_length) = package_fixture();
        let package = RpmRemotePackage::new_from_read(&bytes[..metadata_length]).unwrap();
        assert_eq!(package.package_name().unwrap(), "metadata-test");
        assert_eq!(package.package_iteration(), Some("4"));
    }

    /// Blocking readers need no runtime, Unpin bound, or payload download.
    #[test]
    fn reads_metadata_inside_an_executor_without_reading_payload_or_requiring_unpin() {
        use std::{cell::Cell, io::Cursor, marker::PhantomPinned, rc::Rc};

        let (bytes, metadata_length) = package_fixture();

        /// A non-Unpin reader that rejects requests extending into the payload.
        struct MetadataReader {
            /// The complete package, including its payload.
            bytes: Cursor<Vec<u8>>,
            /// First offset containing package payload bytes.
            metadata_length: usize,
            /// Observed stream position after metadata parsing.
            bytes_read: Rc<Cell<usize>>,
            /// Prevents an accidental Unpin requirement in the public API.
            _pinned: PhantomPinned,
        }

        impl Read for MetadataReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let remaining = self.metadata_length - self.bytes.position() as usize;
                assert!(
                    buffer.len() <= remaining,
                    "RPM metadata parsing tried to read the payload"
                );
                let read = self.bytes.read(buffer)?;
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
        assert_eq!(metadata.package_iteration(), Some("4"));
        assert!(matches!(
            metadata.package_type(),
            crate::RemotePackageType::Rpm
        ));
        assert_eq!(bytes_read.get(), metadata_length);
    }

    /// Direct HTTP queries parse packages served by a local fixture server.
    #[cfg(feature = "http")]
    #[test]
    fn test_package() {
        let (url, server) = crate::tests::serve_package(crate::tests::rpm_package_bytes());
        let package = RpmRemotePackage::new_from_url(&url).expect("Failed to download package");
        assert_eq!(package.package_name().unwrap(), "metadata-test");
        server.join().unwrap();
    }
}
