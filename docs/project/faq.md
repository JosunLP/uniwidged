# FAQ

### Why can't my widget choose its own colours or fonts?

Because then twenty widgets by twenty authors would look like twenty toolkits,
and none of them would follow your dark mode, contrast setting or font scale.
A widget names a **role** and an **accent**; the theme makes it right on *this*
desktop. See [Core concepts](../guide/concepts.md#you-describe-the-host-decides).

### Can a widget draw a chart?

A sparkline, yes — `sparkline(values)` hands the host up to 256 points and the
host draws the line, the area and the latest-point marker in the theme's style.
That is as far towards a canvas as uniwidged goes, on purpose.

### Which languages can I write a widget in?

Any language that compiles to a WebAssembly **component** implementing
`wit/uniwidged.wit`. Two SDKs exist: **Rust** (`uniwidged-guest` on crates.io)
and **C** ([`sdks/c`](https://github.com/JosunLP/uniwidged/tree/main/sdks/c),
built with wasi-sdk on bindings `wit-bindgen c` generates). CI builds the C
example and runs it in the real runtime, which is what makes the interface file
a contract rather than a description of the Rust SDK.

### Does it work on GNOME?

On GNOME Wayland there is no `wlr-layer-shell`, so widgets open as ordinary
windows that are not kept below others — the start-up log says so. GNOME on
X11 works as a proper desktop layer.

### Windows? macOS?

Both, from 1.1. On **Windows** a widget is a `WS_EX_NOREDIRECTIONBITMAP`
tool window kept at `HWND_BOTTOM`, presented through a DirectComposition
swapchain so the wallpaper shows through; the keychain's key is sealed with
DPAPI. On **macOS** it is a non-activating panel at desktop-icon level with a
`CAMetalLayer`, on every Space and still under Mission Control; the keychain's
key lives in your login keychain. Each platform's exit criterion is measured
against its real window manager in CI. They are new, and have had less use on
real desktops than Linux has — issues are welcome.

### How much CPU does it use?

With nothing changing, effectively none: there is no idle loop, no idle frame
and no timer between wakes. A clock wakes once a minute; eight clocks share
that one wake-up.

### Why Flatpak isn't offered

A widget host draws below every window, reads the session's fonts, watches the
monitors and verifies signatures before running anybody's code. A Flatpak
sandbox opened up for each of those would hold nothing. The sandbox that
matters is the one inside the process, around each widget.

### Is a widget's data kept when I remove it?

Yes, unless you say `uniwidged remove <name> --purge`. Reinstalling picks up
where it left off.

### How do I publish a widget?

Sign it with `uniwidged keygen` and `uniwidged sign`, publish your public key,
and either hand out the files or put them in a registry — one signed TOML file
on any static host. See [Finding widgets](../guide/registries.md#publishing-one).

### Will my widget break when uniwidged updates?

Not within 1.x. A widget built against 1.0 runs on every 1.x host. See
[ABI & stability](../reference/abi.md).

### Why "uniwidged"?

**Uni**fied **widge**ts — and the mark is a **u** whose right arm is the stem
of an **i**: *ui*. See [Brand](./brand.md).
