/*
 * The C SDK's implementation: the tree builder, the host helpers, and the
 * four exports the component model calls, which forward to the `uw_widget_*`
 * functions a widget defines.
 *
 * Every allocation here is `malloc`-family, because the generated bindings
 * free the returned tree with `free` once the host has read it. That is the
 * whole memory model, and the reason nothing in this file frees a node.
 */
#include "uniwidged.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* --- Allocation ------------------------------------------------------------ */

/* A widget that cannot allocate cannot describe anything, and a trap is what
 * the host turns into "this widget failed" — which is the truth. */
static void *grow(void *ptr, size_t bytes) {
    void *next = realloc(ptr, bytes);
    if (!next && bytes) {
        abort();
    }
    return next;
}

static widget_string_t owned(const char *text, size_t len) {
    widget_string_t s = {0};
    if (len == 0) {
        return s;
    }
    s.ptr = grow(NULL, len);
    memcpy(s.ptr, text, len);
    s.len = len;
    return s;
}

static widget_string_t owned_c(const char *text) {
    return owned(text ? text : "", text ? strlen(text) : 0);
}

static widget_string_t owned_vf(const char *format, va_list args) {
    va_list again;
    va_copy(again, args);
    int len = vsnprintf(NULL, 0, format, again);
    va_end(again);
    if (len <= 0) {
        widget_string_t empty = {0};
        return empty;
    }
    char *buffer = grow(NULL, (size_t)len + 1);
    vsnprintf(buffer, (size_t)len + 1, format, args);
    widget_string_t s = {(uint8_t *)buffer, (size_t)len};
    return s;
}

/* --- Ids --------------------------------------------------------------------- */

uint32_t uw_id(const char *name) {
    uint32_t hash = 0x811c9dc5u;
    for (const unsigned char *p = (const unsigned char *)name; p && *p; p++) {
        hash ^= *p;
        hash *= 0x01000193u;
    }
    return hash == 0 ? 1 : hash;
}

/* --- The tree ---------------------------------------------------------------- */

static uniwidged_widget_types_node_t *at(uw_tree *t, uw_node n) {
    if (n >= t->len) {
        abort();
    }
    return &t->nodes[n];
}

static uw_node add(uw_tree *t, uw_node parent, uniwidged_widget_types_content_t content) {
    if (parent == UW_NONE && t->len != 0) {
        /* A second root is a tree the host refuses; under the first is the
         * only reading of this that is not a mistake waiting to happen. */
        parent = 0;
    }
    if (t->len == t->cap) {
        t->cap = t->cap ? t->cap * 2 : 16;
        t->nodes = grow(t->nodes, t->cap * sizeof *t->nodes);
    }
    uw_node index = (uw_node)t->len++;
    uniwidged_widget_types_node_t *node = &t->nodes[index];
    memset(node, 0, sizeof *node);
    node->content = content;
    node->style.layout.align = UW_ALIGN_STRETCH;
    if (parent != UW_NONE) {
        widget_list_u32_t *children = &at(t, parent)->children;
        children->ptr = grow(children->ptr, (children->len + 1) * sizeof *children->ptr);
        children->ptr[children->len++] = index;
    }
    return index;
}

static uniwidged_widget_types_content_t content_of(uint8_t tag) {
    uniwidged_widget_types_content_t c;
    memset(&c, 0, sizeof c);
    c.tag = tag;
    return c;
}

uw_node uw_panel(uw_tree *t, uint32_t id) {
    uw_node n = add(t, UW_NONE, content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_GROUP));
    at(t, n)->id = id;
    uw_role(t, n, UW_ROLE_PANEL);
    uw_padding(t, n, 2);
    return n;
}

uw_node uw_group(uw_tree *t, uw_node parent) {
    return add(t, parent, content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_GROUP));
}

uw_node uw_row(uw_tree *t, uw_node parent) {
    uw_node n = uw_group(t, parent);
    uw_direction_row(t, n);
    uw_align(t, n, UW_ALIGN_CENTER);
    return n;
}

uw_node uw_column(uw_tree *t, uw_node parent) { return uw_group(t, parent); }

