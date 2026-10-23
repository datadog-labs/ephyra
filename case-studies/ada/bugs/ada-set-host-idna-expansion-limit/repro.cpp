// set_hostname with IDNA-expanding characters under a length limit.
#include <cstdio>
#include <string>
#include "ada.h"
template <class T> void check(const char* name) {
  auto u = ada::parse<T>("ws:a");
  ada::set_max_input_length(59);
  bool ok = u->set_hostname("\xe3\x88\x9e\xe3\x8e\xaf\xe3\x8c\x99\xe3\x8d\xbf");  // ㈞㎯㌙㍿
  ada::set_max_input_length(UINT32_MAX);
  std::string href(u->get_href());
  printf("%-10s set_hostname -> %d, href %zu bytes (limit 59)%s\n", name, ok, href.size(), href.size() > 59 ? "  OVER THE LIMIT" : "");
}
int main() { check<ada::url>("url"); check<ada::url_aggregator>("aggregator"); }
