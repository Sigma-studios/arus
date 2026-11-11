// arus for Linux: the hosted arus in a GTK window, rendered by WebKitGTK.
//
// Flutter has no embeddable webview on Linux, so this runner shows WebKitGTK
// itself instead of a Flutter view (the Dart code in lib/ is not used here).
// It needs the system's libwebkit2gtk-4.1 (installed on most desktops).
//
// - Login cookies and site data: ~/.local/share/xyz.onlineapp.arus
// - File uploads: WebKitGTK's own file chooser.
// - Downloads (exports): a save dialog.
// - Links to other sites: the default browser.
// - Another server: `arus --url http://localhost:8080` or ARUS_URL.

#include "my_application.h"

#include <webkit2/webkit2.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

static const char* kDefaultUrl = "https://arus.online-app.xyz";
static const char* kBackground = "#0B0B0C";

struct _MyApplication {
  GtkApplication parent_instance;
  char* url;
  char* host;
  GtkWindow* window;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// The folder next to the executable holding Flutter's assets.
static gchar* asset_path(const char* name) {
  g_autofree gchar* exe = g_file_read_link("/proc/self/exe", nullptr);
  if (exe == nullptr) return nullptr;
  g_autofree gchar* dir = g_path_get_dirname(exe);
  return g_build_filename(dir, "data", "flutter_assets", "assets", name, nullptr);
}

static gboolean is_own_site(MyApplication* self, const char* uri) {
  g_autoptr(GUri) parsed = g_uri_parse(uri, G_URI_FLAGS_NONE, nullptr);
  if (parsed == nullptr) return TRUE;
  const char* scheme = g_uri_get_scheme(parsed);
  if (g_strcmp0(scheme, "http") != 0 && g_strcmp0(scheme, "https") != 0) {
    // about:, data:, blob: stay in the page.
    return g_strcmp0(scheme, "mailto") != 0 && g_strcmp0(scheme, "tel") != 0;
  }
  return g_strcmp0(g_uri_get_host(parsed), self->host) == 0;
}

static gboolean decide_policy_cb(WebKitWebView* view, WebKitPolicyDecision* decision,
                                 WebKitPolicyDecisionType type, MyApplication* self) {
  if (type != WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION &&
      type != WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION) {
    return FALSE;  // responses: WebKit's default (show, or download)
  }
  WebKitNavigationAction* action =
      webkit_navigation_policy_decision_get_navigation_action(WEBKIT_NAVIGATION_POLICY_DECISION(decision));
  const char* uri = webkit_uri_request_get_uri(webkit_navigation_action_get_request(action));
  if (!is_own_site(self, uri)) {
    gtk_show_uri_on_window(self->window, uri, GDK_CURRENT_TIME, nullptr);
    webkit_policy_decision_ignore(decision);
    return TRUE;
  }
  if (type == WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION) {
    // One window: open arus links here.
    webkit_web_view_load_uri(view, uri);
    webkit_policy_decision_ignore(decision);
    return TRUE;
  }
  return FALSE;
}

static gboolean load_failed_cb(WebKitWebView* view, WebKitLoadEvent event, const char* uri, GError* error,
                               MyApplication* self) {
  if (g_error_matches(error, WEBKIT_NETWORK_ERROR, WEBKIT_NETWORK_ERROR_CANCELLED) ||
      g_error_matches(error, WEBKIT_POLICY_ERROR, WEBKIT_POLICY_ERROR_FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE)) {
    return FALSE;  // a download, or a link opened elsewhere
  }
  g_autofree gchar* msg = g_markup_escape_text(error->message, -1);
  g_autofree gchar* target = g_markup_escape_text(self->url, -1);
  g_autofree gchar* html = g_strdup_printf(
      "<html><body style=\"background:%s;color:#A0A0A8;font-family:sans-serif;display:flex;"
      "height:100vh;margin:0;align-items:center;justify-content:center;text-align:center\">"
      "<div><h2 style=\"color:#fff;font-weight:600\">Could not reach arus</h2>"
      "<p>Check your internet connection.<br>%s &middot; %s</p>"
      "<p><a href=\"%s\" style=\"color:#fff;background:#8B5CF6;padding:8px 18px;border-radius:8px;"
      "text-decoration:none\">Try again</a></p></div></body></html>",
      kBackground, target, msg, target);
  webkit_web_view_load_alternate_html(view, html, uri, nullptr);
  return TRUE;
}

static gboolean decide_destination_cb(WebKitDownload* download, const gchar* suggested, MyApplication* self) {
  GtkFileChooserNative* dialog = gtk_file_chooser_native_new("Save", self->window, GTK_FILE_CHOOSER_ACTION_SAVE,
                                                             "_Save", "_Cancel");
  GtkFileChooser* chooser = GTK_FILE_CHOOSER(dialog);
  gtk_file_chooser_set_do_overwrite_confirmation(chooser, TRUE);
  gtk_file_chooser_set_current_name(chooser, suggested);
  const char* downloads = g_get_user_special_dir(G_USER_DIRECTORY_DOWNLOAD);
  if (downloads != nullptr) gtk_file_chooser_set_current_folder(chooser, downloads);
  if (gtk_native_dialog_run(GTK_NATIVE_DIALOG(dialog)) == GTK_RESPONSE_ACCEPT) {
    g_autofree gchar* dest = gtk_file_chooser_get_uri(chooser);
    webkit_download_set_allow_overwrite(download, TRUE);
    webkit_download_set_destination(download, dest);
  } else {
    webkit_download_cancel(download);
  }
  g_object_unref(dialog);
  return TRUE;
}

static void download_started_cb(WebKitWebContext* context, WebKitDownload* download, MyApplication* self) {
  g_signal_connect(download, "decide-destination", G_CALLBACK(decide_destination_cb), self);
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  GtkWindow* window = GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));
  self->window = window;

  // A header bar on GNOME, a plain title bar on other X11 window managers.
  gboolean use_header_bar = TRUE;
