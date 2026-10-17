// Test-only GTK fixture. Resolve the pinned runtime passed by the acceptance runner.
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>

static void *runtime;
static void (*label_set_text)(void *, const char *);
static void (*widget_set_sensitive)(void *, int);
static const char *(*editable_get_text)(void *);
static double (*spin_get_value)(void *);
static int (*row_get_index)(void *);
struct Change { const char *name; void *label; };
static struct Change text_changes[2], value_changes[2], selection_changes[2];

static void *symbol(const char *name) {
    void *value = dlsym(runtime, name);
    if (!value) { fprintf(stderr, "missing GTK symbol %s\n", name); exit(1); }
    return value;
}

static void clicked(void *button, void *label) {
    label_set_text(label, "Saved");
    widget_set_sensitive(button, 0);
}
static void text_changed(void *entry, void *data) {
    struct Change *change = data;
    char value[256];
    snprintf(value, sizeof(value), "%s text: %s", change->name, editable_get_text(entry));
    label_set_text(change->label, value);
}
static void value_changed(void *spin, void *data) {
    struct Change *change = data;
    char value[128];
    snprintf(value, sizeof(value), "%s value: %g", change->name, spin_get_value(spin));
    label_set_text(change->label, value);
}
static void row_selected(void *list, void *row, void *data) {
    (void)list;
    struct Change *change = data;
    char value[128];
    snprintf(value, sizeof(value), "%s selection: %d", change->name, row ? row_get_index(row) + 1 : 0);
    label_set_text(change->label, value);
}

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    runtime = dlopen(argv[1], RTLD_NOW | RTLD_GLOBAL);
    if (!runtime) { fprintf(stderr, "%s\n", dlerror()); return 1; }
    void (*init)(void) = symbol("gtk_init");
    void *(*application_new)(const char *, int) = symbol("gtk_application_new");
    int (*application_register)(void *, void *, void **) = symbol("g_application_register");
    void *(*window_new)(void *) = symbol("gtk_application_window_new");
    void (*window_set_title)(void *, const char *) = symbol("gtk_window_set_title");
    void (*window_set_default_size)(void *, int, int) = symbol("gtk_window_set_default_size");
    void (*window_set_child)(void *, void *) = symbol("gtk_window_set_child");
    void (*window_present)(void *) = symbol("gtk_window_present");
    void *(*box_new)(int, int) = symbol("gtk_box_new");
    void (*box_append)(void *, void *) = symbol("gtk_box_append");
    void *(*button_new)(const char *) = symbol("gtk_button_new_with_label");
    void *(*label_new)(const char *) = symbol("gtk_label_new");
    void *(*password_new)(void) = symbol("gtk_password_entry_new");
    void *(*entry_new)(void) = symbol("gtk_entry_new");
    void *(*spin_new)(double, double, double) = symbol("gtk_spin_button_new_with_range");
    void (*spin_set_value)(void *, double) = symbol("gtk_spin_button_set_value");
    void (*accessible_update_property)(void *, int, ...) = symbol("gtk_accessible_update_property");
    void *(*list_new)(void) = symbol("gtk_list_box_new");
    void *(*row_new)(void) = symbol("gtk_list_box_row_new");
    void (*row_set_child)(void *, void *) = symbol("gtk_list_box_row_set_child");
    void (*list_append)(void *, void *) = symbol("gtk_list_box_append");
    void (*list_select)(void *, void *) = symbol("gtk_list_box_select_row");
    void (*list_mode)(void *, int) = symbol("gtk_list_box_set_selection_mode");
    void (*editable_set_text)(void *, const char *) = symbol("gtk_editable_set_text");
    unsigned long (*signal_connect)(void *, const char *, void (*)(void *, void *), void *, void *, int) = symbol("g_signal_connect_data");
    unsigned long (*row_signal_connect)(void *, const char *, void (*)(void *, void *, void *), void *, void *, int) = symbol("g_signal_connect_data");
    int (*iterate)(void *, int) = symbol("g_main_context_iteration");
    label_set_text = symbol("gtk_label_set_text");
    widget_set_sensitive = symbol("gtk_widget_set_sensitive");
    editable_get_text = symbol("gtk_editable_get_text");
    spin_get_value = symbol("gtk_spin_button_get_value");
    row_get_index = symbol("gtk_list_box_row_get_index");
    init();
    void *application = application_new("org.dojo.AccessibilityFixture", 1 << 5);
    if (!application_register(application, NULL, NULL)) return 1;
    for (int index = 0; index < 2; index++) {
        void *window = window_new(application);
        window_set_title(window, "Dojo accessibility fixture");
        window_set_default_size(window, 320, 240);
        void *box = box_new(1, 12);
        void *button = button_new(index == 0 ? "Alpha Save" : "Beta Save");
        void *label = label_new(index == 0 ? "Alpha pending" : "Beta pending");
        void *password = password_new();
        void *entry = entry_new();
        void *spin = spin_new(0, 100, 1);
        void *text_label = label_new(index == 0 ? "Alpha text: initial" : "Beta text: initial");
        void *value_label = label_new(index == 0 ? "Alpha value: 1" : "Beta value: 1");
        void *list = list_new();
        void *first_row = row_new(), *second_row = row_new();
        void *selection_label = label_new(index == 0 ? "Alpha selection: 1" : "Beta selection: 1");
        // GtkAccessibleProperty LABEL is 4 in the pinned GTK4 public ABI.
        accessible_update_property(entry, 4, index == 0 ? "Alpha Input" : "Beta Input", -1);
        accessible_update_property(spin, 4, index == 0 ? "Alpha Value" : "Beta Value", -1);
        accessible_update_property(first_row, 4, index == 0 ? "Alpha Choice One" : "Beta Choice One", -1);
        accessible_update_property(second_row, 4, index == 0 ? "Alpha Choice Two" : "Beta Choice Two", -1);
        row_set_child(first_row, label_new(index == 0 ? "Alpha Choice One" : "Beta Choice One"));
        row_set_child(second_row, label_new(index == 0 ? "Alpha Choice Two" : "Beta Choice Two"));
        list_append(list, first_row);
        list_append(list, second_row);
        list_mode(list, 1);
        list_select(list, first_row);
        editable_set_text(entry, "initial");
        spin_set_value(spin, 1);
        text_changes[index] = (struct Change){index == 0 ? "Alpha" : "Beta", text_label};
        value_changes[index] = (struct Change){index == 0 ? "Alpha" : "Beta", value_label};
        selection_changes[index] = (struct Change){index == 0 ? "Alpha" : "Beta", selection_label};
        signal_connect(entry, "changed", text_changed, &text_changes[index], NULL, 0);
        signal_connect(spin, "value-changed", value_changed, &value_changes[index], NULL, 0);
        row_signal_connect(list, "row-selected", row_selected, &selection_changes[index], NULL, 0);
        editable_set_text(password, "fixture-sensitive-password");
        signal_connect(button, "clicked", clicked, label, NULL, 0);
        box_append(box, button);
        box_append(box, label);
        box_append(box, entry);
        box_append(box, text_label);
        box_append(box, spin);
        box_append(box, value_label);
        box_append(box, list);
        box_append(box, selection_label);
        if (index == 0) box_append(box, password);
        window_set_child(window, box);
        window_present(window);
    }
    for (;;) iterate(NULL, 1);
}
