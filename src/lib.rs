//! A simple crate to query remote packages for information.
#![deny(
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results
)]

/// Types of remote package.
pub enum RemotePackageType {
    #[cfg(feature = "debian")]
    /// Debian package
    Deb,

    #[cfg(feature = "rpm")]
    /// RPM package
    Rpm,
}

/// Error type for this crate.
#[derive(thiserror::Error, Debug)]
pub enum PkgError {
    /// An error from the underlying Debian package library
    #[cfg(feature = "debian")]
    #[error("Debpkg Error")]
    DebPkgError(#[from] debpkg::Error),

    /// An error from the underlying RPM package library
    #[cfg(feature = "rpm")]
    #[error("RPM error: {0}")]
    RpmError(#[from] ::rpm::Error),

    /// An error from the underlying HTTP client library
    #[cfg(feature = "http")]
    #[error("HTTP Error")]
    HTTPError(#[from] reqwest::Error),

    /// Failure to infer package type
    #[error("Failed to infer package type")]
    InferError,

    /// Field not found in Debian control data.
    #[cfg(feature = "debian")]
    #[error("Debian field not found: {0}")]
    DebianControlFieldNotFound(String),

    /// Package type can't be queried.
    #[error("Package type cannot be queried (inferred: {0})")]
    UnknownPackageType(String),
}

/// Trait representing a remote package.
///
/// All remote packages support these methods.
pub trait RemotePackage {
    /// Get the package type.
    fn package_type(&self) -> RemotePackageType;

    /// Get the package name.
    ///
    /// # Errors
    ///
    /// Returns an error if the package metadata does not contain a valid name.
    fn package_name(&self) -> Result<&str, PkgError>;

    /// Get the package version.
    ///
    /// # Errors
    ///
    /// Returns an error if the package metadata does not contain a valid version.
    fn package_version(&self) -> Result<&str, PkgError>;

    /// Get the package iteration. Different package types get this from
    /// different places.
    fn package_iteration(&self) -> Option<&str>;

    /// Get the package architecture.
    ///
    /// # Errors
    ///
    /// Returns an error if the package metadata does not contain a valid architecture.
    fn package_arch(&self) -> Result<&str, PkgError>;
}

// Include Debian package support
#[cfg(feature = "debian")]
pub mod debian;

// Include RPM package support
#[cfg(feature = "rpm")]
pub mod rpm;

/// Create a RemotePackage from a URL.
///
/// Uses a blocking HTTP client. In an async environment, call this from
/// `tokio::task::spawn_blocking`.
///
/// # Errors
///
/// Returns an error if the request fails, the response has an HTTP error status,
/// the package type cannot be detected or is disabled, or its metadata is invalid.
#[cfg(feature = "http")]
pub fn from_url(url: &str) -> Result<Box<dyn RemotePackage>, PkgError> {
    use std::io::Read;

    let client = reqwest::blocking::Client::new();

    // Send an HTTP request for the package and get the Response.
    let response = client.get(url).send()?.error_for_status()?;

    // Read the first 1024 bytes for infer.
    let mut reader = response.take(1024);
    let mut infer_buf = vec![];
    let _ = reader
        .read_to_end(&mut infer_buf)
        .map_err(|_| PkgError::InferError)?;

    // Infer uses magic to detect file types from starting bytes.
    let ext = infer::get(&infer_buf).map(|t| t.extension());
    // Using a cursor and chain allows us to reconstruct the original response.
    #[cfg(any(feature = "debian", feature = "rpm"))]
    let rsp = std::io::Cursor::new(infer_buf).chain(reader.into_inner());

    // If the feature is enabled and the package is Debian, make a Debian remote package.
    #[cfg(feature = "debian")]
    if ext == Some("deb") {
        let pkg = debian::DebianRemotePackage::new_from_read(rsp)?;
        return Ok(Box::new(pkg));
    }

    // If the feature is enabled and the package is RPM, make an RPM remote package.
    #[cfg(feature = "rpm")]
    if ext == Some("rpm") {
        let pkg = rpm::RpmRemotePackage::new_from_read(rsp)?;
        return Ok(Box::new(pkg));
    }

    // The package type was unknown or the necessary feature was disabled.
    // Return an error in either case.
    Err(PkgError::UnknownPackageType(
        ext.unwrap_or("unknown").to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    //! In-memory package fixtures and loopback HTTP integration tests.

    #[cfg(feature = "http")]
    use super::*;

    /// Build a small Debian package containing the supplied control fields.
    #[cfg(feature = "debian")]
    pub(crate) fn debian_package_bytes(control: &str) -> Vec<u8> {
        let mut control_tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(control.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        control_tar
            .append_data(&mut header, "./control", control.as_bytes())
            .unwrap();
        let control_tar = control_tar.into_inner().unwrap();

        let mut package = ar::Builder::new(Vec::new());
        package
            .append(
                &ar::Header::new(b"debian-binary".to_vec(), 4),
                &b"2.0\n"[..],
            )
            .unwrap();
        package
            .append(
                &ar::Header::new(b"control.tar".to_vec(), control_tar.len() as u64),
                control_tar.as_slice(),
            )
            .unwrap();
        package.into_inner().unwrap()
    }

    /// Build a small RPM fixture with a release distinct from its version.
    #[cfg(all(feature = "http", feature = "rpm"))]
    pub(crate) fn rpm_package_bytes() -> Vec<u8> {
        let package =
            ::rpm::PackageBuilder::new("metadata-test", "1.2.3", "MIT", "x86_64", "HTTP fixture")
                .release("4")
                .build()
                .unwrap();
        let mut bytes = Vec::new();
        package.write(&mut bytes).unwrap();
        bytes
    }

    /// Serve a fixture once from a local HTTP endpoint.
    #[cfg(feature = "http")]
    pub(crate) fn serve_package(bytes: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        serve_response("200 OK", bytes)
    }

    /// Serve a response with a chosen status, independent of external hosts.
    #[cfg(feature = "http")]
    pub(crate) fn serve_response(
        status: &'static str,
        bytes: Vec<u8>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
            time::Duration,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/package", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(
                    request.len() < 8192,
                    "HTTP request headers exceeded test limit"
                );
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(&bytes).unwrap();
        });
        (url, server)
    }

    /// Verify inferred RPM metadata through the real HTTP client.
    #[cfg(all(feature = "http", feature = "rpm"))]
    #[test]
    fn test_from_url_rpm() {
        let (url, server) = serve_package(rpm_package_bytes());
        let package = from_url(&url).unwrap();
        assert!(matches!(package.package_type(), RemotePackageType::Rpm));
        assert_eq!(package.package_name().unwrap(), "metadata-test");
        assert_eq!(package.package_version().unwrap(), "1.2.3");
        assert_eq!(package.package_arch().unwrap(), "x86_64");
        assert_eq!(package.package_iteration(), Some("4"));
        server.join().unwrap();
    }

    /// Verify inferred Debian metadata through the real HTTP client.
    #[cfg(all(feature = "http", feature = "debian"))]
    #[test]
    fn test_from_url_deb() {
        let bytes = debian_package_bytes(
            "Package: metadata-test\nVersion: 1.2.3-4\nArchitecture: all\nDescription: HTTP fixture\n",
        );
        let (url, server) = serve_package(bytes);
        let package = from_url(&url).unwrap();
        assert!(matches!(package.package_type(), RemotePackageType::Deb));
        assert_eq!(package.package_name().unwrap(), "metadata-test");
        assert_eq!(package.package_version().unwrap(), "1.2.3-4");
        assert_eq!(package.package_arch().unwrap(), "all");
        assert_eq!(package.package_iteration(), Some("4"));
        server.join().unwrap();
    }

    /// Unknown files retain a useful format error rather than a parser error.
    #[cfg(feature = "http")]
    #[test]
    fn rejects_unknown_package_type() {
        let (url, server) = serve_package(b"Not a package".to_vec());
        assert!(matches!(
            from_url(&url),
            Err(PkgError::UnknownPackageType(format)) if format == "unknown"
        ));
        server.join().unwrap();
    }

    /// HTTP failures are reported before attempting package detection.
    #[cfg(feature = "http")]
    #[test]
    fn reports_http_status_errors() {
        let (url, server) = serve_response("404 Not Found", Vec::new());
        assert!(matches!(
            from_url(&url),
            Err(PkgError::HTTPError(error)) if error.status() == Some(reqwest::StatusCode::NOT_FOUND)
        ));
        server.join().unwrap();
    }
}