uw_node uw_card(uw_tree *t, uw_node parent) {
    uw_node n = uw_group(t, parent);
    uw_role(t, n, UW_ROLE_CARD);
    uw_padding(t, n, 2);
    return n;
}

uw_node uw_text(uw_tree *t, uw_node parent, const char *text) {
    uniwidged_widget_types_content_t c = content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_TEXT);
    c.val.text = owned_c(text);
    return add(t, parent, c);
}

uw_node uw_textf(uw_tree *t, uw_node parent, const char *format, ...) {
    uniwidged_widget_types_content_t c = content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_TEXT);
    va_list args;
    va_start(args, format);
    c.val.text = owned_vf(format, args);
    va_end(args);
    return add(t, parent, c);
}

uw_node uw_chip(uw_tree *t, uw_node parent, const char *text) {
    uw_node n = uw_text(t, parent, text);
    uw_role(t, n, UW_ROLE_CHIP);
    return n;
}

uw_node uw_icon(uw_tree *t, uw_node parent, uw_icon_t icon) {
    uniwidged_widget_types_content_t c = content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_ICON);
    c.val.icon = (uniwidged_widget_types_icon_t)icon;
    return add(t, parent, c);
}

uw_node uw_image(uw_tree *t, uw_node parent, uint32_t handle) {
    uniwidged_widget_types_content_t c = content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_IMAGE);
    c.val.image = handle;
    return add(t, parent, c);
}

uw_node uw_divider(uw_tree *t, uw_node parent) {
    return add(t, parent, content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_DIVIDER));
}

uw_node uw_spacer(uw_tree *t, uw_node parent) {
    uw_node n = add(t, parent, content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_SPACER));
    uw_grow(t, n, 1);
    return n;
}

static float clamp01(float v) { return v < 0.0f ? 0.0f : (v > 1.0f ? 1.0f : (v == v ? v : 0.0f)); }

uw_node uw_progress(uw_tree *t, uw_node parent, float value) {
    uniwidged_widget_types_content_t c = content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_PROGRESS);
    c.val.progress = clamp01(value);
    return add(t, parent, c);
}

uw_node uw_sparkline(uw_tree *t, uw_node parent, const float *values, size_t count) {
    enum { MAX_POINTS = 256 };
    if (count > MAX_POINTS) {
        values += count - MAX_POINTS;
        count = MAX_POINTS;
    }
    uniwidged_widget_types_content_t c = content_of(UNIWIDGED_WIDGET_TYPES_CONTENT_SPARKLINE);
    if (count) {
        c.val.sparkline.ptr = grow(NULL, count * sizeof(float));
        for (size_t i = 0; i < count; i++) {
            c.val.sparkline.ptr[i] = clamp01(values[i]);
        }
        c.val.sparkline.len = count;
    }
    return add(t, parent, c);
}

/* --- Changing a node -------------------------------------------------------------- */

void uw_role(uw_tree *t, uw_node n, uw_role_t role) { at(t, n)->style.role = (uint8_t)role; }

void uw_accent(uw_tree *t, uw_node n, uw_accent_t accent) {
    at(t, n)->style.accent.tag = (uint8_t)accent;
}

void uw_series(uw_tree *t, uw_node n, uint8_t series) {
    at(t, n)->style.accent.tag = UNIWIDGED_WIDGET_TYPES_ACCENT_SERIES;
    at(t, n)->style.accent.val.series = series;
}

void uw_muted(uw_tree *t, uw_node n) { at(t, n)->style.muted = true; }

void uw_direction_row(uw_tree *t, uw_node n) {
    at(t, n)->style.layout.direction = UNIWIDGED_WIDGET_TYPES_DIRECTION_ROW;
}

void uw_gap(uw_tree *t, uw_node n, uint8_t steps) { at(t, n)->style.layout.gap = steps; }
void uw_padding(uw_tree *t, uw_node n, uint8_t steps) { at(t, n)->style.layout.padding = steps; }
void uw_align(uw_tree *t, uw_node n, uw_align_t align) { at(t, n)->style.layout.align = (uint8_t)align; }

