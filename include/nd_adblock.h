// Built-in content blocking (src/adblock.zig), for the Swift host's Chromium
// engine. The Zig engine on Linux calls the same functions directly.
#ifndef ND_ADBLOCK_H
#define ND_ADBLOCK_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
  ND_ADBLOCK_ALLOW = 0,
  ND_ADBLOCK_BLOCK = 1,
  ND_ADBLOCK_REDIRECT = 2,
} nd_adblock_action;

typedef struct {
  int action;             // nd_adblock_action
  uint8_t *body;          // ND_ADBLOCK_REDIRECT: the resource to serve instead
  size_t body_len;
  char *mime;
} nd_adblock_decision;

typedef struct {
  uint8_t *body;          // may be NULL: an empty 200
  size_t body_len;
  const char *mime;       // static, never freed
} nd_adblock_served;

// Whether any lists were ever loaded; until then nothing needs routing here.
bool nd_adblock_active(void);

// One network request. `kind` is the request's cef_resource_type_t. Safe on
// CEF's IO thread. Free with nd_adblock_decision_free.
void nd_adblock_decide(const char *url, const char *initiator, const char *top_url,
                       int kind, nd_adblock_decision *out);
void nd_adblock_decision_free(nd_adblock_decision *d);

// The response to one request on the "nd-adblock" scheme, which the hosts
// serve themselves with Access-Control-Allow-Origin: *. `top_url` is the tab's
// main-frame URL. Safe on CEF's IO thread. Free with nd_adblock_served_free.
void nd_adblock_serve(const char *url, const char *top_url, nd_adblock_served *out);
void nd_adblock_served_free(nd_adblock_served *s);

#ifdef __cplusplus
}
#endif

#endif
