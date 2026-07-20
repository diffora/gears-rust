# cf-gears-file-storage-sdk

Public API surface (client trait, model types, GTS constants, error envelope) for the
`file-storage` gear. See `gears/file-storage/docs/DESIGN.md`.

**Status**: model types (`File`, `FileVersion`, `ByteRange`, owner/metadata types, …) and GTS constants are real and
used by the gear today. The inter-gear client trait (`FileStorageClientV1` in `src/api.rs`) is still a stub — it
exposes only a placeholder `module_name()` method, with no presign/bind/metadata-CRUD operations wired yet.
