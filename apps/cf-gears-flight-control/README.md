# CF/Gears Flight Control

The minimal platform control-plane deployment unit for a *distributed*
(out-of-process) CF/Gears deployment.

## Overview

`flight-control` is the composed "flight-control image": it links the minimal
platform **control plane** — the directory, transport, edge, GTS catalogue, and
edge JWT validation — and runs them under the ToolKit `HostRuntime`. Everything
else — the AuthZ plane and all application gears — runs *elsewhere*, as separate
out-of-process (OoP) worker processes or Kubernetes pods that discover
flight-control at runtime via its `DirectoryService`.

It is the distributed counterpart to `cf-gears-example-server`, which links
*every* gear into a single embedded (Profile 1) process. `flight-control`
deliberately links *only* the control-plane gears, so every other gear can be
deployed and scaled independently:

- **Profile 2 (Self-Hosted)** — flight-control runs the directory + edge; OoP
  worker processes register with it over UDS (single-node) or TCP.
- **Profile 3 (K8s Native)** — flight-control runs as the platform pod; each
  other gear runs as its own pod, fronted by an external gateway.

The authoritative model lives in `docs/arch/toolkit-oop/`: ADR-0001 (deployment
profiles) and the DESIGN "Flight Control Composition" section.

## What it links

- `service-discovery` — hosts the `DirectoryService` that OoP gears register with.
- `grpc-hub` — the gRPC transport surface for the directory and platform-plane RPCs.
- `api-gateway` — the built-in edge and REST host; reverse-proxies `exposed` OoP
  routes it discovers via the directory (see ADR-0003, gateway abstraction), and
  hosts the co-located `authn-resolver` routes.
- `types-registry` — the Global Type System (GTS) catalogue.
- `authn-resolver` — turns a bearer token into a tenant `SecurityContext` at the edge.

Gear isolation for OoP images is achieved by the dependency graph (which gears a
binary links), not by `#[cfg]` gates — see `src/registered_gears.rs`.

## What it does NOT link

Everything else runs out-of-process and registers with flight-control's
directory. The default is **one gear, one pod**. The exception is **coupling**:
when a gear is trust-coupled to another — it reaches its peer under a synthetic /
anonymous `SecurityContext` that is safe only in-process — the coupled gears stay
embedded together in a single OoP unit rather than split into separate pods,
until the coupling is removed (a remote surface on the peer plus S2S credentials
to replace the anonymous context).

The **AuthZ plane** is one such bundle: `authz-resolver` (the PDP) with
`tenant-resolver` and `resource-group`, which chain to each other under
`SecurityContext::anonymous()`. It runs as its own OoP unit serving
`/authz-resolver/v1/evaluate`; PEPs reach it via directory resolution. Other
platform services and all application gears run OoP too.

## Plugins & vendor selection

Only the `authn-resolver` plugin axis is relevant to this minimal control plane
(authz / tenant-resolution plugins live with the AuthZ plane). Plugin
*availability* is a build-time choice selected via Cargo features; the *active*
vendor is chosen at runtime from `authn-resolver`'s GTS `vendor` config.
`static-authn` (the default) is the accept-all dev plugin and boots with zero
external dependencies; build the production image with
`--no-default-features --features oidc-authn` to wire real OIDC. See `Cargo.toml`.

## Feature flags

- `fips` — route all crypto through the AWS-LC FIPS-validated module.
- `k8s` — Kubernetes platform-plane auth (SA-token TokenReview validation).
- `otel` — OpenTelemetry tracing/metrics export.

## Usage

```text
flight-control --config <path> [run]     # start flight-control (run is the default)
flight-control --config <path> --list-gears
flight-control --config <path> --print-config
```

A ready-to-run dev config (the `static-authn` default) lives at
`config/flight-control.yaml`.

## Helm component databases

The Flight Control binary includes SQLite, PostgreSQL and MySQL/MariaDB drivers.
Choose the database separately for each database-consuming component through the
standard ToolKit configuration in chart values:

- `config.data.database.servers.<name>` declares a named connection with `engine:
  sqlite`, `postgres` or `mysql`. MariaDB uses `engine: mysql` and port 3306.
- `config.data.gears.<component>.database.server` selects that connection.
  Component fields such as `dbname`, `pool` and credentials can override connection
  defaults, so components can share a server or use different servers and engines.
- SQLite's `file`/`path` belongs to the component. When switching a component to
  PostgreSQL/MySQL, remove any inherited SQLite `file`/`path` with `null` in the
  override values.

Currently `types-registry` is the database-consuming component in this composition.
The chart defaults to SQLite at `/app/data/types-registry/types_registry.db`, with
no external database dependency. This is pod-local storage; mount persistent storage
at `/app/data` through `extraVolumes`/`extraVolumeMounts` to retain it across pod
replacement. Keep SQLite deployments at one replica; use an external server for a
shared catalogue across replicas.

For PostgreSQL, install with the checked-in override after provisioning the database,
user and Kubernetes Secret `flight-control-postgres` with key `password`:

```bash
helm install flight-control apps/cf-gears-flight-control/deploy/helm/flight-control \
  -f apps/cf-gears-flight-control/deploy/helm/flight-control/values-postgres.yaml
```

For MySQL or MariaDB, use `values-mysql.yaml` instead and create Secret
`flight-control-mysql` with key `password`. Change the example host to the actual
MySQL/MariaDB service. The chart connects to existing database servers; it does not
install them or create their databases/users.

For example, this values override binds only the registry to PostgreSQL:

```yaml
config:
  data:
    database:
      servers:
        postgres:
          engine: postgres
          host: postgres.default.svc.cluster.local
          port: 5432
          user: flight_control
          password: "${TYPES_REGISTRY_DB_PASSWORD}"
    gears:
      types-registry:
        database:
          server: postgres
          dbname: flight_control
          file: null
```

Supply password environment variables through `extraEnv[].valueFrom.secretKeyRef`
as shown in the override files. Each component can use its own variable and Secret.
The ConfigMap contains environment placeholders, not passwords. Optional
`secrets[]` entries create chart-managed Secrets; their contents and `config.data`
participate in rollout checksums. For externally managed Secrets, restart the pods
after rotating credentials.

Database connections use the standard toolkit connection fields above; the chart has
no chart-specific database values. Local YAML examples under `config/` continue to use
SQLite.
