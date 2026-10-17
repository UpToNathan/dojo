#define NAPI_VERSION 8
#include <node_api.h>
#include <atk-bridge.h>

static napi_value cleanup(napi_env env, napi_callback_info info) {
  napi_value result;
  atk_bridge_adaptor_cleanup();
  if (napi_get_undefined(env, &result) != napi_ok) return NULL;
  return result;
}

static napi_value initialize(napi_env env, napi_callback_info info) {
  napi_value result;
  if (atk_bridge_adaptor_init(NULL, NULL) != 0) {
    napi_throw_error(env, NULL, "ATK bridge initialization failed");
    return NULL;
  }
  if (napi_get_undefined(env, &result) != napi_ok) return NULL;
  return result;
}

NAPI_MODULE_INIT() {
  const napi_property_descriptor properties[] = {
    {"cleanup", NULL, cleanup, NULL, NULL, NULL, napi_default, NULL},
    {"initialize", NULL, initialize, NULL, NULL, NULL, napi_default, NULL},
  };
  if (napi_define_properties(env, exports, 2, properties) != napi_ok) return NULL;
  return exports;
}