void uw_justify(uw_tree *t, uw_node n, uw_justify_t justify) {
    at(t, n)->style.layout.justify = (uint8_t)justify;
}

void uw_grow(uw_tree *t, uw_node n, uint8_t share) { at(t, n)->style.layout.grow = share; }
void uw_wrap(uw_tree *t, uw_node n) { at(t, n)->style.layout.wrap = true; }

void uw_on_click(uw_tree *t, uw_node n, uint32_t id) {
    at(t, n)->id = id;
    at(t, n)->semantics.interaction.tag = UNIWIDGED_WIDGET_TYPES_INTERACTION_BUTTON;
}

void uw_link(uw_tree *t, uw_node n, const char *url) {
    uniwidged_widget_types_interaction_t *i = &at(t, n)->semantics.interaction;
    if (i->tag == UNIWIDGED_WIDGET_TYPES_INTERACTION_LINK) {
        free(i->val.link.ptr);
    }
    i->tag = UNIWIDGED_WIDGET_TYPES_INTERACTION_LINK;
    i->val.link = owned_c(url);
}

static void set_option(widget_option_string_t *slot, const char *text) {
    if (slot->is_some) {
        free(slot->val.ptr);
    }
    slot->is_some = true;
    slot->val = owned_c(text);
}

void uw_label(uw_tree *t, uw_node n, const char *label) { set_option(&at(t, n)->semantics.label, label); }

void uw_tooltip(uw_tree *t, uw_node n, const char *tooltip) {
    set_option(&at(t, n)->semantics.tooltip, tooltip);
}

void uw_disabled(uw_tree *t, uw_node n) { at(t, n)->semantics.state.disabled = true; }
void uw_selected(uw_tree *t, uw_node n) { at(t, n)->semantics.state.selected = true; }

void uw_checked(uw_tree *t, uw_node n, bool checked) {
    at(t, n)->semantics.state.checked.is_some = true;
    at(t, n)->semantics.state.checked.val = checked;
}

/* --- Events and waking -------------------------------------------------------------- */

bool uw_clicked(const uw_event *e, uint32_t id) {
    return e->tag == UNIWIDGED_WIDGET_TYPES_EVENT_CLICKED && e->val.clicked == id;
}

bool uw_configured(const uw_event *e) { return e->tag == UNIWIDGED_WIDGET_TYPES_EVENT_CONFIGURED; }

uw_wake uw_wake_never(void) {
    uw_wake w;
    memset(&w, 0, sizeof w);
    w.tag = UNIWIDGED_WIDGET_TYPES_WAKE_NEVER;
    return w;
}

uw_wake uw_wake_after_ms(uint32_t ms) {
    uw_wake w = uw_wake_never();
    w.tag = UNIWIDGED_WIDGET_TYPES_WAKE_AFTER_MS;
    w.val.after_ms = ms;
    return w;
}

uw_wake uw_wake_at(int64_t epoch_seconds) {
    uw_wake w = uw_wake_never();
    w.tag = UNIWIDGED_WIDGET_TYPES_WAKE_AT;
    w.val.at = epoch_seconds;
    return w;
}

uw_wake uw_wake_next_minute(void) {
    int64_t now = uw_now();
    return uw_wake_at(now - now % 60 + 60);
}

/* --- The host ------------------------------------------------------------------------- */

int64_t uw_now(void) { return (int64_t)time(NULL); }

void uw_log(const char *message) {
    widget_string_t s = owned_c(message);
    uniwidged_widget_host_log(&s);
    free(s.ptr);
}

void uw_logf(const char *format, ...) {
    va_list args;
    va_start(args, format);
    widget_string_t s = owned_vf(format, args);
    va_end(args);
    uniwidged_widget_host_log(&s);
    free(s.ptr);
}

static bool copy_out(widget_string_t *s, char *out, size_t size) {
    if (size == 0) {
        widget_string_free(s);
        return false;
    }
    size_t n = s->len < size - 1 ? s->len : size - 1;
    memcpy(out, s->ptr, n);
    out[n] = '\0';
    widget_string_free(s);
    return true;
}

