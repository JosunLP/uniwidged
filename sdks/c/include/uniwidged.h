/*
 * uniwidged — the C SDK.
 *
 * A widget in C is the same thing as a widget in Rust: a WebAssembly
 * component that *describes* what it shows, built for `wasm32-wasip2` against
 * `wit/uniwidged.wit`. This header is a thin layer over the bindings
 * `wit-bindgen c` generates from that file — `widget.h`, which is included
 * below and is always there underneath for anything this layer does not wrap.
 *
 * It exists for one reason: the interface is a WIT world, and a contract that
 * only one language has ever been written against is a Rust API with extra
 * steps. This SDK is the second language.
 *
 * ## What you write
 *
 *     #include <uniwidged.h>
 *
 *     static int count;
 *
 *     void uw_widget_view(uw_tree *t) {
 *         uw_node root = uw_panel(t, uw_id("counter"));
 *         uw_textf(t, root, "%d", count);
 *         uw_on_click(t, uw_chip(t, root, "+1"), uw_id("plus"));
 *     }
 *
 *     bool uw_widget_on_event(const uw_event *e) {
 *         if (uw_clicked(e, uw_id("plus"))) { count++; return true; }
 *         return false;
 *     }
 *
 * `uw_widget_view` is required. `uw_widget_start`, `uw_widget_on_event` and
 * `uw_widget_wake` are optional: leave them out and the widget starts without
 * complaint, ignores events, and is never woken — which is what `Wake::Never`
 * means in the Rust SDK, and is right more often than it looks.
 *
 * ## Memory
 *
 * Everything a view allocates belongs to the tree, and the tree belongs to the
 * host once `uw_widget_view` returns: the generated bindings free it after the
 * host has read it. So a view never frees anything, and never keeps a pointer
 * into the tree past its own return.
 *
 * ## Defaults
 *
 * A node starts as the Rust SDK's does: a column, no gap, no padding,
 * `UW_ALIGN_STRETCH`, the plain role, the neutral accent. `uw_panel`, `uw_row`,
 * `uw_card`, `uw_spacer` and `uw_chip` set what their Rust namesakes set, so a
 * tree built here and one built there lay out the same.
 *
 * Licensed MIT OR Apache-2.0, like the rest of uniwidged.
 */
#ifndef UNIWIDGED_H
#define UNIWIDGED_H

#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "widget.h"

