# remote-package

A simple crate to query remote packages for information.

Requires Rust 1.88 or newer and uses Rust edition 2024. RPM queries use
[`rpm-rs`](https://github.com/rpm-rs/rpm-rs) to read only package metadata, so large
package payloads do not need to be downloaded.

The default features enable HTTP, Debian, and RPM support. Disable default
features and select `http`, `debian`, and/or `rpm` to use only the support you need.

RPM errors wrap `rpm::Error` in `PkgError::RpmError`.
