# RustKVM documentation

| Guide | For |
| --- | --- |
| [Getting started on RK3588](getting-started.md) | Flashing, deploying, wiring and first login |
| [API reference](api.md) | HTTP routes, JSON-RPC, MCP tools, HID-RPC, MQTT, Prometheus metrics |
| [Building from source](../README.md#building-from-source) | Frontend, x86_64 host build and the aarch64 cross toolchain |
| [Architecture and conventions](../AGENTS.md) | Module tree, coding rules, gotchas |
| [Contributing](../CONTRIBUTING.md) | Workflow, CI gates, PR expectations, release process |
| [Changelog](../CHANGELOG.md) | What changed between versions |
| [Security policy](../SECURITY.md) | Reporting vulnerabilities, hardening tips |

The device also serves its own HTTP schema at `/api-doc/openapi.json`, with an
interactive explorer at `/scalar-ui`.
