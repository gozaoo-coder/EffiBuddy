# Registry

Package store index. Each release publishes a `index.json` listing all
available packages. Core's `registry_client` fetches this file to populate
the package store UI.

## Schema

```jsonc
{
  "schema": 1,
  "updated": "<ISO8601>",
  "packages": [
    {
      "id": "com.author.name",        // reverse-DNS, unique
      "name": "Display Name",
      "version": "0.1.0",             // semver
      "description": "...",
      "author": "...",
      "download_url": "https://...",  // .tar.gz or .zip
      "sha256": "<hex>|null",         // optional integrity check
      "size": 12345                   // bytes
    }
  ]
}
```

## Publishing

1. Bump version in `packages/<name>/manifest.json`.
2. Build backend (`cargo build --release`) + frontend (`pnpm build`).
3. Pack `packages/<name>/` into `<name>-<version>.tar.gz`.
4. Compute sha256, upload archive to GitHub release.
5. Add entry to `index.json`, commit, push.
