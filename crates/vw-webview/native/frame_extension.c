/* Native frame evaluation for the browser. WebKit's private script world owns state;
 * the UI process alone can request evaluation via UserMessage, or, for an act that must count as the person's
 * gesture, through the main frame's private world (a function only that world can see). No page IPC binding.
 * Frame weak references, 64-frame and 2 MiB bounds limit retained state and answers. */
#include <webkit2/webkit-web-extension.h>

#define AGENT_INPUT "__vwAgentInput"
static char *agent_input(const char *target, const char *script, gpointer data);

static WebKitScriptWorld *agent_world;
typedef struct { GWeakRef frame; guint64 id; } Frame;
typedef struct { GPtrArray *frames; guint64 generation; } Registry;

static void free_registry(gpointer data)
{
    Registry *registry = data;
    g_ptr_array_unref(registry->frames);
    g_free(registry);
}

static void free_frame(gpointer data)
{
    Frame *entry = data;
    g_weak_ref_clear(&entry->frame);
    g_free(entry);
}

static void prune_frames(Registry *registry)
{
    GPtrArray *frames = registry->frames;
    for (guint i = frames->len; i > 0; --i) {
        Frame *entry = g_ptr_array_index(frames, i - 1);
        WebKitFrame *frame = g_weak_ref_get(&entry->frame);
        if (frame) g_object_unref(frame);
        else {
            g_ptr_array_remove_index(frames, i - 1);
            ++registry->generation;
        }
    }
}

/* A frame removed from its page stays alive (and answers) until it is collected: drop it as soon as its window has
 * no parent, so a removal changes the registry once, when it happens, and never later in the middle of an action. */
static void drop_detached(Registry *registry)
{
    GPtrArray *frames = registry->frames;
    for (guint i = frames->len; i > 0; --i) {
        Frame *entry = g_ptr_array_index(frames, i - 1);
        WebKitFrame *frame = g_weak_ref_get(&entry->frame);
        if (!frame) continue;
        gboolean detached = FALSE;
        if (!webkit_frame_is_main_frame(frame)) {
            JSCContext *context = webkit_frame_get_js_context_for_script_world(frame, agent_world);
            JSCValue *value = jsc_context_evaluate(context, "window.parent === null || window.top === null", -1);
            detached = !jsc_context_get_exception(context) && jsc_value_is_boolean(value) && jsc_value_to_boolean(value);
            jsc_context_clear_exception(context);
            g_object_unref(value);
            g_object_unref(context);
        }
        g_object_unref(frame);
        if (detached) {
            g_ptr_array_remove_index(frames, i - 1);
            ++registry->generation;
        }
    }
}

static void window_cleared(WebKitScriptWorld *world, WebKitWebPage *page, WebKitFrame *frame, gpointer unused)
{
    (void)world; (void)unused;
    Registry *registry = g_object_get_data(G_OBJECT(page), "agent-frames");
    if (!registry) return;
    GPtrArray *frames = registry->frames;
    /* Every document replacement counts, including reused handles and frames beyond the retained bound. */
    ++registry->generation;
    if (webkit_frame_is_main_frame(frame)) g_ptr_array_set_size(frames, 0);
    else prune_frames(registry);
    guint64 id = webkit_frame_get_id(frame);
    for (guint i = 0; i < frames->len; ++i)
        if (((Frame *)g_ptr_array_index(frames, i))->id == id) return;
    if (webkit_frame_is_main_frame(frame)) {
        JSCContext *context = webkit_frame_get_js_context_for_script_world(frame, world);
        JSCValue *function = jsc_value_new_function(context, AGENT_INPUT, G_CALLBACK(agent_input), page, NULL,
            G_TYPE_STRING, 2, G_TYPE_STRING, G_TYPE_STRING);
        jsc_context_set_value(context, AGENT_INPUT, function);
        g_object_unref(function);
        g_object_unref(context);
    }
    if (frames->len >= 64) return;
    Frame *entry = g_new0(Frame, 1);
    entry->id = id;
    g_weak_ref_init(&entry->frame, frame);
    g_ptr_array_add(frames, entry);
}

/* Evaluates `script` in every registered frame (empty `target`) or the one with that id. Answers are
 * (id, generation, main, uri, json); NULL with `*error` set when a frame threw or the answers grew too large. */