bool uw_format_time(int64_t epoch_seconds, char *out, size_t size) {
    widget_string_t s;
    uniwidged_widget_host_denied_t denied;
    if (!uniwidged_widget_host_format_time(epoch_seconds, &s, &denied)) {
        uniwidged_widget_host_denied_free(&denied);
        return false;
    }
    return copy_out(&s, out, size);
}

bool uw_format_date(int64_t epoch_seconds, char *out, size_t size) {
    widget_string_t s;
    uniwidged_widget_host_denied_t denied;
    if (!uniwidged_widget_host_format_date(epoch_seconds, &s, &denied)) {
        uniwidged_widget_host_denied_free(&denied);
        return false;
    }
    return copy_out(&s, out, size);
}

bool uw_cpu_load(float *out) {
    uniwidged_widget_host_denied_t denied;
    if (!uniwidged_widget_host_cpu_load(out, &denied)) {
        uniwidged_widget_host_denied_free(&denied);
        return false;
    }
    return true;
}

bool uw_memory_load(float *out) {
    uniwidged_widget_host_denied_t denied;
    if (!uniwidged_widget_host_memory_load(out, &denied)) {
        uniwidged_widget_host_denied_free(&denied);
        return false;
    }
    return true;
}

static bool setting(const char *key, uniwidged_widget_host_setting_value_t *value) {
    widget_string_t k = owned_c(key);
    bool found = uniwidged_widget_host_setting(&k, value);
    free(k.ptr);
    return found;
}

bool uw_setting_number(const char *key, double *out) {
    uniwidged_widget_host_setting_value_t v;
    if (!setting(key, &v)) {
        return false;
    }
    bool ok = v.tag == UNIWIDGED_WIDGET_HOST_SETTING_VALUE_NUMBER;
    if (ok) {
        *out = v.val.number;
    }
    uniwidged_widget_host_setting_value_free(&v);
    return ok;
}

bool uw_setting_toggle(const char *key, bool *out) {
    uniwidged_widget_host_setting_value_t v;
    if (!setting(key, &v)) {
        return false;
    }
    bool ok = v.tag == UNIWIDGED_WIDGET_HOST_SETTING_VALUE_TOGGLE;
    if (ok) {
        *out = v.val.toggle;
    }
    uniwidged_widget_host_setting_value_free(&v);
    return ok;
}

bool uw_setting_text(const char *key, char *out, size_t size) {
    uniwidged_widget_host_setting_value_t v;
    if (!setting(key, &v)) {
        return false;
    }
    if (v.tag != UNIWIDGED_WIDGET_HOST_SETTING_VALUE_TEXT) {
        uniwidged_widget_host_setting_value_free(&v);
        return false;
    }
    widget_string_t s = v.val.text;
    return copy_out(&s, out, size);
}

/* --- The optional half of a widget ---------------------------------------------------
 *
 * Weak, so a widget that defines one replaces it and one that does not gets
 * the Rust SDK's defaults. */

__attribute__((weak)) bool uw_widget_start(const char **error) {
    (void)error;
    return true;
}

__attribute__((weak)) bool uw_widget_on_event(const uw_event *e) {
    (void)e;
    return false;
}

__attribute__((weak)) uw_wake uw_widget_wake(void) { return uw_wake_never(); }

/* --- The exports ------------------------------------------------------------------------ */

uint32_t exports_uniwidged_widget_guest_abi_version(void) { return UW_ABI_VERSION; }

bool exports_uniwidged_widget_guest_start(widget_string_t *err) {
    const char *why = NULL;
    if (uw_widget_start(&why)) {
        return true;
    }
    *err = owned_c(why ? why : "the widget refused to start");
    return false;
}

void exports_uniwidged_widget_guest_view(exports_uniwidged_widget_guest_tree_t *ret) {
    uw_tree t = {0};
    uw_widget_view(&t);
    ret->nodes.ptr = t.nodes;
    ret->nodes.len = t.len;
}

void exports_uniwidged_widget_guest_next_wake(exports_uniwidged_widget_guest_wake_t *ret) {
    *ret = uw_widget_wake();
}

bool exports_uniwidged_widget_guest_on_event(exports_uniwidged_widget_guest_event_t *e) {
    return uw_widget_on_event(e);
}
