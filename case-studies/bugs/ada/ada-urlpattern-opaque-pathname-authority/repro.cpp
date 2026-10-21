// A URLPattern built from a non-special URL whose path starts with "//"
// loses that part of its path: it does not match the URL it was made from,
// and matches a different one.
#include <cstdio>
#include <string>
#include "ada.h"
int main() {
  using P = ada::url_pattern_regex::std_regex_provider;
  auto p = ada::parse_url_pattern<P>(std::string_view("foo://h//x/y"));
  printf("pathname component: %s\n", std::string(p->get_pathname()).c_str());
  printf("test(foo://h//x/y) = %d\n", int(*p->test(std::string_view("foo://h//x/y"))));
  printf("test(foo://h/y)    = %d\n", int(*p->test(std::string_view("foo://h/y"))));
}
