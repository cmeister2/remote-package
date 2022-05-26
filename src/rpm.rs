//! Support for remote RPM packages
use fez::RPMPackage;

use crate::{PkgError, RemotePackage};

/// A structure representing a remote RPM package.
#[derive(Debug)]
pub struct RpmRemotePackage {
    package: RPMPackage,
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
        let response = client.get(url).send()?;

        let mut buf_response = std::io::BufReader::new(response);

        // blocking::Response impls Read, so we can pass it to rpm-rs.
        let package = fez::RPMPackage::parse(&mut buf_response)?;

        Ok(Self { package })
    }
}

impl RemotePackage for RpmRemotePackage {
    fn package_name(&self) -> Result<&str, PkgError> {
        Ok(self.package.metadata.header.get_name()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "http")]
    #[test]
    fn test_package() {
        // Pick a random small package from Centos 9
        let url = "http://mirror.stream.centos.org/9-stream/AppStream/x86_64/os/Packages/khmer-os-muol-fonts-all-5.0-36.el9.noarch.rpm";

        let package = RpmRemotePackage::new_from_url(url).expect("Failed to download package");
        assert_eq!(package.package_name().unwrap(), "khmer-os-muol-fonts-all");
    }
}
