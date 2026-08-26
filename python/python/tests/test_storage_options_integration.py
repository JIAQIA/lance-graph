# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: Copyright The Lance Authors

"""Integration tests for DirNamespace explicit storage_options.

Verifies the TFROB-806 / TFRobotV2 contract end-to-end: a ``DirNamespace``
built with fully explicit storage options (endpoint + credentials as the only
credential source) reaches the object store with **zero ambient AWS env**.

Requires a running S3-compatible object store (e.g. MinIO) configured via:
``LANCE_STORAGE_ENDPOINT``, ``LANCE_STORAGE_ACCESS_KEY``,
``LANCE_STORAGE_SECRET_KEY`` (optional ``LANCE_STORAGE_REGION``). Skipped
otherwise. The test also refuses to run if any ``AWS_*`` env var is set, so a
going-green run proves the explicit-options path, not ambient credentials.
"""

import os

import pytest
from lance_graph import CypherQuery, DirNamespace, GraphConfig

ENDPOINT = os.environ.get("LANCE_STORAGE_ENDPOINT")
ACCESS_KEY = os.environ.get("LANCE_STORAGE_ACCESS_KEY")
SECRET_KEY = os.environ.get("LANCE_STORAGE_SECRET_KEY")
REGION = os.environ.get("LANCE_STORAGE_REGION", "us-east-1")

pytestmark = [pytest.mark.integration]

requires_minio = pytest.mark.skipif(
    ENDPOINT is None or ACCESS_KEY is None or SECRET_KEY is None,
    reason="LANCE_STORAGE_ENDPOINT/ACCESS_KEY/SECRET_KEY not configured",
)

AMBIENT_AWS_VARS = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
]


@requires_minio
def test_execute_with_namespace_explicit_storage_options_zero_aws_env():
    """Query through DirNamespace with explicit options, no ambient AWS env."""
    # The explicit config below must be the only credential source in scope.
    ambient = [v for v in AMBIENT_AWS_VARS if os.environ.get(v)]
    assert not ambient, (
        f"ambient AWS env detected ({ambient}): cannot prove zero-env reach"
    )

    import pyarrow as pa

    write_dataset = pytest.importorskip("lance").write_dataset

    bucket = "lance-storage-options-test"
    uri = f"s3://{bucket}"
    # The endpoint is http (MinIO): lance requires allow_http for that.
    storage_options = {
        "endpoint": ENDPOINT,
        "access_key_id": ACCESS_KEY,
        "secret_access_key": SECRET_KEY,
        "region": REGION,
        "allow_http": "true",
    }

    # Write with lance's own explicit options: isolates the read side (our
    # DirNamespace -> execute_with_namespace) as the code under test.
    table = pa.table(
        {
            "id": [1, 2, 3],
            "name": ["Alice", "Bob", "Carol"],
            "age": [28, 34, 29],
        }
    )
    # overwrite keeps the test re-runnable (the dataset persists on the store).
    write_dataset(
        table, f"{uri}/Person.lance", storage_options=storage_options, mode="overwrite"
    )

    config = (
        GraphConfig.builder()
        .with_node_label("Person", "id")
        .build()
    )
    query = CypherQuery("MATCH (p:Person) WHERE p.age > 30 RETURN p.name").with_config(
        config
    )

    result = query.execute_with_namespace(
        DirNamespace(uri, storage_options=storage_options)
    )
    data = result.to_pydict()

    assert set(data["p.name"]) == {"Bob"}
