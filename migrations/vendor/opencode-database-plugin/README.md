# opencode-database-plugin vendored SQL

Source: https://github.com/aemr3/opencode-database-plugin
Package: opencode-database-plugin@1.0.12
Upstream commit: 53fea73
License: Apache-2.0

`schema.sql` is copied from upstream `sql/schema.sql` and kept as the reference
DDL. The executable HyperVibes migration adapts this schema to run under the
dedicated `opencode` Postgres schema by setting `search_path`.
