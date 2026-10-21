#include <cstdio>
#include <string>
#include "ada.h"
template <class T> void check(const char* name, const char* input, const char* host, uint32_t limit) {
  auto u = ada::parse<T>(input);
  ada::set_max_input_length(limit);
  bool ok = u->set_host(host);
  ada::set_max_input_length(UINT32_MAX);
  printf("%-10s limit %2u: %s .host = \"%s\" -> %d, href %s\n", name, limit, input, host, ok,
         std::string(u->get_href()).c_str());
}
int main() {
  check<ada::url>("url", "https://bank.example:1/", "evil.example:9443", 25);
  check<ada::url_aggregator>("aggregator", "https://bank.example:1/", "evil.example:9443", 25);
  check<ada::url>("url", "http://example.com/", "example.org:8080", 20);
  check<ada::url_aggregator>("aggregator", "http://example.com/", "example.org:8080", 20);
}
