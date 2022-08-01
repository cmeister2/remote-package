//! Support for remote RPM packages
use std::io::Read;

use fez::{RPMPackageMetadata, RpmPkgReader};

use crate::{PkgError, RemotePackage};

/// A structure representing a remote RPM package.
pub struct RpmRemotePackage {
    metadata: RPMPackageMetadata,
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
        let mut package = RpmPkgReader::parse(reader)?;
        let metadata = package.metadata()?;

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

    #[cfg(feature = "http")]
    #[test]
    fn test_package() {
        // Kibana is huge but should finish very quickly if just getting metadata.
        let url = "https://artifacts.elastic.co/downloads/kibana/kibana-8.2.1-x86_64.rpm";

        let package = RpmRemotePackage::new_from_url(url).expect("Failed to download package");
        assert_eq!(package.package_name().unwrap(), "kibana");
    }
}