#ifdef GDK_WINDOWING_X11
  GdkScreen* screen = gtk_window_get_screen(window);
  if (GDK_IS_X11_SCREEN(screen)) {
    const gchar* wm_name = gdk_x11_screen_get_window_manager_name(screen);
    if (g_strcmp0(wm_name, "GNOME Shell") != 0) use_header_bar = FALSE;
  }
#endif
  if (use_header_bar) {
    GtkHeaderBar* header_bar = GTK_HEADER_BAR(gtk_header_bar_new());
    gtk_widget_show(GTK_WIDGET(header_bar));
    gtk_header_bar_set_title(header_bar, "arus");
    gtk_header_bar_set_show_close_button(header_bar, TRUE);
    gtk_window_set_titlebar(window, GTK_WIDGET(header_bar));
  } else {
    gtk_window_set_title(window, "arus");
  }
  gtk_window_set_default_size(window, 1280, 800);
  g_autofree gchar* icon = asset_path("icon.png");
  if (icon != nullptr) gtk_window_set_icon_from_file(window, icon, nullptr);

  // Site data and the login cookie persist between runs.
  g_autofree gchar* data_dir = g_build_filename(g_get_user_data_dir(), APPLICATION_ID, nullptr);
  g_autofree gchar* cache_dir = g_build_filename(g_get_user_cache_dir(), APPLICATION_ID, nullptr);
  g_autoptr(WebKitWebsiteDataManager) data = webkit_website_data_manager_new(
      "base-data-directory", data_dir, "base-cache-directory", cache_dir, nullptr);
  g_autoptr(WebKitWebContext) context = webkit_web_context_new_with_website_data_manager(data);
  g_autofree gchar* cookies = g_build_filename(data_dir, "cookies.sqlite", nullptr);
  webkit_cookie_manager_set_persistent_storage(webkit_website_data_manager_get_cookie_manager(data), cookies,
                                               WEBKIT_COOKIE_PERSISTENT_STORAGE_SQLITE);
  g_signal_connect(context, "download-started", G_CALLBACK(download_started_cb), self);

  WebKitWebView* view = WEBKIT_WEB_VIEW(webkit_web_view_new_with_context(context));
  WebKitSettings* settings = webkit_web_view_get_settings(view);
  // "arus-app" in the user agent: the site hides its Download page here.
  webkit_settings_set_user_agent_with_application_details(settings, "arus-app", "1");
  webkit_settings_set_media_playback_requires_user_gesture(settings, FALSE);
  webkit_settings_set_javascript_can_access_clipboard(settings, TRUE);
  webkit_settings_set_enable_developer_extras(settings, FALSE);
  GdkRGBA bg;
  gdk_rgba_parse(&bg, kBackground);
  webkit_web_view_set_background_color(view, &bg);

  g_signal_connect(view, "decide-policy", G_CALLBACK(decide_policy_cb), self);
  g_signal_connect(view, "load-failed", G_CALLBACK(load_failed_cb), self);

  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));
  webkit_web_view_load_uri(view, self->url);
  gtk_widget_show_all(GTK_WIDGET(window));
  gtk_widget_grab_focus(GTK_WIDGET(view));
}

// Implements GApplication::local_command_line: `--url <url>`.
static gboolean my_application_local_command_line(GApplication* application, gchar*** arguments,
                                                  int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  const char* url = g_getenv("ARUS_URL");
  gchar** args = *arguments;
  for (int i = 1; args[i] != nullptr; i++) {
    if (g_strcmp0(args[i], "--url") == 0 && args[i + 1] != nullptr) url = args[i + 1];
  }
  if (url == nullptr || *url == '\0') url = kDefaultUrl;
  self->url = g_strdup(url);
  g_autoptr(GUri) parsed = g_uri_parse(url, G_URI_FLAGS_NONE, nullptr);
  self->host = g_strdup(parsed != nullptr ? g_uri_get_host(parsed) : "");

  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
    g_warning("Failed to register: %s", error->message);
    *exit_status = 1;
    return TRUE;
  }
  g_application_activate(application);
  *exit_status = 0;
  return TRUE;
}

static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  g_clear_pointer(&self->url, g_free);
  g_clear_pointer(&self->host, g_free);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->local_command_line = my_application_local_command_line;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {}

MyApplication* my_application_new() {
  // The application ID names the window for the desktop (and its .desktop file).
  g_set_prgname(APPLICATION_ID);
  return MY_APPLICATION(g_object_new(my_application_get_type(), "application-id", APPLICATION_ID, "flags",
                                     G_APPLICATION_NON_UNIQUE, nullptr));
}
