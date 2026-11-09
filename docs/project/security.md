# Security model

uniwidged runs **other people's code in your desktop session, all day**. This
page is what it promises about that, what it does not, and how to report a
problem.

## What a widget is up against

| Layer | Question it answers | Mechanism |
|---|---|---|
| **Signature** | Who wrote this, and is it what they published? | Ed25519 over the component **and** its manifest, checked against your `trusted-keys` before anything runs. |
| **Consent** | What did you agree it may do? | The manifest's capability list, shown at install, recorded in `config.toml` as `granted`. |
| **Sandbox** | What can it actually reach? | A Wasmtime component with WASI Preview 2 and no ambient authority. Every host function checks the grant on every call. |
| **Containment** | What happens when it misbehaves? | Memory cap, 16 ms deadline per call, three failures and it is given up on. |

### Signatures

* **Both halves are signed.** A signature over the component alone would let
  anybody widen `capabilities` and pass a widget on under its author's name.
  The digest covers component and manifest, length-prefixed and
  domain-separated.
* **A bad signature is never installable**, `--allow-unsigned` or not. A
  missing signature is an absence; a failing one is a positive claim that
  something is wrong.
* **Trust is a file.** `trusted-keys`, one key per line, readable and editable
  by the person it protects. No key server, no certificate chain, nothing that
  needs infrastructure the project could not honestly run.

### Registries

* The **index** is signed by the registry's publisher, and that key is pinned
  when the registry is added. It says who publishes the list — never who wrote
  a widget.
* Each **widget** is still checked against *your* trusted keys. A registry can
  offer, and never install: downloading is not trusting.
* What a listing claims is checked against the manifest that arrives. A
  download whose manifest asks for more than the entry showed is refused with
  every signature valid.
* `update` — the one command that installs something nobody looked at — refuses
  a version that asks for more than the installed one was granted.

### The sandbox

* **No ambient authority.** WASI gives the component no files, sockets or
  environment of its own — those are absent, not merely denied. What it can
  reach is uniwidged's own host functions, and each of them checks the grant
  on every call: an ungranted one answers `NotGranted` and does nothing.
* **`fs`** is checked twice: textually, and again after the filesystem has
  resolved the path, because only the second sees a symlink somebody planted.
  Writes land through a rename. Per-file and per-widget quotas.
* **`net`** is HTTPS only (rustls, no OpenSSL) to exactly the granted host; a
  redirect off it is refused.
* **`link`** opens `https` URLs on the granted host only, at the user's click.
* **`command`** runs the granted program name — no shell, no path, no standard
  input, almost no environment, bounded time and output.
* **`secrets`** is an XChaCha20-Poly1305 store, one namespace per widget, with
  no argument through which to name another's. An edited store fails to open
  rather than decrypting to something a widget then believes.

### Containment

* **Memory** — a resource limiter per instance, at the manifest's `memory-mb`.
* **Time** — epoch interruption with a deadline on every call. The desktop
  never blocks on guest code.
* **Crashes** — a trapped instance is never re-entered. A fresh one starts from
  `Default`; after three failures in a row the desktop gives up on it, showing
  its last frame.
* **Inputs** — every view tree is validated before it is touched: node count,
  string length, sparkline length, child indices that must increase so a cycle
  cannot be expressed.

## What it does not promise

Named so that nobody has to find out:

* **`command:<name>` is as powerful as that command.** `command:sh` is
  arbitrary code execution. The host cannot know what a program does; you are
  agreeing to the program.
* **A widget can show you anything.** It cannot draw arbitrary pixels, but it
  can write misleading text in its own panel.
* **The file keychain protects against other widgets, not against you.** Its
  key sits beside it, readable by your user; `install` says what the keychain
  on this machine is at the moment you approve a widget that asks for it.
* **The host itself is not sandboxed** from your session. It has to draw on
  your desktop, read your fonts and watch your monitors. The sandbox that
  matters is the one inside it, around each widget.

## Supply chain

Every dependency comes from crates.io. `cargo deny` runs in CI on every change
for advisories, licences and sources, and Wasmtime is pinned at a release that
fixes its known advisories. See `deny.toml` and `NOTICE.md`.

## Reporting a vulnerability

Please **do not open a public issue** for a security problem. Use GitHub's
[private vulnerability reporting](https://github.com/JosunLP/uniwidged/security/advisories/new)
for this repository. You will get an acknowledgement, and a fix will be
released with credit to you unless you prefer otherwise. See
[`SECURITY.md`](https://github.com/JosunLP/uniwidged/blob/main/SECURITY.md).
