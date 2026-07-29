# zed-api-server

The [zed-pkg](https://zpkg.tech) registry REST API, in Rust: package and
version metadata in Postgres (SeaORM + the `migration/` crate), artifact
archives (tar.gz/zip) in any S3-compatible object storage — Cloudflare R2 in
production, AWS S3 or MinIO for alternatives and local dev. This is the
service `zed publish` and `zed install` talk to, and the whole stack is
self-hostable for private registries.

## Endpoints

| Method | Path | Notes |
| --- | --- | --- |
| GET | `/healthz` | liveness + db status |
| GET | `/v1/packages/{org}/{name}` | `PackageMetadata` |
| GET | `/v1/packages/{org}/{name}/versions/{version}` | `VersionMetadata` |
| PUT | `/v1/packages/{org}/{name}/versions/{version}` | publish: multipart `meta` (PublishMeta JSON) + `artifact` (bytes); bearer token; versions are immutable |
| GET | `/v1/artifacts/{sha256}` | 302 presigned URL (s3) or streamed bytes (local) |
| GET | `/v1/search?q=` | name/description match, cap 50 |
| POST | `/v1/orgs` | claim a namespace; bearer token |
| GET | `/v1/files/{org}/{name}/{version}/{path}` | unpkg-style: serve one file out of an artifact, immutable cache headers |

Errors are JSON `ApiError { code, message }` — codes include `not_found`,
`unauthorized`, `org_not_found`, `org_taken`, `version_exists`,
`sha256_mismatch`, `tag_not_found`, `invalid_manifest`.

Publish pipeline: bearer token -> manifest validation -> URL/manifest
agreement -> server-side sha256 recomputation -> org ownership -> VCS tag
verification (policy below) -> immutability check -> store artifact -> record
version.

## Configuration (env)

| Var | Default | Notes |
| --- | --- | --- |
| `BIND_ADDR` | `0.0.0.0:8080` | |
| `DATABASE_URL` | required | Postgres |
| `AUTO_MIGRATE` | `true` | run `migration/` on boot |
| `STORAGE_BACKEND` | `local` | `local` or `s3` |
| `STORAGE_LOCAL_DIR` | `.data/artifacts` | local backend |
| `S3_BUCKET` | required for s3 | |
| `S3_ENDPOINT_URL` | unset | set for R2/MinIO |
| `S3_REGION` | `auto` | R2 uses `auto` |
| `S3_FORCE_PATH_STYLE` | `true` | MinIO needs it |
| `PUBLIC_BASE_URL` | `http://localhost:8080` | used in download URLs |
| `ZED_VERIFY_TAGS` | `off` | `off` or `github` |
| `GITHUB_TOKEN` | unset | raises tag-check rate limits |
| `MAX_ARTIFACT_BYTES` | `104857600` | request body cap |
| `RUST_LOG` | `info` | |

### Cloudflare R2 mapping

| Setting | Value |
| --- | --- |
| `S3_ENDPOINT_URL` | `https://<account_id>.r2.cloudflarestorage.com` |
| `S3_REGION` | `auto` |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | R2 API token pair |
| Bucket | see `zed-infra/terraform/cloudflare` |

## Tag verification

`ZED_VERIFY_TAGS=github` enforces the provenance rule server-side: the
declared backing repo must have the tag the client publishes from
(`tag_not_found` otherwise). github.com is implemented in `src/verify.rs`;
GitLab/Bitbucket/Codeberg/SourceHut and hg hosts are the marked extension
point, and unverifiable hosts (self-hosted forges) are allowed through with a
warning so they are not locked out. The CLI independently verifies tags
client-side either way.

## Run it

```sh
# full local stack: postgres + minio + api (from the parent directory)
docker compose -f zed-api-server.rs/docker-compose.yml up --build

# or bare, against your own postgres
DATABASE_URL=postgres://zed:zed@localhost:5432/zed cargo run

# mint a token (printed once)
DATABASE_URL=... cargo run -- create-token --name ci --org acme

# then, from a package directory
zed org claim acme --registry http://localhost:8080 --token zpkg_...
zed publish --registry http://localhost:8080 --token zpkg_...
```

Equivalent curl publish:

```sh
curl -X PUT http://localhost:8080/v1/packages/acme/demo/versions/0.1.0 \
  -H "Authorization: Bearer zpkg_..." \
  -F 'meta={"manifest":{...},"vcs_tag":"v0.1.0","sha256":"...","size":123,"format":"tar.gz"};type=application/json' \
  -F 'artifact=@.zed/pack/acme-demo-0.1.0.tar.gz'
```

## Development

Clone side by side with `zed-interfaces` (path dependency). `cargo test`
runs without Postgres or network; DB-backed handler paths are exercised via
the compose stack (not unit-tested yet — known gap).

## Formal methods

[`formal/fm.toml`](formal/fm.toml) defines a schema-v1 `fmctl` gate for the
package-publication state machine. Its Quint model exhaustively checks the
finite concurrent-publisher state graph for a single immutable winner, atomic
advertisement/metadata visibility, fail-closed authorization, and committed
artifact availability. See [`formal/README.md`](formal/README.md) for bounds,
failure witnesses, the concurrency finding behind retained failed-upload
blobs, and an explicit account of what is not proved.

## License

MIT
