#include <cstdio>
#include "ada.h"
int main() {
  using P = ada::url_pattern_regex::std_regex_provider;
  for (const char* src : {"*://*", "https://*"}) {
    auto p = ada::parse_url_pattern<P>(std::string_view(src));
    std::string_view in = "not a url";
    auto t = p->test(ada::url_pattern_input(in));
    auto e = p->exec(ada::url_pattern_input(in));
    printf("%-10s test(\"not a url\") = %s, exec = %s, parse = %d\n", src,
           t ? (*t ? "true" : "false") : "error", e ? (*e ? "match" : "null") : "error",
           bool(ada::parse<ada::url_aggregator>(in)));
    if (e && *e) printf("  protocol input '%s', pathname input '%s'\n", (**e).protocol.input.c_str(), (**e).pathname.input.c_str());
  }
}
