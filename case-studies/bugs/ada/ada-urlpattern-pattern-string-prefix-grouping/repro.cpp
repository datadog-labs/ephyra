// generate_pattern_string drops the {} around a part whose prefix starts with
// "/" but is longer than "/", so the generated pathname string no longer
// parses back to the same pattern.
//   c++ -std=c++20 -DADA_INCLUDE_URL_PATTERN=1 -DADA_USE_UNSAFE_STD_REGEX_PROVIDER \
//       -I<amalgamation> repro.cpp <amalgamation>/ada.cpp -o repro
#include <cstdio>
#include <string>
#include "ada.h"

using P = ada::url_pattern_regex::std_regex_provider;

static ada::url_pattern<P> make(std::string pathname) {
  ada::url_pattern_init init;
  init.pathname = std::move(pathname);
  return *ada::parse_url_pattern<P>(std::move(init));
}

int main() {
  int bad = 0;
  struct { const char* pattern; const char* expected; } gen[] = {
      {"{/9*}", "{/9*}"},
      {"/a{/9*}?", "/a{/9*}?"},
      {"/a{/ab:id}?", "/a{/ab:id}?"},
  };
  for (auto& g : gen) {
    std::string got(make(g.pattern).get_pathname());
    printf("pathname %-14s -> %-14s (spec: %s)\n", g.pattern, got.c_str(), g.expected);
    bad += got != g.expected;
  }
  // Feed the generated string back in; it must match the same URLs.
  struct { const char* pattern; const char* url; } rt[] = {
      {"/a{/9*}?", "https://h/a"},
      {"/a{/ab:id}?", "https://h/a"},
      {"/a{/ab:id}?", "https://h/a/ab"},
  };
  for (auto& r : rt) {
    auto orig = make(r.pattern);
    auto again = make(std::string(orig.get_pathname()));
    bool a = *orig.test(std::string_view(r.url));
    bool b = *again.test(std::string_view(r.url));
    printf("test(%s): %s -> %d, round-tripped %s -> %d\n", r.url, r.pattern, a,
           std::string(orig.get_pathname()).c_str(), b);
    bad += a != b;
  }
  printf(bad ? "FAIL\n" : "OK\n");
  return bad ? 1 : 0;
}
