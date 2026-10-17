#include <atk/atk.h>
#include <atk-bridge.h>
#include <glib.h>
#include <stdio.h>
#include <string.h>

static AtkObject *root;
static AtkObject *first;
static AtkObject *second;
static GMainLoop *loop;
static gboolean first_defunct;
static AtkStateSet *states(AtkObject *object) {
  AtkStateSet *result = atk_state_set_new();
  if (object == first && first_defunct) atk_state_set_add_state(result, ATK_STATE_DEFUNCT);
  return result;
}
static const gchar *toolkit_name(void) { return "Dojo endpoint fixture"; }
static const gchar *toolkit_version(void) { return "1"; }
static AtkObject *get_root(void) { return root; }

static gboolean refuses(AtkObject *object) {
  gchar *owner = NULL, *path = NULL;
  gboolean result = dojo_atk_bridge_dup_accessible_reference(object, &owner, &path);
  g_free(owner); g_free(path);
  return !result;
}

static gpointer other_thread(gpointer data) {
  gboolean *result = data;
  *result = refuses(first);
  return NULL;
}

static void publish(void) {
  gchar *owner1 = NULL, *path1 = NULL, *owner2 = NULL, *path2 = NULL;
  g_assert(dojo_atk_bridge_dup_accessible_reference(first, &owner1, &path1));
  g_assert(dojo_atk_bridge_dup_accessible_reference(second, &owner2, &path2));
  g_assert_cmpstr(owner1, ==, owner2);
  g_assert_cmpstr(path1, !=, path2);
  gboolean refused = FALSE;
  GThread *worker = g_thread_new("foreign-reader", other_thread, &refused);
  g_thread_join(worker);
  g_assert(refused);
  printf("{\"owner\":\"%s\",\"first\":\"%s\",\"second\":\"%s\",\"foreign_thread_refused\":true}\n", owner1, path1, path2);
  fflush(stdout);
  g_free(owner1); g_free(path1); g_free(owner2); g_free(path2);
}

static gboolean command(GIOChannel *channel, GIOCondition condition, gpointer unused) {
  gchar *line = NULL;
  if ((condition & G_IO_HUP) || g_io_channel_read_line(channel, &line, NULL, NULL, NULL) != G_IO_STATUS_NORMAL) {
    g_main_loop_quit(loop);
    return G_SOURCE_REMOVE;
  }
  if (g_str_has_prefix(line, "defunct")) {
    first_defunct = TRUE;
    g_assert(refuses(first));
    first_defunct = FALSE;
    puts("{\"defunct_refused\":true}");
    fflush(stdout);
  } else if (g_str_has_prefix(line, "cleanup")) {
    atk_bridge_adaptor_cleanup();
    g_assert(refuses(first));
    puts("{\"cleanup_refused\":true}");
    fflush(stdout);
  } else if (g_str_has_prefix(line, "init")) {
    g_assert_cmpint(atk_bridge_adaptor_init(NULL, NULL), ==, 0);
    publish();
  } else {
    g_main_loop_quit(loop);
  }
  g_free(line);
  return G_SOURCE_CONTINUE;
}

int main(void) {
  root = g_object_new(ATK_TYPE_OBJECT, NULL);
  first = g_object_new(ATK_TYPE_OBJECT, NULL);
  second = g_object_new(ATK_TYPE_OBJECT, NULL);
  atk_object_set_role(root, ATK_ROLE_APPLICATION);
  atk_object_set_role(first, ATK_ROLE_PUSH_BUTTON);
  atk_object_set_role(second, ATK_ROLE_PUSH_BUTTON);
  atk_object_set_name(root, "Dojo endpoint fixture");
  atk_object_set_name(first, "Dojo endpoint first");
  atk_object_set_name(second, "Dojo endpoint second");
  atk_object_set_parent(first, root);
  atk_object_set_parent(second, root);
  ATK_OBJECT_GET_CLASS(first)->ref_state_set = states;
  AtkUtilClass *util = g_type_class_ref(ATK_TYPE_UTIL);
  util->get_root = get_root;
  util->get_toolkit_name = toolkit_name;
  util->get_toolkit_version = toolkit_version;
  g_assert(refuses(first));
  g_assert_cmpint(atk_bridge_adaptor_init(NULL, NULL), ==, 0);
  publish();
  loop = g_main_loop_new(NULL, FALSE);
  GIOChannel *input = g_io_channel_unix_new(0);
  g_io_add_watch(input, G_IO_IN | G_IO_HUP, command, NULL);
  g_main_loop_run(loop);
  atk_bridge_adaptor_cleanup();
  g_io_channel_unref(input);
  g_main_loop_unref(loop);
  g_object_unref(second); g_object_unref(first); g_object_unref(root);
  g_type_class_unref(util);
  return 0;
}
