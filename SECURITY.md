# Security policy

uniwidged runs other people's code in your desktop session, so security
reports are taken seriously and handled first.

## Supported versions

| Version | Supported |
|---|---|
| 1.x | ✅ security fixes |
| < 1.0 | ❌ please upgrade |

## Reporting a vulnerability

**Please do not open a public issue.** Use GitHub's
[private vulnerability reporting](https://github.com/JosunLP/uniwidged/security/advisories/new).

Include what you can of: the version (`uniwidged --version`), the platform and
session type (`uniwidged doctor`), a widget or steps that reproduce it, and
what you expected the sandbox, signature check or capability gate to do.

You will get an acknowledgement within a week. A fix is released as a 1.x
patch, with credit to you unless you prefer otherwise.

## In scope

* A widget reaching anything its granted capabilities do not cover — a file
  outside its folder, a host it was not granted, another widget's secrets.
* A widget installing, or being updated, past a signature or consent check.
* A widget hanging, crashing or exhausting the desktop despite the deadline and
  memory limits.
* Registry indexes or downloads that are accepted when they should not be.

The [security model](https://josunlp.github.io/uniwidged/project/security)
describes what is promised, and what deliberately is not.
