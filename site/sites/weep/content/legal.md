---
title: Legal
layout: page
description: WEEP is free software under the MIT license, from RAGTUX LLC. The license in plain words, trademarks, third-party components, privacy, and how to reach us.
---

# Legal

WEEP is free software published by RAGTUX LLC, a Minnesota limited liability company. This
page is in ordinary words; the binding text is
[LICENSE](https://github.com/ragtux/WEEP/blob/main/LICENSE), which governs where the two
differ.

## The license

WEEP is under the **MIT license**. In plain words:

- You may use it for anything, including commercially, without paying anyone.
- You may read, change and redistribute the source, and ship your own version.
- You must keep the copyright notice and the license text with any copy you pass on.
- It comes with **no warranty of any kind**.

> **Warning** v0.1.0, saving files in place with autosave on. Keep your work in version
> control.

## Trademarks

**JetBrains** and **Fleet** are trademarks of JetBrains s.r.o. WEEP is an independent,
clean-room project that reads Fleet's formats and follows its interface; it is **not
affiliated with, endorsed by, or sponsored by** JetBrains. The names say only what WEEP is
modeled on and compatible with.

WEEP contains no Fleet code. The only material it ships from the Fleet distribution is
openly licensed: the Inter and JetBrains Mono fonts.

**WEEP** and **RAGTUX** are names of RAGTUX LLC. The MIT license covers the code, not the
names: a fork is welcome under its own name.

## Third-party components

| Component | License |
|---|---|
| [GPUI](https://gpui.rs), the interface toolkit from Zed | Apache 2.0 |
| Inter, JetBrains Mono | SIL Open Font License 1.1 |
| IntelliJ Platform `expui` icons | Apache 2.0, header in every file |
| WEEP's own icons | MIT |

Every Rust dependency keeps its own license, listed in `Cargo.toml` and `Cargo.lock`.

## Privacy

WEEP makes **no network connection of its own**: no telemetry, no analytics, no update
check, no crash reporting, no account. The processes it starts — git, your shell, your
language server, your run configurations — do what they do on their own.

This site is static files: no cookies, no analytics, no third-party content. The server
keeps ordinary logs, unshared. Full statement:
[ragtux.com/privacy](https://ragtux.com/privacy/).

## Reaching us

| For | Address |
|---|---|
| Bugs, patches, feature discussion | [GitHub issues](https://github.com/ragtux/WEEP/issues) |
| Anything else about WEEP | [support@ragtux.com](mailto:support@ragtux.com) |
| Licensing and legal | [legal@ragtux.com](mailto:legal@ragtux.com) |
| Security reports | [security@ragtux.com](mailto:security@ragtux.com) |