static GVariant *run_frames(WebKitWebPage *page, const char *target, const char *script, const char **error)
{
    Registry *registry = g_object_get_data(G_OBJECT(page), "agent-frames");
    if (!registry) {
        *error = "frame document changed, detached, or could not be evaluated";
        return NULL;
    }
    prune_frames(registry);
    drop_detached(registry);
    GPtrArray *frames = registry->frames;
    GVariantBuilder replies;
    g_variant_builder_init(&replies, G_VARIANT_TYPE("a(stbss)"));
    *error = NULL;
    gsize bytes = 0;
    for (guint i = 0; i < frames->len; ++i) {
        Frame *entry = g_ptr_array_index(frames, i);
        char id[32];
        g_snprintf(id, sizeof(id), "%" G_GUINT64_FORMAT, entry->id);
        if (*target && strcmp(target, id) != 0) continue;
        WebKitFrame *frame = g_weak_ref_get(&entry->frame);
        if (!frame) continue;
        JSCContext *context = webkit_frame_get_js_context_for_script_world(frame, agent_world);
        JSCValue *value = jsc_context_evaluate(context, script, -1);
        if (jsc_context_get_exception(context)) {
            /* Never return a page exception (it may contain a filled value). */
            *error = "frame document changed, detached, or could not be evaluated";
            jsc_context_clear_exception(context);
        } else {
            char *json = jsc_value_to_json(value, 0);
            const char *uri = webkit_frame_get_uri(frame);
            bytes += json ? strlen(json) : 0;
            if (bytes > 2 * 1024 * 1024) *error = "frame answers exceed the size limit";
            else g_variant_builder_add(&replies, "(stbss)", id, registry->generation, webkit_frame_is_main_frame(frame), uri ? uri : "about:blank", json ? json : "null");
            g_free(json);
        }
        g_object_unref(value);
        g_object_unref(context);
        g_object_unref(frame);
        if (*error) break;
    }
    if (*error) {
        g_variant_builder_clear(&replies);
        return NULL;
    }
    return g_variant_builder_end(&replies);
}

static gboolean evaluate(WebKitWebPage *page, WebKitUserMessage *message, gpointer unused)
{
    (void)unused;
    if (g_strcmp0(webkit_user_message_get_name(message), "agent-evaluate") != 0) return FALSE;
    GVariant *params = webkit_user_message_get_parameters(message);
    if (!params || !g_variant_is_of_type(params, G_VARIANT_TYPE("(ss)"))) return FALSE;
    const char *target, *script;
    g_variant_get(params, "(&s&s)", &target, &script);
    if (strlen(script) > 2 * 1024 * 1024) return FALSE;
    const char *error = NULL;
    GVariant *answers = run_frames(page, target, script, &error);
    if (!answers) {
        webkit_user_message_send_reply(message, webkit_user_message_new("agent-error", g_variant_new_string(error)));
    } else {
        webkit_user_message_send_reply(message, webkit_user_message_new("agent-answers", answers));
    }
    return TRUE;
}

/* The same evaluation, called from the main frame's private world while the UI process runs a script there. WebKit
 * runs that script as a user gesture (webkit_web_view_evaluate_javascript), so the act's synchronous click opens a
 * window the way a person's click does; no gesture exists outside such a call. Answers as a JSON string:
 * [{"id","generation","main","url","value"}], or {"error": text}. Only this world can see the function. */
static char *agent_input(const char *target, const char *script, gpointer data)
{
    WebKitWebPage *page = data;
    JSCContext *context = jsc_context_get_current();
    if (!context || !target || !script || strlen(script) > 2 * 1024 * 1024) return g_strdup("{\"error\":\"invalid frame request\"}");
    const char *error = NULL;
    GVariant *answers = run_frames(page, target, script, &error);
    JSCValue *out;
    if (!answers) {
        out = jsc_value_new_object(context, NULL, NULL);
        JSCValue *text = jsc_value_new_string(context, error);
        jsc_value_object_set_property(out, "error", text);
        g_object_unref(text);
    } else {
        out = jsc_value_new_array(context, G_TYPE_NONE);
        GVariantIter iter;
        const char *id, *uri, *json;
        guint64 generation;
        gboolean main;
        guint index = 0;
        g_variant_ref_sink(answers);
        g_variant_iter_init(&iter, answers);
        while (g_variant_iter_next(&iter, "(&stb&s&s)", &id, &generation, &main, &uri, &json)) {
            JSCValue *item = jsc_value_new_object(context, NULL, NULL);
            JSCValue *values[5] = {
                jsc_value_new_string(context, id), jsc_value_new_number(context, (double)generation),
                jsc_value_new_boolean(context, main), jsc_value_new_string(context, uri), jsc_value_new_string(context, json),
            };
            const char *names[5] = { "id", "generation", "main", "url", "value" };
            for (int i = 0; i < 5; ++i) {
                jsc_value_object_set_property(item, names[i], values[i]);
                g_object_unref(values[i]);
            }
            jsc_value_object_set_property_at_index(out, index++, item);
            g_object_unref(item);
        }
        g_variant_unref(answers);
    }
    char *text = jsc_value_to_json(out, 0);
    g_object_unref(out);
    return text ? text : g_strdup("{\"error\":\"frame answers could not be encoded\"}");
}

static void page_created(WebKitWebExtension *extension, WebKitWebPage *page, gpointer unused)
{
    (void)extension; (void)unused;
    Registry *registry = g_new0(Registry, 1);
    registry->frames = g_ptr_array_new_with_free_func(free_frame);
    g_object_set_data_full(G_OBJECT(page), "agent-frames", registry, free_registry);
    g_signal_connect(page, "user-message-received", G_CALLBACK(evaluate), NULL);
}

G_MODULE_EXPORT void webkit_web_extension_initialize(WebKitWebExtension *extension)
{
    agent_world = webkit_script_world_new_with_name("vibe-work-agent");
    g_signal_connect(agent_world, "window-object-cleared", G_CALLBACK(window_cleared), NULL);
    g_signal_connect(extension, "page-created", G_CALLBACK(page_created), NULL);
    g_object_set_data_full(G_OBJECT(extension), "agent-world", agent_world, g_object_unref);
}
