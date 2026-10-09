# Security Policy

RustKVM gives remote, BIOS-level control of the machine it is attached to, so we take
vulnerabilities seriously.

## Supported versions

RustKVM has not had a tagged release yet. Security fixes land on the `dev` branch,
which is the only supported line until the first release.

| Version | Supported |
| --- | --- |
| `dev` branch | Yes |
| Anything older | No |

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report privately through GitHub:
[**Report a vulnerability**](https://github.com/Rust-KVM/rustkvm/security/advisories/new)
(Security tab → Advisories → Report a vulnerability).

Include as much as you can:

- the affected component (web server, auth, RPC method, MCP, USB gadget, cloud relay, …)
- the commit you tested
- steps to reproduce or a proof of concept
- the impact you expect (for example authentication bypass or remote code execution)

We aim to acknowledge reports within 7 days and to agree on a disclosure timeline with
you once the issue is confirmed. Reporters are credited in the advisory unless they ask
not to be.

## Scope

In scope: everything in this repository, including `rustkvm_app`, the `rkvm-*` crates and
the `rkvm-web` frontend.

Out of scope: vulnerabilities in third-party dependencies that are already public
(we track those with `cargo deny` and Dependabot), and attacks that need physical
access to the device.

## Hardening tips for operators

- Set a device password in the setup wizard; avoid `noPassword` mode on shared networks.
- Treat API tokens like passwords. Revoke unused ones with the `revokeApiToken` RPC.
- Keep the device off the public internet. Prefer a VPN such as Tailscale.
- The default TLS certificate is self-signed and regenerated at every boot; install
  your own certificate if browsers must trust the device.
