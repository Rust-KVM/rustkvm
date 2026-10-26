# RustKVM documentation

| Guide | For |
| --- | --- |
| [Quick start: flash and first boot](quickstart.md) | Installing a release image and first login |
| [Getting started on RK3588](getting-started.md) | Flashing, deploying, wiring and first login |
| [API reference](api.md) | HTTP routes, JSON-RPC, MCP tools, HID-RPC, MQTT, Prometheus metrics |
| [Building](building.md) | Docker devkit, application package and firmware image, pinned versions |
| [Customizing your build](customizing.md) | Your own hostname, UI name, USB IDs, firmware version and build images |
| [Stability soak and repro log (中文)](stability-testing.md) | How the 2026-10 soak test was run, sampled and judged; repro, root cause and fix for each problem found |
| [Releasing and shipping firmware](firmware-release.md) | Versioning, release checklist, flashing, factory state, acceptance test |
| [Building from source manually](../README.md#building-from-source) | Frontend, x86_64 host build and the aarch64 cross toolchain without containers |
| [Architecture and conventions](../AGENTS.md) | Module tree, coding rules, gotchas |
| [Contributing](../CONTRIBUTING.md) | Workflow, CI gates, PR expectations, release process |
| [Changelog](../CHANGELOG.md) | What changed between versions |
| [Security policy](../SECURITY.md) | Reporting vulnerabilities, hardening tips |

The device also serves its own HTTP schema at `/api-doc/openapi.json`, with an
interactive explorer at `/scalar-ui`.