#ifdef __cplusplus
extern "C" {
#endif

/* The ABI this SDK speaks: the frozen 1.x interface. */
#define UW_ABI_VERSION 9u

/* A node, by its index in the tree. */
typedef uint32_t uw_node;
/* "No parent": what the first node of a tree is added under. */
#define UW_NONE ((uw_node)UINT32_MAX)

typedef uniwidged_widget_types_event_t uw_event;
typedef uniwidged_widget_types_wake_t uw_wake;

/* The tree a view is building. Opaque in spirit; touch it through the calls
 * below. */
typedef struct uw_tree {
    uniwidged_widget_types_node_t *nodes;
    size_t len;
    size_t cap;
} uw_tree;

/* --- Roles, accents, layout ------------------------------------------- */

typedef enum {
    UW_ROLE_PLAIN = UNIWIDGED_WIDGET_TYPES_ROLE_PLAIN,
    UW_ROLE_PANEL = UNIWIDGED_WIDGET_TYPES_ROLE_PANEL,
    UW_ROLE_CARD = UNIWIDGED_WIDGET_TYPES_ROLE_CARD,
    UW_ROLE_CHIP = UNIWIDGED_WIDGET_TYPES_ROLE_CHIP,
    UW_ROLE_TITLE = UNIWIDGED_WIDGET_TYPES_ROLE_TITLE,
    UW_ROLE_DISPLAY = UNIWIDGED_WIDGET_TYPES_ROLE_DISPLAY,
    UW_ROLE_SECTION = UNIWIDGED_WIDGET_TYPES_ROLE_SECTION,
    UW_ROLE_BODY = UNIWIDGED_WIDGET_TYPES_ROLE_BODY,
    UW_ROLE_BODY_STRONG = UNIWIDGED_WIDGET_TYPES_ROLE_BODY_STRONG,
    UW_ROLE_CAPTION = UNIWIDGED_WIDGET_TYPES_ROLE_CAPTION,
} uw_role_t;

typedef enum {
    UW_ACCENT_NEUTRAL = UNIWIDGED_WIDGET_TYPES_ACCENT_NEUTRAL,
    UW_ACCENT_BRAND = UNIWIDGED_WIDGET_TYPES_ACCENT_BRAND,
    UW_ACCENT_POSITIVE = UNIWIDGED_WIDGET_TYPES_ACCENT_POSITIVE,
    UW_ACCENT_WARNING = UNIWIDGED_WIDGET_TYPES_ACCENT_WARNING,
    UW_ACCENT_CRITICAL = UNIWIDGED_WIDGET_TYPES_ACCENT_CRITICAL,
} uw_accent_t;

typedef enum {
    UW_ALIGN_START = UNIWIDGED_WIDGET_TYPES_ALIGN_START,
    UW_ALIGN_CENTER = UNIWIDGED_WIDGET_TYPES_ALIGN_CENTER,
    UW_ALIGN_END = UNIWIDGED_WIDGET_TYPES_ALIGN_END,
    UW_ALIGN_STRETCH = UNIWIDGED_WIDGET_TYPES_ALIGN_STRETCH,
} uw_align_t;

typedef enum {
    UW_JUSTIFY_START = UNIWIDGED_WIDGET_TYPES_JUSTIFY_START,
    UW_JUSTIFY_CENTER = UNIWIDGED_WIDGET_TYPES_JUSTIFY_CENTER,
    UW_JUSTIFY_END = UNIWIDGED_WIDGET_TYPES_JUSTIFY_END,
    UW_JUSTIFY_SPACE_BETWEEN = UNIWIDGED_WIDGET_TYPES_JUSTIFY_SPACE_BETWEEN,
} uw_justify_t;

typedef enum {
    UW_ICON_REFRESH = UNIWIDGED_WIDGET_TYPES_ICON_REFRESH,
    UW_ICON_SETTINGS = UNIWIDGED_WIDGET_TYPES_ICON_SETTINGS,
    UW_ICON_CLOSE = UNIWIDGED_WIDGET_TYPES_ICON_CLOSE,
    UW_ICON_CHEVRON = UNIWIDGED_WIDGET_TYPES_ICON_CHEVRON,
    UW_ICON_CHECK = UNIWIDGED_WIDGET_TYPES_ICON_CHECK,
    UW_ICON_ALERT = UNIWIDGED_WIDGET_TYPES_ICON_ALERT,
    UW_ICON_OFFLINE = UNIWIDGED_WIDGET_TYPES_ICON_OFFLINE,
} uw_icon_t;

/* --- Ids ---------------------------------------------------------------- */

/* A stable id for a name — the same 32-bit FNV-1a as the Rust SDK's
 * `NodeId::named`, so `uw_id("plus")` here and `NodeId::named("plus")` there
 * are the same node. Never 0, which is the anonymous id. */
uint32_t uw_id(const char *name);

/* --- Building a view ------------------------------------------------------
 *
 * Each call appends one node under `parent` (or as the root, for `UW_NONE` on
 * an empty tree) and returns it. A parent is always added before its children,
 * which is the property the host's validation requires. */

uw_node uw_panel(uw_tree *t, uint32_t id);
uw_node uw_group(uw_tree *t, uw_node parent);
uw_node uw_row(uw_tree *t, uw_node parent);
uw_node uw_column(uw_tree *t, uw_node parent);
uw_node uw_card(uw_tree *t, uw_node parent);
uw_node uw_text(uw_tree *t, uw_node parent, const char *text);
uw_node uw_textf(uw_tree *t, uw_node parent, const char *format, ...)
    __attribute__((format(printf, 3, 4)));
uw_node uw_chip(uw_tree *t, uw_node parent, const char *text);
uw_node uw_icon(uw_tree *t, uw_node parent, uw_icon_t icon);
uw_node uw_image(uw_tree *t, uw_node parent, uint32_t handle);
uw_node uw_divider(uw_tree *t, uw_node parent);
uw_node uw_spacer(uw_tree *t, uw_node parent);
uw_node uw_progress(uw_tree *t, uw_node parent, float value);
/* The most recent 256 of `values` are kept, as in the Rust SDK. */
uw_node uw_sparkline(uw_tree *t, uw_node parent, const float *values, size_t count);

/* --- Changing a node ------------------------------------------------------ */

void uw_role(uw_tree *t, uw_node n, uw_role_t role);
void uw_accent(uw_tree *t, uw_node n, uw_accent_t accent);
void uw_series(uw_tree *t, uw_node n, uint8_t series);
void uw_muted(uw_tree *t, uw_node n);
void uw_direction_row(uw_tree *t, uw_node n);
void uw_gap(uw_tree *t, uw_node n, uint8_t steps);
void uw_padding(uw_tree *t, uw_node n, uint8_t steps);
void uw_align(uw_tree *t, uw_node n, uw_align_t align);
void uw_justify(uw_tree *t, uw_node n, uw_justify_t justify);
void uw_grow(uw_tree *t, uw_node n, uint8_t share);
void uw_wrap(uw_tree *t, uw_node n);
/* Makes the node a button reported as `id`. */
void uw_on_click(uw_tree *t, uw_node n, uint32_t id);
void uw_link(uw_tree *t, uw_node n, const char *url);
void uw_label(uw_tree *t, uw_node n, const char *label);
void uw_tooltip(uw_tree *t, uw_node n, const char *tooltip);
void uw_disabled(uw_tree *t, uw_node n);
void uw_selected(uw_tree *t, uw_node n);
void uw_checked(uw_tree *t, uw_node n, bool checked);

/* --- Events and waking ------------------------------------------------------ */

bool uw_clicked(const uw_event *e, uint32_t id);
bool uw_configured(const uw_event *e);

uw_wake uw_wake_never(void);
uw_wake uw_wake_after_ms(uint32_t ms);
uw_wake uw_wake_at(int64_t epoch_seconds);
/* The top of the next wall-clock minute — what a clock asks for. */
uw_wake uw_wake_next_minute(void);

/* --- The host ----------------------------------------------------------------
 *
 * The common calls, in C's terms. Each answers false where the host said
 * `denied`, and the full answer — `not-granted`, `malformed`, `unavailable` —
 * is one call away in `widget.h`. */

/* Seconds since the Unix epoch, by the wall clock. */
int64_t uw_now(void);
void uw_log(const char *message);
void uw_logf(const char *format, ...) __attribute__((format(printf, 1, 2)));
/* Writes at most `size` bytes, terminated. Needs `locale`. */
bool uw_format_time(int64_t epoch_seconds, char *out, size_t size);
bool uw_format_date(int64_t epoch_seconds, char *out, size_t size);
/* Needs `metrics`. */
bool uw_cpu_load(float *out);
bool uw_memory_load(float *out);
/* This widget's own settings, as its manifest declared them. False for a key
 * the manifest does not declare, or one of another type. */
bool uw_setting_number(const char *key, double *out);
bool uw_setting_toggle(const char *key, bool *out);
bool uw_setting_text(const char *key, char *out, size_t size);

/* --- What a widget provides ------------------------------------------------- */

/* Required. Build the view into `t`. */
void uw_widget_view(uw_tree *t);
/* Optional. Return false and set `*error` (a static string is fine) to refuse
 * to start. */
bool uw_widget_start(const char **error);
/* Optional. Return true if the view would now differ. */
bool uw_widget_on_event(const uw_event *e);
/* Optional. When to be called again. */
uw_wake uw_widget_wake(void);

#ifdef __cplusplus
}
#endif

#endif /* UNIWIDGED_H */
