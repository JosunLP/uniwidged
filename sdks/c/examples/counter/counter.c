/*
 * The C SDK's worked example: what `uniwidged new` writes in Rust, in C.
 *
 * It draws, it responds to a click, it reads one of its own settings, and it
 * says something in the log — which is everything a widget does that needs no
 * capability at all.
 */
#include <uniwidged.h>

static int count;

bool uw_widget_start(const char **error) {
    (void)error;
    double start = 0;
    if (uw_setting_number("start", &start)) {
        count = (int)start;
    }
    uw_logf("counting from %d, in C", count);
    return true;
}

void uw_widget_view(uw_tree *t) {
    uw_node root = uw_panel(t, uw_id("counter"));
    uw_gap(t, root, 1);
    uw_role(t, uw_text(t, root, "Clicked"), UW_ROLE_CAPTION);

    uw_node line = uw_row(t, root);
    uw_gap(t, line, 1);
    uw_node value = uw_textf(t, line, "%d", count);
    uw_role(t, value, UW_ROLE_DISPLAY);
    uw_spacer(t, line);
    uw_node plus = uw_chip(t, line, "+1");
    uw_accent(t, plus, UW_ACCENT_BRAND);
    uw_on_click(t, plus, uw_id("plus"));
    uw_label(t, plus, "Count one more");
}

bool uw_widget_on_event(const uw_event *e) {
    if (uw_clicked(e, uw_id("plus"))) {
        count++;
        return true;
    }
    if (uw_configured(e)) {
        double start;
        if (uw_setting_number("start", &start) && (int)start > count) {
            count = (int)start;
            return true;
        }
    }
    return false;
}
