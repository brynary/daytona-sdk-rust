# Daytona upstream version

The generated Rust API clients are synchronized with
[`daytona/clients` v0.205.1](https://github.com/daytona/clients/releases/tag/v0.205.1).

- Main API source: `openapi-specs/api.json`
- Toolbox API source: `openapi-specs/toolbox.json`
- OpenAPI Generator: `7.21.0`
- Main API SHA-256: `7682a9cefcd12810da736ea76527acc4e6e9c32dd53b39337bfbb1708b7b48a3`
- Toolbox API SHA-256: `3dbcb22f53b0205deedc9310dc2e87aa346cf28db9372508c28919b70a13b3f6`

The post-processing script retains Rust-specific compatibility changes for
large file sizes, generated-code lint warnings, and unknown response permission
values.
