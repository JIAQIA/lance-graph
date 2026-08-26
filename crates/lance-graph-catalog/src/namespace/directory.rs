// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use async_trait::async_trait;
use lance_namespace::models::{DescribeTableRequest, DescribeTableResponse};
use lance_namespace::{Error as NamespaceError, LanceNamespace, Result};
use snafu::location;

/// A namespace that resolves table names relative to a base directory or URI.
#[derive(Debug, Clone, Default)]
pub struct DirNamespace {
    base_uri: String,
    /// Object-store options (e.g. AWS `endpoint`/`access_key_id`/`region`) carried
    /// through `DescribeTableResponse.storage_options` so downstream openers
    /// (`DatasetBuilder::from_namespace`) can build an explicit object store
    /// without relying on ambient AWS env (TFROB-806 / TFRobotV2).
    storage_options: Option<HashMap<String, String>>,
}

impl DirNamespace {
    /// Create a new directory-backed namespace rooted at `base_uri`.
    ///
    /// The URI is normalized so that it does not end with a trailing slash.
    /// Uses no explicit storage options (backwards-compatible): downstream opening
    /// falls back to ambient env (AWS_*) resolution.
    pub fn new(base_uri: impl Into<String>) -> Self {
        Self::new_with_storage_options(base_uri, None)
    }

    /// Create a directory-backed namespace with explicit object-store options.
    ///
    /// The options are returned by `describe_table` under
    /// [`DescribeTableResponse::storage_options`], and merged by
    /// `DatasetBuilder::from_namespace` when opening the datasets. This enables
    /// fully-explicit remote storage configuration (MinIO/S3 custom endpoint)
    /// with zero ambient AWS env, e.g. from `TFRobotV2`'s `storage_options` field.
    pub fn new_with_storage_options(
        base_uri: impl Into<String>,
        storage_options: Option<HashMap<String, String>>,
    ) -> Self {
        let uri = base_uri.into();
        let clean_uri = uri.trim_end_matches('/').to_string();
        Self {
            base_uri: clean_uri,
            storage_options,
        }
    }

    /// Return the normalized base URI.
    pub fn base_uri(&self) -> &str {
        &self.base_uri
    }
}

#[async_trait]
impl LanceNamespace for DirNamespace {
    fn namespace_id(&self) -> String {
        format!("DirNamespace {{ base_uri: '{}' }}", self.base_uri)
    }

    async fn describe_table(&self, request: DescribeTableRequest) -> Result<DescribeTableResponse> {
        let id = request.id.ok_or_else(|| {
            NamespaceError::invalid_input(
                "DirNamespace requires the table identifier to be provided",
                location!(),
            )
        })?;

        if id.len() != 1 {
            return Err(NamespaceError::invalid_input(
                format!(
                    "DirNamespace expects identifiers with a single component, got {:?}",
                    id
                ),
                location!(),
            ));
        }

        let table_name = &id[0];
        let location = format!("{}/{}.lance", self.base_uri, table_name);

        let mut response = DescribeTableResponse::new();
        response.location = Some(location);
        // Explicit object-store options are the source of truth when provided:
        // downstream openers merge them over any user-provided options. `None`
        // keeps the legacy behavior (ambient env resolution).
        response.storage_options = self.storage_options.clone();
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn describe_table_returns_clean_location() {
        let namespace = DirNamespace::new("s3://bucket/path/");
        let mut request = DescribeTableRequest::new();
        request.id = Some(vec!["users".to_string()]);

        let response = namespace.describe_table(request).await.unwrap();
        assert_eq!(
            response.location.as_deref(),
            Some("s3://bucket/path/users.lance")
        );
    }

    #[tokio::test]
    async fn describe_table_carries_storage_options() {
        let opts = HashMap::from([
            ("endpoint".to_string(), "http://minio:9000".to_string()),
            ("region".to_string(), "us-east-1".to_string()),
        ]);
        let namespace = DirNamespace::new_with_storage_options("s3://bucket/path", Some(opts));
        let mut request = DescribeTableRequest::new();
        request.id = Some(vec!["users".to_string()]);

        let response = namespace.describe_table(request).await.unwrap();
        let carried = response.storage_options.expect("storage_options carried");
        assert_eq!(carried.get("endpoint").map(String::as_str), Some("http://minio:9000"));
        assert_eq!(carried.get("region").map(String::as_str), Some("us-east-1"));
    }

    #[tokio::test]
    async fn describe_table_defaults_to_no_storage_options() {
        let namespace = DirNamespace::new("s3://bucket/path");
        let mut request = DescribeTableRequest::new();
        request.id = Some(vec!["users".to_string()]);

        let response = namespace.describe_table(request).await.unwrap();
        assert!(response.storage_options.is_none());
    }

    #[tokio::test]
    async fn describe_table_rejects_missing_identifier() {
        let namespace = DirNamespace::new("file:///tmp");
        let request = DescribeTableRequest::new();

        let err = namespace.describe_table(request).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("DirNamespace requires the table identifier"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn describe_table_rejects_multi_component_identifier() {
        let namespace = DirNamespace::new("memory://namespace");
        let mut request = DescribeTableRequest::new();
        request.id = Some(vec!["foo".into(), "bar".into()]);

        let err = namespace.describe_table(request).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("expects identifiers with a single component"),
            "unexpected error: {err}"
        );
    }
}
